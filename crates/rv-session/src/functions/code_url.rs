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

/// Match an explicitly labelled code, never an arbitrary number in IDs or dates.
fn text_codes(body: &str) -> Vec<String> {
    static CODE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?ix)
            (?:
                \b(?:your\s+)?apple\s+(?:account|id)\s+(?:verification\s+)?code
                | \b(?:verification|security|one[-\s]time|otp)\s+code
                | 验证码 | 驗證碼 | 安全码 | 安全碼
            )
            \s*(?:is|为|為|是)?\s*[:：]?\s*
            ([0-9]{6})
        ",
        )
        .expect("valid verification-code pattern")
    });
    CODE.captures_iter(body)
        .filter_map(|capture| {
            let code = capture.get(1)?;
            // Check the boundary without consuming the next SMS label.
            if body
                .as_bytes()
                .get(code.end())
                .is_some_and(u8::is_ascii_digit)
            {
                return None;
            }
            Some(code.as_str().to_owned())
        })
        .collect()
}

/// Accept complete codes, explicit JSON fields and labelled SMS text.
/// Preserve leading zeroes and reject conflicting candidates.
pub fn extract_verification_code(body: &str) -> Result<String, String> {
    let body = body.trim();
    if valid_code(body) {
        return Ok(body.into());
    }
    let mut codes = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        for candidate in [&value, &value["data"], &value["result"]] {
            if let Some(text) = candidate.as_str() {
                if valid_code(text) {
                    codes.push(text.to_owned());
                } else {
                    codes.extend(text_codes(text));
                }
            }
            for key in ["code", "verification_code", "verificationCode", "otp"] {
                if let Some(code) = candidate.get(key).and_then(Value::as_str)
                    && valid_code(code)
                {
                    codes.push(code.to_owned());
                }
            }
            for key in ["message", "text", "body", "content", "sms"] {
                if let Some(text) = candidate.get(key).and_then(Value::as_str) {
                    codes.extend(text_codes(text));
                }
            }
        }
    } else {
        codes.extend(text_codes(body));
    }
    codes.sort();
    codes.dedup();
    if codes.len() != 1 {
        return Err("暂未获取验证码".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_apple_sms_with_timestamp_and_preserves_leading_zero() {
        assert_eq!(
            extract_verification_code(
                "Your Apple Account code is: 053322. Do not share it with anyone.|2026-11-29"
            )
            .unwrap(),
            "053322"
        );
    }

    #[test]
    fn labeled_codes_exclude_long_numbers_dates_and_ambiguous_codes() {
        assert_eq!(
            extract_verification_code("Apple ID verification code: 001234.").unwrap(),
            "001234"
        );
        assert_eq!(
            extract_verification_code("您的 Apple 验证码是：053322，请勿分享。").unwrap(),
            "053322"
        );
        assert!(extract_verification_code("Your Apple Account code is: 10533220.").is_err());
        assert!(extract_verification_code("Your Apple Account code is: 0533220.").is_err());
        assert!(extract_verification_code("2026-11-29 / id=053322").is_err());
        assert!(
            extract_verification_code("Apple Account code: 053322. Apple ID code: 001234.")
                .is_err()
        );
        assert!(extract_verification_code("验证码：053322验证码：001234").is_err());
        assert!(extract_verification_code(r#"{"id":"053322","date":"2026-11-29"}"#).is_err());
        assert_eq!(
            extract_verification_code(
                r#"{"data":{"message":"Your Apple Account code is: 053322.|2026-11-29"}}"#
            )
            .unwrap(),
            "053322"
        );
    }
}
