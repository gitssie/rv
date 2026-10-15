use super::*;
mod storage;
pub struct Vinted;
impl BusinessStrategy for Vinted {
    fn request(&self, operation: Operation, auth: &AppAuth) -> Result<RequestPlan, String> {
        let url = match operation {
            Operation::Session => "https://www.vinted.com/api/v2/users/current",
            Operation::Inbox => "https://api.vinted.com/messaging/main/inbox",
            Operation::NotificationStats => "https://www.vinted.com/api/v2/conversations/stats",
            Operation::Refresh => "https://api.vinted.com/oauth/api/token.issue",
            Operation::Products | Operation::Orders => {
                return Err("列表请求需要店铺及分页参数".into());
            }
        };
        let body = if operation == Operation::Refresh {
            let token = auth
                .refresh_token
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or("备份没有 refresh token")?;
            let mut encoded = String::new();
            for byte in token.bytes() {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    encoded.push(byte as char)
                } else {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
            Some(format!(
                "grant_type=refresh_token&client_id=ios&scope=user&refresh_token={encoded}"
            ))
        } else {
            None
        };
        Ok(RequestPlan {
            method: if body.is_some() {
                reqwest::Method::POST
            } else {
                reqwest::Method::GET
            },
            url: url.into(),
            body,
        })
    }
    fn directions(&self, operation: Operation) -> Vec<Option<&'static str>> {
        if operation == Operation::Orders {
            vec![Some("sold"), Some("purchased")]
        } else {
            vec![None]
        }
    }
    fn request_page(
        &self,
        operation: Operation,
        auth: &AppAuth,
        app: &AppInstance,
        page: u32,
        direction: Option<&str>,
    ) -> Result<RequestPlan, String> {
        let url = match operation {
            Operation::Products => {
                let metadata: Value =
                    serde_json::from_str(&app.metadata).map_err(|_| "店铺信息格式错误")?;
                let user = metadata["user_id"]
                    .as_u64()
                    .filter(|id| *id > 0)
                    .ok_or("请先检测会话，取得已验证的店铺用户 ID")?;
                format!(
                    "https://www.vinted.com/api/v2/wardrobe/{user}/items?page={page}&per_page=20"
                )
            }
            Operation::Orders => {
                let direction = direction
                    .filter(|d| ["sold", "purchased"].contains(d))
                    .ok_or("订单方向无效")?;
                format!(
                    "https://api.vinted.com/escrow-orders/api/v2/current-user/escrow-orders?page={page}&per_page=20&type={direction}&status=all"
                )
            }
            _ => return self.request(operation, auth),
        };
        Ok(RequestPlan {
            method: reqwest::Method::GET,
            url,
            body: None,
        })
    }
    fn collect_page(
        &self,
        operation: Operation,
        response: Value,
        combined: &mut Value,
        page: u32,
        direction: Option<&str>,
    ) -> Result<bool, String> {
        if !matches!(operation, Operation::Products | Operation::Orders) {
            *combined = response;
            return Ok(false);
        }
        if response.get("code").is_some_and(|code| code != 0) {
            return Err("Vinted 业务回复未成功".into());
        }
        let key = if operation == Operation::Products {
            "items"
        } else {
            "my_orders"
        };
        let rows = response[key].as_array().ok_or("业务列表结构无效")?;
        let pagination = &response["pagination"];
        let current = pagination["current_page"].as_u64().ok_or("分页信息缺失")?;
        let total = pagination["total_pages"].as_u64().ok_or("分页信息缺失")?;
        if current != page as u64 || total > 200 || (total > 0 && current > total) {
            return Err("分页信息不一致".into());
        }
        if combined.is_null() {
            *combined = json!({(key):[],"code":0});
        }
        let output = combined[key].as_array_mut().ok_or("分页合并失败")?;
        for row in rows {
            if !row.is_object() {
                return Err("业务记录结构无效".into());
            }
            let mut row = redact(row, 0);
            if let Some(direction) = direction {
                row["_rv_direction"] = json!(direction);
            }
            output.push(row);
        }
        Ok(current < total)
    }
    fn task_result(&self, operation: Operation, result: &Value) -> Value {
        if matches!(operation, Operation::Products | Operation::Orders) {
            json!({"http_status":result["http_status"],"record_count":result["response"][if operation==Operation::Products{"items"}else{"my_orders"}].as_array().map_or(0,Vec::len)})
        } else {
            result.clone()
        }
    }
    fn persist(
        &self,
        db: &rusqlite::Connection,
        id: &AppId,
        operation: Operation,
        result: &Value,
    ) -> Result<(), rv_core::StoreError> {
        storage::persist(db, id, operation, result)
    }
    fn read_data(
        &self,
        db: &rusqlite::Connection,
        id: &AppId,
        dataset: data::Dataset,
        text: &str,
        offset: usize,
    ) -> Result<data::BusinessPage, rv_core::StoreError> {
        storage::read(db, id, dataset, text, offset)
    }
    fn project(&self, operation: Operation, status: u16, response: &Value) -> Value {
        let mut result = json!({"http_status":status});
        if operation == Operation::Refresh {
            result["credentials_updated"] = json!((200..300).contains(&status));
            return result;
        }
        if status == 401 {
            result["session_status"] = json!("已过期");
        }
        if operation == Operation::Session {
            let user = &response["user"];
            result["session_status"] = json!(if status == 200 && user["id"].is_number() {
                "有效"
            } else if status == 401 {
                "已过期"
            } else {
                "未知"
            });
            if status == 200 && user["id"].is_number() {
                result["user_id"] = user["id"].clone();
                for key in ["login", "country_code"] {
                    if user[key].is_string() {
                        result[if key == "country_code" { "market" } else { key }] =
                            user[key].clone();
                    }
                }
            }
        }
        if status == 200 {
            if let Some(unread) = response["unread_msg_count"].as_u64() {
                result["unread_count"] = json!(unread);
            }
            // Retain read results without credential/header/cookie fields.
            result["response"] = redact(response, 0);
        }
        result
    }
}
pub(crate) fn redact(value: &Value, depth: usize) -> Value {
    if depth > 128 {
        return Value::Null;
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    let k = key.to_ascii_lowercase();
                    !k.contains("token")
                        && !k.contains("password")
                        && !k.contains("authorization")
                        && !k.contains("cookie")
                        && k != "headers"
                })
                .map(|(k, v)| (k.clone(), redact(v, depth + 1)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| redact(v, depth + 1)).collect()),
        _ => value.clone(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn response_never_exposes_credentials_or_infers_success_from_forbidden() {
        let result = Vinted.project(
            Operation::Session,
            403,
            &json!({"user":{"id":1},"access_token":"fixture"}),
        );
        assert_eq!(result["session_status"], "未知");
        assert!(!result.to_string().contains("fixture"));
        let result=Vinted.project(Operation::Inbox,200,&json!({"conversations":[{"id":1,"message":"hello","nested":{"refresh_token":"fixture"}}]}));
        assert_eq!(result["response"]["conversations"][0]["message"], "hello");
        assert!(!result.to_string().contains("fixture"));
    }
    #[test]
    fn refresh_uses_fixed_endpoint_and_encodes_token_as_form_data() {
        let auth = AppAuth {
            access_token: "fixture".into(),
            refresh_token: Some("a&b+=".into()),
            headers: Default::default(),
            ..AppAuth::default()
        };
        let plan = Vinted.request(Operation::Refresh, &auth).unwrap();
        assert_eq!(plan.method, reqwest::Method::POST);
        assert!(plan.body.unwrap().ends_with("a%26b%2B%3D"));
    }
}
