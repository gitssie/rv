//! One provider operation for both URL-based and password-session IP rotation.
use super::{ProxyError, bounded_body};
use crate::network::{self, ProxyClient};
use rv_core::{
    StorePaths,
    workspace::{ProxyRotation, SocksProxy, WorkspaceStore},
};
use std::time::Duration;

/// Called while the managed exit lease is held. The returned client uses the
/// resulting credentials and the complete existing front/exit chain.
pub(super) async fn switch_exit(
    paths: &StorePaths,
    proxy: &mut SocksProxy,
    expected_front: &Option<SocksProxy>,
) -> Result<ProxyClient, ProxyError> {
    match &proxy.ip.rotation {
        ProxyRotation::Http => {
            let url = proxy
                .ip
                .change_url
                .as_deref()
                .ok_or(ProxyError::MissingChangeUrl)?;
            // This is a provider control URL; observation and work still use the proxy.
            let control = reqwest::Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|_| ProxyError::InvalidConfiguration)?;
            match control.get(url).send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        return Err(ProxyError::RotationRejected);
                    }
                    let body = bounded_body(response, 16384)
                        .await
                        .map_err(|_| ProxyError::RotationUnconfirmed)?;
                    if serde_json::from_slice::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("success").cloned())
                        == Some(false.into())
                    {
                        return Err(ProxyError::RotationRejected);
                    }
                }
                Err(_) => { /* Observe the exit before deciding whether it changed. */ }
            }
        }
        ProxyRotation::PasswordSession { .. } => {
            *proxy = WorkspaceStore::open(paths)
                .and_then(|mut store| store.rotate_proxy_password_session(proxy, expected_front))
                .map_err(|_| ProxyError::InvalidConfiguration)?;
        }
    }
    network::probe_snapshot_client(paths, proxy, expected_front)
        .map_err(|_| ProxyError::InvalidConfiguration)
}
