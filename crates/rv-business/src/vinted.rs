mod product;
use super::*;
mod storage;
pub struct Vinted;
impl BusinessStrategy for Vinted {
    fn product_refresh(&self, id: &str) -> Result<Box<dyn BusinessStrategy>, String> {
        Ok(Box::new(product::ProductDetails::new(id)?))
    }
    fn requires_proxy(&self) -> bool {
        true
    }
    fn request(&self, operation: Operation, auth: &AppAuth) -> Result<RequestPlan, String> {
        let url = match operation {
            Operation::Session => "https://www.vinted.com/api/v2/users/current",
            Operation::Inbox => "https://api.vinted.com/messaging/main/inbox",
            Operation::NotificationStats => {
                "https://api.vinted.com/messaging/main/users/unread_count"
            }
            Operation::Notifications => {
                return Err("通知列表请求需要分页参数".into());
            }
            Operation::Refresh => "https://api.vinted.com/oauth/api/token.issue",
            Operation::Products | Operation::Orders => {
                return Err("列表请求需要店铺及分页参数".into());
            }
        };
        let body=if operation==Operation::Refresh {
            let token=auth.refresh_token.as_deref().filter(|s|!s.is_empty()).ok_or("备份没有 refresh token")?;
            let scope="user";
            let encode=|value:&str|value.bytes().map(|byte|if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte){(byte as char).to_string()}else{format!("%{byte:02X}")}).collect::<String>();
            Some([("grant_type","refresh_token"),("client_id","ios"),("scope",scope),("refresh_token",token)].into_iter().map(|(key,value)|format!("{key}={}",encode(value))).collect::<Vec<_>>().join("&"))
        }else{None};
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
        if operation == Operation::Notifications {
            vec![Some("unread_count"), Some("list")]
        } else if operation == Operation::Orders {
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
            Operation::Notifications if direction == Some("unread_count") => {
                "https://api.vinted.com/inbox-notifications/v1/notifications/unread_count".into()
            }
            Operation::Notifications => format!(
                "https://api.vinted.com/inbox-notifications/v1/notifications?page={page}&per_page=20"
            ),
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
        if operation == Operation::Notifications && direction == Some("unread_count") {
            *combined = json!({"notifications":[],"code":0,"notification_unread_count":response["count"].as_u64()});
            return Ok(false);
        }
        if !matches!(
            operation,
            Operation::Products | Operation::Orders | Operation::Notifications
        ) {
            *combined = response;
            return Ok(false);
        }
        if response.get("code").is_some_and(|code| code != 0) {
            return Err("Vinted 业务回复未成功".into());
        }
        let key = match operation {
            Operation::Products => "items",
            Operation::Notifications => "notifications",
            _ => "my_orders",
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
    fn image_urls(&self, operation: Operation, result: &Value) -> Vec<String> {
        match operation {
            Operation::Notifications => crate::notifications::image_urls(result),
            Operation::Session => result["shop"]["avatar_url"]
                .as_str()
                .map(str::to_owned)
                .into_iter()
                .collect(),
            Operation::Products | Operation::Orders => {
                let dataset = if operation == Operation::Products {
                    crate::data::Dataset::Products
                } else {
                    crate::data::Dataset::Orders
                };
                result["response"][if operation == Operation::Products {
                    "items"
                } else {
                    "my_orders"
                }]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| crate::presentation::vinted(row, dataset).image_url)
                .collect()
            }
            _ => vec![],
        }
    }
    fn task_result(&self, operation: Operation, result: &Value) -> Value {
        if result["http_status"].as_u64().is_some_and(|status|status>=400) {return result.clone();}
        if operation == Operation::Session {
            let mut summary = json!({});
            for key in [
                "http_status",
                "session_status",
                "user_id",
                "login",
                "market",
                "shop",
            ] {
                if let Some(value) = result.get(key) {
                    summary[key] = value.clone();
                }
            }
            summary
        } else if matches!(
            operation,
            Operation::Products | Operation::Orders | Operation::Notifications
        ) {
            let key = match operation {
                Operation::Products => "items",
                Operation::Notifications => "notifications",
                _ => "my_orders",
            };
            json!({"http_status":result["http_status"],"record_count":result["response"][key].as_array().map_or(0,Vec::len)})
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
        if operation == Operation::Notifications {
            return crate::notifications::persist(db, id, result);
        }
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
        if status==401 {result["session_status"]=json!("已过期");}
        if status>=400 {
            if operation==Operation::Session && status!=401 {result["session_status"]=json!("未知");}
            if operation==Operation::Refresh {result["credentials_updated"]=json!(false);}
            let code=response["error"].as_str().or_else(||response["error_code"].as_str()).or_else(||response["error"]["code"].as_str()).map(str::to_owned)
                .or_else(||response["code"].as_i64().map(|value|value.to_string()));
            let description=response["error_description"].as_str().or_else(||response["message"].as_str()).or_else(||response["error"]["message"].as_str()).unwrap_or_default();
            if let Some(code)=&code {result["error_code"]=json!(code);}
            if !description.is_empty() {result["error_description"]=json!(description);}
            result["error"]=json!(format!("HTTP {status}{}{}",code.map(|c|format!(" · {c}")).unwrap_or_default(),if description.is_empty(){String::new()}else{format!("：{description}")}));
            return result;
        }
        if operation == Operation::Refresh {
            result["credentials_updated"] = json!((200..300).contains(&status));
            return result;
        }
        if status == 401 {
            result["session_status"] = json!("已过期");
        }
        if operation == Operation::Session {
            let user = &response["user"];
            let profile = (status == 200)
                .then(|| crate::shop::ShopProfile::from_user(user))
                .flatten();
            result["session_status"] = json!(if profile.is_some() {
                "有效"
            } else if status == 401 {
                "已过期"
            } else {
                "未知"
            });
            if let Some(profile) = profile {
                result["shop"] = json!(profile);
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
            if operation == Operation::NotificationStats {
                result["unread_count"] = json!(response["unread_count"].as_u64());
            }
            if operation == Operation::Notifications {
                result["notification_unread_count"] =
                    json!(response["notification_unread_count"].as_u64());
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
    fn unread_quantities_use_verified_endpoints_and_never_count_list_rows() {
        let auth = AppAuth::default();
        assert_eq!(
            Vinted
                .request(Operation::NotificationStats, &auth)
                .unwrap()
                .url,
            "https://api.vinted.com/messaging/main/users/unread_count"
        );
        assert_eq!(
            Vinted.project(
                Operation::NotificationStats,
                200,
                &json!({"unread_count":31})
            )["unread_count"],
            31
        );
        assert!(
            Vinted.project(Operation::NotificationStats, 200, &json!({}))["unread_count"].is_null()
        );
        let mut combined = Value::Null;
        assert!(
            !Vinted
                .collect_page(
                    Operation::Notifications,
                    json!({"count":7}),
                    &mut combined,
                    1,
                    Some("unread_count")
                )
                .unwrap()
        );
        assert!(!Vinted.collect_page(Operation::Notifications,json!({"notifications":[{"id":1,"read":false}],"pagination":{"current_page":1,"total_pages":1}}),&mut combined,1,Some("list")).unwrap());
        assert_eq!(
            Vinted.project(Operation::Notifications, 200, &combined)["notification_unread_count"],
            7
        );
        let mut missing = Value::Null;
        Vinted
            .collect_page(
                Operation::Notifications,
                json!({}),
                &mut missing,
                1,
                Some("unread_count"),
            )
            .unwrap();
        assert!(
            Vinted.project(Operation::Notifications, 200, &missing)["notification_unread_count"]
                .is_null()
        );
    }
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
        let mut auth = AppAuth {
            access_token: "fixture".into(),
            refresh_token: Some("a&b+=".into()),
            headers: Default::default(),
            ..AppAuth::default()
        };
        auth.extra.insert("scope".into(),json!("public user"));
        let plan = Vinted.request(Operation::Refresh, &auth).unwrap();
        assert_eq!(plan.method, reqwest::Method::POST);
        let body=plan.body.unwrap();
        assert!(body.contains("&scope=user&"));
        assert!(!body.contains("public"));
        assert!(body.ends_with("a%26b%2B%3D"));
    }
    #[test]
    fn current_user_projects_shop_fields_without_a_second_endpoint_or_private_profile() {
        let auth = AppAuth {
            access_token: "fixture".into(),
            ..AppAuth::default()
        };
        assert_eq!(
            Vinted.request(Operation::Session, &auth).unwrap().url,
            "https://www.vinted.com/api/v2/users/current"
        );
        let response = json!({"user":{"id":42,"login":"shop","country_code":"fr","country_iso_code":"FR","country_title":"France","photo":{"url":"https://images.vinted.net/avatar.jpg"},"feedback_reputation":0.96,"feedback_count":25,"item_count":8,"total_items_count":30,"email":"private-address","default_address":{"street":"private-street"}}});
        let result = Vinted.project(Operation::Session, 200, &response);
        let shop = &result["shop"];
        assert_eq!(shop["user_id"], 42);
        assert_eq!(shop["login"], "shop");
        assert_eq!(shop["country_code"], "FR");
        assert_eq!(shop["country_title"], "France");
        assert_eq!(shop["item_count"], 8);
        assert_eq!(shop["feedback_reputation"], 0.96);
        assert_eq!(shop["feedback_count"], 25);
        assert_eq!(shop["avatar_url"], "https://images.vinted.net/avatar.jpg");
        let summary = Vinted.task_result(Operation::Session, &result).to_string();
        assert!(!summary.contains("private-address") && !summary.contains("private-street"));
        assert!(
            Vinted
                .project(Operation::Session, 403, &response)
                .get("shop")
                .is_none()
        );
    }
    #[test]
    fn absent_shop_fields_stay_unknown_and_invalid_identity_never_becomes_valid() {
        let result = Vinted.project(
            Operation::Session,
            200,
            &json!({"user":{"id":42,"photo":null}}),
        );
        assert!(result["shop"]["feedback_reputation"].is_null());
        assert!(result["shop"]["item_count"].is_null());
        assert!(result["shop"]["avatar_url"].is_null());
        for id in [json!(0), json!(-1), json!("42")] {
            let result = Vinted.project(Operation::Session, 200, &json!({"user":{"id":id}}));
            assert_eq!(result["session_status"], "未知");
            assert!(result.get("shop").is_none());
        }
    }
}
