//! Resolve native safety parameters from a fresh phone snapshot, never pasted text.
use super::*;

pub(super) fn needs_current_account(spec: &OperationSpec) -> bool {
    spec.task == 31
        && !matches!(
            spec.method,
            "queryState" | "signInAppStore" | "enableICloud"
        )
}

fn domain_account<'a>(state: &'a Value, domain: &str) -> Result<Option<&'a str>, String> {
    let section = &state[domain];
    match section["status"].as_str() {
        Some("not_signed_in") if section["account"].is_null() => Ok(None),
        Some("signed_in") => section["account"]["apple_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty() && id.len() <= 4096)
            .map(Some)
            .ok_or_else(|| "手机账户资料不完整，未执行操作".into()),
        _ => Err("手机账户状态冲突或不可读取，未执行操作".into()),
    }
}

pub(super) fn current_account(method: &str, state: &Value) -> Result<String, String> {
    if method == "signOutAppStore" {
        return domain_account(state, "app_store")?
            .map(str::to_owned)
            .ok_or_else(|| "手机未登录 App Store，未执行操作".into());
    }
    // iCloud operations must never pick the App Store account or a historical record.
    let cloud = domain_account(state, "icloud")?;
    if matches!(
        method,
        "signOutICloud" | "disableFindMy" | "queryICloudDevices"
    ) {
        return cloud
            .map(str::to_owned)
            .ok_or_else(|| "手机未登录 iCloud，未执行操作".into());
    }
    let identity = domain_account(state, "identity")?;
    if let (Some(cloud), Some(identity)) = (cloud, identity)
        && !cloud.eq_ignore_ascii_case(identity)
    {
        return Err("手机 iCloud 与 Apple 身份不一致，未执行操作".into());
    }
    if let Some(id) = identity.or(cloud) {
        return Ok(id.into());
    }
    // Complete Apple signout can also target a phone signed in to App Store only.
    if method == "signOutAppleID"
        && let Some(id) = domain_account(state, "app_store")?
        && state["app_store"]["account"]["authenticated"] == true
    {
        return Ok(id.into());
    }
    Err("手机未登录此操作所需的 Apple 账户，未执行操作".into())
}

pub(super) async fn query_current_account(
    client: &crate::phone_http::PhoneHttpClient,
    method: &str,
) -> Result<String, String> {
    query_account_state(client)
        .await
        .and_then(|state| current_account(method, &state))
        .map_err(|error| format!("未提交操作：{error}"))
}

pub(super) async fn query_account_state(
    client: &crate::phone_http::PhoneHttpClient,
) -> Result<Value, String> {
    // Read-only preflight uses the shared session HTTP/2 pool.
    // Failures here cannot have submitted the requested mutation.
    let result = tokio::time::timeout(Duration::from_secs(600), async {
        let mut wire = Wire::connect(client).await?;
        let mut record = wire
            .request(
                31,
                json!({"action":"invoke","method":"queryAccountState","params":{}}),
            )
            .await?;
        drop(wire);
        let id = record["operation_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or_else(|| service_error(&record))?
            .to_owned();
        loop {
            if record["operation_id"] != id {
                return Err("手机账户查询标识不匹配".into());
            }
            match record["status"].as_str() {
                Some("completed") => {
                    if record.get("error").is_some() {
                        return Err(service_error(&record));
                    }
                    return Ok(record["result"].clone());
                }
                Some("running") => {}
                _ => return Err("手机账户查询状态无效".into()),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            let mut wire = Wire::connect(client).await?;
            record = wire
                .request(31, json!({"action":"result","operation_id":id}))
                .await?;
        }
    })
    .await;
    result.unwrap_or_else(|_| Err("读取手机当前账户超时".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(cloud: Option<&str>, identity: Option<&str>, store: Option<&str>) -> Value {
        let section = |id: Option<&str>| match id {
            Some(id) => {
                json!({"status":"signed_in","account":{"apple_id":id,"authenticated":true}})
            }
            None => json!({"status":"not_signed_in","account":null}),
        };
        json!({"icloud":section(cloud),"identity":section(identity),"app_store":section(store)})
    }
    #[test]
    fn cloud_logout_uses_cloud_even_when_store_and_identity_differ() {
        assert_eq!(
            current_account(
                "signOutICloud",
                &state(Some("cloud@test"), Some("other@test"), Some("store@test"))
            )
            .unwrap(),
            "cloud@test"
        );
        assert!(current_account("signOutICloud", &state(None, None, Some("store@test"))).is_err());
    }
    #[test]
    fn settings_reject_conflicts_and_ignore_store_profile() {
        assert!(
            current_account(
                "setName",
                &state(Some("cloud@test"), Some("other@test"), None)
            )
            .is_err()
        );
        assert!(current_account("setName", &state(None, None, Some("store@test"))).is_err());
        assert_eq!(
            current_account(
                "queryEmail",
                &state(None, Some("phone@test"), Some("store@test"))
            )
            .unwrap(),
            "phone@test"
        );
        assert_eq!(
            current_account("signOutAppleID", &state(None, None, Some("store@test"))).unwrap(),
            "store@test"
        );
    }
    #[test]
    fn missing_or_conflicting_state_never_selects_a_target() {
        assert!(current_account("signOutICloud", &json!({})).is_err());
        let mut value = state(Some("cloud@test"), None, None);
        value["icloud"]["status"] = json!("conflict");
        assert!(current_account("signOutICloud", &value).is_err());
    }
}
