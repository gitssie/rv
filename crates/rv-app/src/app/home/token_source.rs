use super::*;
use gpui_component::{Disableable, button::{Button, ButtonVariants}};
use rv_core::workspace::TokenSource;

impl AddressBookApp {
    pub(super) fn render_phone_token_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let Ok(TokenSource::Phone(device)) = WorkspaceStore::open(self.book.paths())
            .and_then(|store| store.token_source(&id)) else {
                return div().into_any_element();
            };
        Button::new("shop-phone-token")
            .ghost()
            .icon(crate::assets::FunctionIcon::Refresh)
            .debug_selector(|| "shop-phone-token".into())
            .tooltip("从关联手机获取最新凭据")
            .disabled(self.read_only || self.home.handoff_running.contains_key(&id))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !device.is_some_and(|phone| this.phone_online(phone, cx)) {
                    this.set_error_status("关联手机尚未连接", cx);
                    return;
                }
                match WorkspaceStore::open(this.book.paths())
                    .and_then(|store| store.request_phone_token(&id, true)) {
                    Ok(()) => this.set_status("正在从关联手机读取凭据…", cx),
                    Err(error) => this.set_error_status(error.to_string(), cx),
                }
            }))
            .into_any_element()
    }
}
