//! Explicit HTTP code source, separate from mailbox providers and phone operations.
use super::valid_code;
use serde_json::Value;
use std::time::Duration;

pub fn validate_code_url(text: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(text).map_err(|_| "验证码链接格式无效")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("验证码链接须为不带用户信息或片段的 HTTP(S) 地址".into());
    }
    Ok(())
}

/// Only a complete six-digit value or explicit JSON code fields are accepted.
/// Do not guess arbitrary six-digit substrings in IDs, dates or an email body.
pub fn extract_verification_code(body: &str) -> Result<String, String> {
    let body = body.trim();
    if valid_code(body) {
        return Ok(body.into());
    }
    let value: Value =
        serde_json::from_str(body).map_err(|_| "接口未返回明确的六位验证码，可手动粘贴验证码")?;
    let mut codes = Vec::new();
    for candidate in [&value, &value["data"], &value["result"]] {
        if let Some(s) = candidate.as_str()
            && valid_code(s)
        {
            codes.push(s.to_owned());
        }
        for key in ["code", "verification_code", "verificationCode", "otp"] {
            if let Some(s) = candidate.get(key).and_then(Value::as_str)
                && valid_code(s)
            {
                codes.push(s.to_owned());
            }
        }
    }
    codes.sort();
    codes.dedup();
    if codes.len() != 1 {
        return Err("接口验证码缺失或不唯一，可手动粘贴六位验证码".into());
    }
    Ok(codes.remove(0))
}
pub(super) async fn fetch_code(url: &str) -> Result<String, String> {
    validate_code_url(url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建验证码请求")?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "读取验证码失败，请手动输入或点击重新读取")?;
    if !response.status().is_success() {
        return Err("验证码接口返回失败状态".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "验证码响应读取失败")? {
        if body.len() + chunk.len() > 65536 {
            return Err("验证码响应过大".into());
        }
        body.extend_from_slice(&chunk);
    }
    extract_verification_code(std::str::from_utf8(&body).map_err(|_| "验证码响应不是 UTF-8 文本")?)
}
