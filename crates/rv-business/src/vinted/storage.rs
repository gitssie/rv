//! Vinted owns its tables, migration, projection and offline search.
use super::*;
use crate::data::*;
use rusqlite::{Connection, params};
use rv_core::StoreError;
use std::time::{SystemTime, UNIX_EPOCH};
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn schema(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch(include_str!("schema.sql"))?;
    let version: i64 = db.query_row(
        "SELECT version FROM vinted_schema WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    if version != 1 {
        return Err(StoreError::Invalid("Vinted 数据版本不受支持".into()));
    }
    Ok(())
}
fn scoped(db: &Connection, id: &AppId) -> Result<(), StoreError> {
    let bundle: String = db.query_row(
        "SELECT bundle_id FROM app_instances WHERE id=?1",
        [id.as_str()],
        |r| r.get(0),
    )?;
    if bundle != "lt.manodrabuziai.fr" {
        return Err(StoreError::Invalid("业务数据与 App 不匹配".into()));
    }
    Ok(())
}
fn external_id(value: &Value) -> Result<String, StoreError> {
    let id = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) if n.is_u64() => n.to_string(),
        _ => String::new(),
    };
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err(StoreError::Invalid("业务记录缺少有效 ID".into()));
    }
    Ok(id)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key].as_str()
}
pub(super) fn mark_product_deleted(db:&Connection,id:&AppId,product:&str)->Result<(),StoreError> {
    schema(db)?;scoped(db,id)?;
    let changed=db.execute("UPDATE vinted_products SET payload=json_set(payload,'$._rv_deleted',json('true'),'$._rv_deleted_at',?3),fetched_at=?3 WHERE app_id=?1 AND product_id=?2",params![id.as_str(),product,now()])?;
    if changed!=1 {return Err(StoreError::Invalid("本地商品记录不存在，未标记删除".into()));}
    Ok(())
}
pub fn persist(
    db: &Connection,
    id: &AppId,
    operation: Operation,
    result: &Value,
) -> Result<(), StoreError> {
    if !matches!(operation, Operation::Products | Operation::Orders) || result["http_status"] != 200
    {
        return Ok(());
    }
    schema(db)?;
    scoped(db, id)?;
    let response = &result["response"];
    if response.get("code").is_some_and(|code| code != 0) {
        return Err(StoreError::Invalid("Vinted 业务回复未成功".into()));
    }
    let timestamp = now();
    if operation == Operation::Products {
        let metadata: String = db.query_row(
            "SELECT metadata FROM app_instances WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        let metadata: Value = serde_json::from_str(&metadata)?;
        let owner = metadata["user_id"]
            .as_u64()
            .filter(|id| *id > 0)
            .ok_or_else(|| StoreError::Invalid("缺少已验证的店铺用户 ID".into()))?;
        let rows = response["items"]
            .as_array()
            .ok_or_else(|| StoreError::Invalid("商品列表结构无效".into()))?;
        for row in rows {
            if row["user_id"].as_u64() != Some(owner) {
                return Err(StoreError::Invalid("商品归属与店铺不匹配".into()));
            }
            let record_id = external_id(&row["id"])?;
            // A stale list must not erase a deletion confirmed by item details.
            // Only a successful details read explicitly clears the local marker.
            db.execute("INSERT INTO vinted_products VALUES(?1,?2,?3,?4) ON CONFLICT(app_id,product_id) DO UPDATE SET payload=CASE WHEN json_extract(vinted_products.payload,'$._rv_deleted')=1 AND json_extract(excluded.payload,'$._rv_deleted') IS NOT 0 THEN vinted_products.payload ELSE excluded.payload END,fetched_at=excluded.fetched_at",params![id.as_str(),record_id,row.to_string(),timestamp])?;
        }
    } else {
        let rows = response["my_orders"]
            .as_array()
            .ok_or_else(|| StoreError::Invalid("订单列表结构无效".into()))?;
        for row in rows {
            // Keep escrow, transaction and conversation identities separate.
            let record_id = external_id(&row["escrow_order_id"])?;
            let direction = text(row, "_rv_direction")
                .filter(|v| ["sold", "purchased"].contains(v))
                .ok_or_else(|| StoreError::Invalid("订单方向缺失".into()))?;
            db.execute("INSERT INTO vinted_orders VALUES(?1,?2,?3,?4,?5) ON CONFLICT(app_id,order_id,direction) DO UPDATE SET payload=excluded.payload,fetched_at=excluded.fetched_at",params![id.as_str(),record_id,direction,row.to_string(),timestamp])?;
        }
    }
    // No destructive replacement: absence from a list never implies deletion.
    Ok(())
}
pub fn read(
    db: &Connection,
    id: &AppId,
    dataset: Dataset,
    search_text: &str,
    offset: usize,
) -> Result<BusinessPage, StoreError> {
    schema(db)?;
    scoped(db, id)?;
    let (table, search) = match dataset {
        Dataset::Products => ("vinted_products", "vinted_product_search"),
        Dataset::Orders => ("vinted_orders", "vinted_order_search"),
    };
    let query = search_text.trim();
    let predicate = if query.is_empty() {
        "1".to_string()
    } else if query.chars().count() >= 3 {
        format!("rowid IN (SELECT rowid FROM {search} WHERE {search} MATCH ?2)")
    } else {
        format!(
            "rowid IN (SELECT rowid FROM {search} WHERE title LIKE ?2 ESCAPE '\\' OR payload LIKE ?2 ESCAPE '\\')"
        )
    };
    let pattern = if query.chars().count() >= 3 {
        format!("\"{}\"", query.replace('"', "\"\""))
    } else {
        format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    };
    // Numbered ?2 is kept bound even for an empty query.
    let predicate = format!("({predicate}) AND (?2 IS NOT NULL)");
    let total: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE app_id=?1 AND {predicate}"),
        params![id.as_str(), pattern],
        |r| r.get(0),
    )?;
    let mut stmt=db.prepare(&format!("SELECT payload,fetched_at FROM {table} WHERE app_id=?1 AND {predicate} ORDER BY fetched_at DESC,CAST(json_extract(payload, '$.id') AS INTEGER) DESC,rowid DESC LIMIT 30 OFFSET ?3"))?;
    let mut rows = stmt.query(params![id.as_str(), pattern, offset as i64])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let payload: Value = serde_json::from_str(&row.get::<_, String>(0)?)?;
        let record_id = external_id(
            &payload[if dataset == Dataset::Products {
                "id"
            } else {
                "escrow_order_id"
            }],
        )?;
        let amount = text(&payload["price"], "amount").unwrap_or("—");
        let currency = text(&payload["price"], "currency_code").unwrap_or("");
        let state = if dataset == Dataset::Products {
            if payload["_rv_deleted"] == true {
                "已删除"
            } else if payload["is_hidden"] == true {
                "隐藏"
            } else if payload["is_draft"] == true {
                "草稿"
            } else if payload["is_closed"] == true {
                "已关闭"
            } else if payload["is_reserved"] == true {
                "预留"
            } else if payload["is_processing"] == true {
                "处理中"
            } else if payload["transaction_permitted"] == true {
                "可交易"
            } else if payload["transaction_permitted"] == false {
                "不可交易"
            } else if payload["is_hidden"] == false {
                "激活"
            } else {
                "状态待确认"
            }
        } else {
            text(&payload, "status").unwrap_or("状态未知")
        };
        let direction = if dataset == Dataset::Orders {
            match text(&payload, "_rv_direction") {
                Some("sold") => " · 卖出",
                Some("purchased") => " · 买入",
                _ => "",
            }
        } else {
            ""
        };
        output.push(BusinessRow {
            id: record_id,
            title: text(&payload, "title").unwrap_or("未提供标题").into(),
            summary: format!("{amount} {currency} · {state}{direction}"),
            visual: crate::presentation::vinted(&payload, dataset),
            payload,
            fetched_at: row.get(1)?,
        });
    }
    Ok(BusinessPage {
        rows: output,
        total: total as usize,
    })
}
