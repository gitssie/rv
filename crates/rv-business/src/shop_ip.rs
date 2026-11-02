//! Read-only IP observation through the shop's actual proxy and password Session.
use crate::{network, proxy};
use rv_core::{StorePaths, workspace::AppId};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct ShopIp {
    pub ip: String,
    pub country: String,
    pub region: String,
    pub city: String,
}

pub async fn inspect(paths: StorePaths, id: AppId) -> Result<ShopIp, String> {
    crate::executor::runtime()?
        .spawn(async move {
            let direct = reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|_| "无法创建 IP 检测客户端")?;
            let (route, _lease) = network::acquire_inspection_route(&paths, &id, &direct).await?;
            let response = route
                .client
                .get("https://ipwho.is/?fields=success,message,ip,country,region,city")
                .header(reqwest::header::CACHE_CONTROL, "no-cache")
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(|_| "IP 检测请求失败")?;
            if !response.status().is_success() {
                return Err("IP 检测服务返回错误".into());
            }
            let body = proxy::bounded_body(response, 16384)
                .await
                .map_err(|_| "IP 检测响应读取失败")?;
            let value: serde_json::Value =
                serde_json::from_slice(&body).map_err(|_| "IP 检测响应格式无效")?;
            decode(&value)
        })
        .await
        .map_err(|_| "IP 检测已中断")?
}

fn decode(value: &serde_json::Value) -> Result<ShopIp, String> {
    if value["success"] != true {
        return Err("IP 检测服务未返回成功结果".into());
    }
    let text = |key: &str| value[key].as_str().unwrap_or("").to_owned();
    let ip = text("ip");
    if ip.is_empty() {
        return Err("IP 检测响应缺少 IP".into());
    }
    Ok(ShopIp {
        ip,
        country: text("country"),
        region: text("region"),
        city: text("city"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_provider_values_and_rejects_failure() {
        let result = decode(&serde_json::json!({"success":true,"ip":"1.2.3.4","country":"France","region":"Île-de-France","city":"Paris"})).unwrap();
        assert_eq!(result.country, "France");
        assert_eq!(result.region, "Île-de-France");
        assert_eq!(result.city, "Paris");
        assert!(decode(&serde_json::json!({"success":false,"message":"failure"})).is_err());
        assert!(decode(&serde_json::json!({"success":true})).is_err());
    }
}
