//! RV capability adapter over the standalone ZXTouch transport.
use serde_json::Value;
#[derive(Clone)]
pub struct PhoneHttpClient(zxtouch::PhoneHttpClient);
impl std::ops::Deref for PhoneHttpClient {
    type Target = zxtouch::PhoneHttpClient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl PhoneHttpClient {
    pub fn new(host: &str, port: u16, token: String) -> Result<Self, reqwest::Error> {
        zxtouch::PhoneHttpClient::new(host, port, token).map(Self)
    }
    pub fn api(&self) -> zxtouch::Api {
        zxtouch::Api::new(self.0.clone())
    }
    pub(crate) fn isolated(&self) -> Result<Self, reqwest::Error> {
        self.0.isolated().map(Self)
    }
    pub(crate) async fn connect(&self, password: &str) -> Result<crate::AppCapabilities, String> {
        let (status, response) = self
            .json(
                "/api/control/connect",
                &serde_json::json!({"password": password}),
            )
            .await?;
        let caps = self.accept_connect(status, response)?;
        // Keep token acceptance atomic with RV capability validation.
        let token = caps.zx_control.as_ref().unwrap().token.clone();
        self.0.set_token(token);
        Ok(caps)
    }
    fn accept_connect(
        &self,
        status: u16,
        response: Value,
    ) -> Result<crate::AppCapabilities, String> {
        if status != 200 {
            return Err(if status == 401
                && response["error"] == "Control authorization expired or unavailable"
            {
                "ZXTouch 认证入口被前置授权拦截；请更新 TrollVNC 3.2-320 或更新版本并重启服务"
            } else if status == 401 && response["error"] == "authentication_failed" {
                "ZXTouch 密码验证失败"
            } else {
                "ZXTouch 独立控制接口不可用，请更新手机版"
            }
            .into());
        }
        let token = response["access_token"]
            .as_str()
            .filter(|t| t.len() <= 4096 && t.split('.').count() == 3)
            .ok_or("ZXTouch 未返回 JWT")?;
        if response["token_type"] != "Bearer" {
            return Err("ZXTouch 认证返回不正确".into());
        }
        let caps: crate::AppCapabilities = serde_json::from_value(response["capabilities"].clone())
            .map_err(|_| "ZXTouch 能力信息不正确")?;
        if !caps
            .zx_control
            .as_ref()
            .is_some_and(|e| e.version == 3 && e.token == token)
        {
            return Err("ZXTouch 独立控制版本不正确".into());
        }
        Ok(caps)
    }
}
pub(crate) fn app_route(op: u8) -> Option<&'static str> {
    Some(match op {
        0 => "/api/control/authorize",
        1 => "/api/apps/list",
        2 => "/api/apps/launch",
        3 => "/api/apps/stop",
        4 => "/api/apps/restart",
        5 => "/api/apps/foreground",
        6 => "/api/apps/icon",
        12 => "/api/device/home",
        16 => "/api/device/clipboard",
        25 => "/api/device/info",
        _ => return None,
    })
}
