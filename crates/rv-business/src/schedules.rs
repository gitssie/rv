//! Only schedule settings persist. Each RV run starts a fresh interval clock.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use rv_core::StoreError;
use std::collections::{HashMap, HashSet};
fn schema(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS business_schedules(app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,operation TEXT NOT NULL,enabled INTEGER NOT NULL,interval_seconds INTEGER NOT NULL,PRIMARY KEY(app_id,operation));")?;
    Ok(())
}
fn operation(key: &str) -> Option<Operation> {
    match key {
        "products" => Some(Operation::Products),
        "orders" => Some(Operation::Orders),
        _ => None,
    }
}
#[derive(Clone, Debug)]
pub struct ScheduledJob {
    pub app_id: AppId,
    pub operation: Operation,
}
pub(crate) fn authorize(db: &Connection, job: &ScheduledJob) -> Result<(), StoreError> {
    let allowed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM business_schedules s JOIN app_instances a ON a.id=s.app_id WHERE s.app_id=?1 AND s.operation=?2 AND s.enabled=1 AND a.archived=0)",params![job.app_id.as_str(),job.operation.key()],|r|r.get(0))?;
    if !allowed {
        return Err(StoreError::Invalid("定时同步已关闭或店铺已归档".into()));
    }
    Ok(())
}
pub fn enabled(paths: &StorePaths, id: &AppId, op: Operation) -> Result<bool, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .business_transaction(|db| {
            schema(db)?;
            Ok(db
                .query_row(
                    "SELECT enabled FROM business_schedules WHERE app_id=?1 AND operation=?2",
                    params![id.as_str(), op.key()],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .map_err(|e| e.to_string())
}
pub fn set(
    paths: &StorePaths,
    id: &AppId,
    op: Operation,
    enabled: bool,
    interval: i64,
) -> Result<(), String> {
    if operation(op.key()).is_none() || !(300..=86400).contains(&interval) {
        return Err("仅支持商品／订单同步，周期需为 5 分钟至 24 小时".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    if strategy(&app.bundle_id).is_none() {
        return Err("此 App 尚未接入定时同步".into());
    }
    if enabled && store.auth(id).map_err(|e| e.to_string())?.is_none() {
        return Err("请先读取账号资料".into());
    }
    store.business_transaction(|db|{schema(db)?;db.execute("INSERT INTO business_schedules VALUES(?1,?2,?3,?4) ON CONFLICT(app_id,operation) DO UPDATE SET enabled=excluded.enabled,interval_seconds=excluded.interval_seconds",params![id.as_str(),op.key(),enabled,interval])?;Ok(())}).map_err(|e|e.to_string())
}
#[derive(Default)]
pub struct ScheduleClock {
    next: HashMap<(AppId, Operation), i64>,
}
impl ScheduleClock {
    pub fn due(&mut self, paths: &StorePaths, timestamp: i64) -> Result<Vec<ScheduledJob>, String> {
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let settings=store.business_transaction(|db|{
            schema(db)?;
            let mut stmt=db.prepare("SELECT s.app_id,s.operation,s.interval_seconds FROM business_schedules s JOIN app_instances a ON a.id=s.app_id JOIN app_auth c ON c.app_id=s.app_id WHERE s.enabled=1 AND a.archived=0 ORDER BY s.app_id,s.operation")?;
            let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?)))?.collect::<Result<Vec<_>,_>>()?;Ok(rows)
        }).map_err(|e|e.to_string())?;
        let mut active = HashSet::new();
        let mut shops = HashSet::new();
        let mut jobs = Vec::new();
        for (id, key, interval) in settings {
            let Some(op) = operation(&key) else {
                continue;
            };
            let id = AppId::from_idfv(&id).map_err(|e| e.to_string())?;
            let key = (id.clone(), op);
            active.insert(key.clone());
            let next = self.next.entry(key).or_insert(timestamp + interval);
            if timestamp >= *next && shops.insert(id.clone()) {
                *next = timestamp + interval;
                jobs.push(ScheduledJob {
                    app_id: id,
                    operation: op,
                });
            }
        }
        self.next.retain(|key, _| active.contains(key));
        Ok(jobs)
    }
}
