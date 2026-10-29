use super::*;
use base64::Engine;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppDeviceInfo {
    pub model: String,
    pub os_version: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AppRequestContext {
    pub bundle_id: String,
    pub idfv: String,
    pub model: String,
    pub os_version: String,
    pub app_version: String,
    pub app_build: String,
}
impl std::fmt::Debug for AppRequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppRequestContext([redacted])")
    }
}
pub(super) fn keychain_value(
    records: &Plist,
    service: &str,
    account: &str,
) -> Result<Option<Vec<u8>>, StoreError> {
    let matches: Vec<_> = records
        .as_array()
        .ok_or_else(|| invalid("Keychain 记录格式错误"))?
        .iter()
        .filter(|r| text(r, "svce") == Some(service) && text(r, "acct") == Some(account))
        .collect();
    if matches.len() > 1 {
        return Err(invalid("Keychain 中有重复身份记录"));
    }
    matches
        .first()
        .map(|r| {
            at(r, "v_Data")?
                .as_data()
                .map(|d| d.to_vec())
                .ok_or_else(|| invalid("Keychain 值格式错误"))
        })
        .transpose()
}
fn stored_string(
    records: &Plist,
    service: &str,
    account: &str,
) -> Result<Option<String>, StoreError> {
    keychain_value(records, service, account)?
        .map(|v| String::from_utf8(v).map_err(|_| invalid("Keychain 字符串编码无效")))
        .transpose()
}
fn pref<'a>(preferences: &'a Plist, bundle: &str, key: &str) -> Option<&'a Plist> {
    let records = preferences.as_dictionary()?.get("records")?.as_array()?;
    // Prefer the App's container-backed any-host domain; never an SDK/group domain.
    records
        .iter()
        .find(|r| {
            text(r, "domain") == Some(bundle)
                && text(r, "container") == Some("data")
                && text(r, "host") == Some("any")
        })?
        .as_dictionary()?
        .get("values")?
        .as_dictionary()?
        .get(key)
}
pub(super) fn credentials(state: &ShadowAppState) -> Result<AppAuth, StoreError> {
    let manifest = parse(&state.manifest)?;
    let bundle = text(&manifest, "bundleID").ok_or_else(|| invalid("state Bundle ID 缺失"))?;
    if bundle != "lt.manodrabuziai.fr" {
        return Err(invalid("此 App 尚未接入凭据解析"));
    }
    let idfv = text(&manifest, "idfv").ok_or_else(|| invalid("state IDFV 缺失"))?;
    let records = parse(&state.payloads[0])?;
    let preferences = parse(&state.payloads[2])?;
    let access = stored_string(&records, "Vintedfr", "token")?
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("state 没有 Vinted access token"))?;
    let refresh = stored_string(&records, "Vintedfr", "refresh_token")?.filter(|s| !s.is_empty());
    let context = AppRequestContext {
        bundle_id: bundle.into(),
        idfv: idfv.into(),
        model: state.device.model.clone(),
        os_version: state.device.os_version.clone(),
        app_version: text(&manifest, "version").unwrap_or("").into(),
        app_build: text(&manifest, "build").unwrap_or("").into(),
    };
    if [
        &context.model,
        &context.os_version,
        &context.app_version,
        &context.app_build,
    ]
    .iter()
    .any(|s| s.is_empty() || s.len() > 128 || s.contains(['\r', '\n']))
    {
        return Err(invalid("请求上下文缺少设备或 App 版本信息"));
    }
    let mut headers = BTreeMap::new();
    headers.insert(
        "User-Agent".into(),
        format!(
            "vinted-ios Vinted/{} ({}; build:{}; iOS {}) {}",
            context.app_version, bundle, context.app_build, context.os_version, context.model
        ),
    );
    headers.insert("X-Device-Model".into(), context.model.clone());
    headers.insert("X-App-Version".into(), context.app_version.clone());
    headers.insert("Short-Bundle-Version".into(), context.app_version.clone());
    if let Some(value) = stored_string(&records, "deviceIdentifierService", "deviceIdentifierKey")?
    {
        headers.insert("X-Device-UUID".into(), value);
    }
    if let Some(value) = keychain_value(&records, "VintedSessionStorage", "user")? {
        let user: Value =
            serde_json::from_slice(&value).map_err(|_| invalid("Vinted session 存储格式错误"))?;
        if let Some(value) = user["anonId"].as_str().filter(|s| !s.is_empty()) {
            headers.insert("X-Anon-Id".into(), value.into());
        }
    }
    if let Some(uuid) =
        pref(&preferences, bundle, "DeviceTokenSessionUUID").and_then(Plist::as_string)
    {
        if uuid::Uuid::parse_str(uuid).is_ok() {
            if let Some(value) = stored_string(
                &records,
                &format!("DeviceToken-{bundle}-{uuid}"),
                "DeviceTokenKeychainValue",
            )? {
                headers.insert("X-V-Udt".into(), value);
            }
        }
    }
    // DeviceTokenSessionUUID selects the installation's token storage. It is
    // deliberately never persisted as X-Session-Id (a task-execution request ID).
    if let Some(locale) = pref(&preferences, bundle, "ISOLocaleDefaultKey")
        .and_then(Plist::as_string)
        .filter(|s| !s.is_empty())
    {
        let locale = locale.replace('_', "-");
        headers.insert("Locale".into(), locale.clone());
    }
    if let Some(language) = pref(&preferences, bundle, "CurrentLanguageCodeDefaultsKey")
        .and_then(Plist::as_string)
        .filter(|s| !s.is_empty())
    {
        headers.insert("Accept-Language".into(), language.into());
    }
    let expires_at = pref(&preferences, bundle, "token_expiration")
        .and_then(Plist::as_date)
        .and_then(|d| {
            std::time::SystemTime::from(d)
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
        })
        .map(|d| d.as_secs() as i64);
    let mut auth = AppAuth {
        access_token: access,
        refresh_token: refresh,
        headers,
        expires_at,
        ..AppAuth::default()
    };
    auth.extra
        .insert("request_context".into(), serde_json::to_value(context)?);
    if let Some(scope) = pref(&preferences, bundle, "token_scope").and_then(Plist::as_string) {
        auth.extra
            .insert("scope".into(), Value::String(scope.into()));
    }
    Ok(auth)
}
pub(super) fn profile_metadata(auth: &AppAuth) -> Value {
    let mut result = serde_json::json!({"session":"unknown","identity_source":"cached_token_unverified","has_refresh_token":auth.refresh_token.is_some()});
    if let Some(part) = auth.access_token.split('.').nth(1).and_then(|p| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(p.trim_end_matches('='))
            .ok()
    }) {
        if let Ok(claims) = serde_json::from_slice::<Value>(&part) {
            for key in ["account_id", "sub", "cc", "exp"] {
                if claims[key].is_string() || claims[key].is_number() {
                    result[key] = claims[key].clone();
                }
            }
            if let Some(user) = claims.get("account_id") {
                result["user_id"] = user.clone();
            }
            if let Some(market) = claims.get("cc") {
                result["market"] = market.clone();
            }
        }
    }
    result
}
pub(super) fn patch_credentials(
    state: &mut ShadowAppState,
    auth: &AppAuth,
) -> Result<(), StoreError> {
    let mut records = parse(&state.payloads[0])?;
    let rows = records
        .as_array_mut()
        .ok_or_else(|| invalid("Keychain 记录格式错误"))?;
    for (account, value) in [
        ("token", Some(auth.access_token.as_str())),
        ("refresh_token", auth.refresh_token.as_deref()),
    ] {
        let Some(value) = value else {
            rows.retain(|r| {
                !(text(r, "svce") == Some("Vintedfr") && text(r, "acct") == Some(account))
            });
            continue;
        };
        let mut count = 0;
        for record in rows
            .iter_mut()
            .filter(|r| text(r, "svce") == Some("Vintedfr") && text(r, "acct") == Some(account))
        {
            if at(record, "v_Data")?.as_data() != Some(value.as_bytes()) {
                set(record, "v_Data", Plist::Data(value.as_bytes().to_vec()))?;
            }
            count += 1;
        }
        if count != 1 {
            return Err(invalid("无法更新唯一的 Vinted 凭据记录"));
        }
    }
    let encoded = encode(&records)?;
    // Avoid rewriting unchanged plists merely because of serialization choices.
    let credentials_changed = parse(&state.payloads[0])? != records;
    if credentials_changed {
        state.payloads[0] = encoded;
    }
    let manifest = parse(&state.manifest)?;
    let bundle = text(&manifest, "bundleID").ok_or_else(|| invalid("Bundle ID 缺失"))?;
    let mut preferences = parse(&state.payloads[2])?;
    let old = preferences.clone();
    let scope_changed = auth
        .extra
        .get("scope")
        .and_then(Value::as_str)
        .is_some_and(|scope| {
            pref(&preferences, bundle, "token_scope").and_then(Plist::as_string) != Some(scope)
        });
    let stored_expiry = pref(&preferences, bundle, "token_expiration")
        .and_then(Plist::as_date)
        .and_then(|date| {
            std::time::SystemTime::from(date)
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
        })
        .map(|value| value.as_secs() as i64);
    let update_all_hosts = credentials_changed
        || scope_changed
        || stored_expiry != auth.expires_at.filter(|v| *v >= 0);
    if let Some(rows) = preferences
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("records"))
        .and_then(Plist::as_array_mut)
    {
        for record in rows.iter_mut().filter(|r| {
            text(r, "domain") == Some(bundle)
                && text(r, "container") == Some("data")
                && (text(r, "host") == Some("any")
                    || (update_all_hosts && text(r, "host") == Some("current")))
        }) {
            if let Some(values) = record
                .as_dictionary_mut()
                .and_then(|d| d.get_mut("values"))
                .and_then(Plist::as_dictionary_mut)
            {
                // Keep the server-returned scope alongside the rotated token pair.
                // Missing scope does not invent a replacement for archived metadata.
                if let Some(scope) = auth.extra.get("scope").and_then(Value::as_str) {
                    values.insert("token_scope".into(), Plist::String(scope.to_owned()));
                }
                if let Some(expiry) = auth.expires_at.filter(|v| *v >= 0) {
                    let stored = values
                        .get("token_expiration")
                        .and_then(Plist::as_date)
                        .and_then(|date| {
                            std::time::SystemTime::from(date)
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()
                        })
                        .map(|v| v.as_secs() as i64);
                    if stored != Some(expiry) {
                        values.insert(
                            "token_expiration".into(),
                            Plist::Date(
                                (std::time::UNIX_EPOCH
                                    + std::time::Duration::from_secs(expiry as u64))
                                .into(),
                            ),
                        );
                    }
                } else {
                    values.remove("token_expiration");
                }
            }
        }
    }
    if old != preferences {
        state.payloads[2] = encode(&preferences)?;
    }
    Ok(())
}
