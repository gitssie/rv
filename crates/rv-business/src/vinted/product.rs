//! Single product reads adapt the details response to the existing product upsert.
use super::*;

pub(super) struct ProductDetails { id: u64 }
impl ProductDetails {
    pub fn new(id: &str) -> Result<Self, String> {
        let id = id.parse::<u64>().ok().filter(|id| *id > 0).ok_or("商品 ID 无效")?;
        Ok(Self { id })
    }
}
impl BusinessStrategy for ProductDetails {
    fn requires_proxy(&self) -> bool { true }
    fn request(&self, _: Operation, _: &AppAuth) -> Result<RequestPlan, String> {
        Ok(RequestPlan {
            method: reqwest::Method::GET,
            url: format!("https://www.vinted.com/api/v2/items/{}/details", self.id),
            body: None,
        })
    }
    fn collect_page(&self, _: Operation, response: Value, combined: &mut Value, _: u32, _: Option<&str>) -> Result<bool, String> {
        if response.get("code").is_some_and(|code| code != 0) {
            return Err("Vinted 商品详情回复未成功".into());
        }
        let item = &response["item"];
        if item["id"].as_u64() != Some(self.id) {
            return Err("商品详情未返回所请求的商品".into());
        }
        let mut item = item.clone();
        // Details can carry the owner as an embedded user instead of user_id.
        if item["user_id"].is_null() {
            item["user_id"] = item["user"]["id"].clone();
        }
        // A successful details response is authoritative evidence of existence.
        item["_rv_deleted"]=json!(false);
        *combined = json!({"code":0,"items":[item]});
        Ok(false)
    }
    fn project(&self, op: Operation, status: u16, response: &Value) -> Value {
        let result=Vinted.project(op,status,response);
        if op==Operation::Products && status==404 && result["error_code"]=="104" {
            json!({"http_status":status,"product_deleted":true,"product_id":self.id.to_string()})
        } else {result}
    }
    fn is_success(&self,op:Operation,status:u16,result:&Value)->bool {
        Vinted.is_success(op,status,result) || (op==Operation::Products && status==404
            && result["product_deleted"]==true && result["product_id"]==self.id.to_string())
    }
    fn image_urls(&self, op: Operation, result: &Value) -> Vec<String> {
        Vinted.image_urls(op, result)
    }
    fn task_result(&self, op: Operation, result: &Value) -> Value {
        Vinted.task_result(op, result)
    }
    fn persist(&self, db: &rusqlite::Connection, id: &AppId, op: Operation, result: &Value) -> Result<(), rv_core::StoreError> {
        if op==Operation::Products && result["http_status"]==404 && result["product_deleted"]==true
            && result["product_id"]==self.id.to_string() {
            super::storage::mark_product_deleted(db,id,&self.id.to_string())
        } else {Vinted.persist(db, id, op, result)}
    }
}
