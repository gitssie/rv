//! Transient application feedback, separate from the page layout.
use super::AddressBookApp;
use gpui::{Context, Window};
use gpui_component::notification::Notification;

impl AddressBookApp {
    pub(super) fn set_error_status(
        &mut self,
        text: impl Into<gpui::SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.set_status(text, cx);
        self.error_status = Some(self.status.clone());
    }

    pub(super) fn notify_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chat)=self.chat.window {
            if chat.window_id()!=window.window_handle().window_id() && chat.is_active(cx)==Some(true){return;}
            if chat.window_id()==window.window_handle().window_id() && !window.is_window_active(){return;}
        }
        if self.notified_status.as_ref() == Some(&self.status) {
            return;
        }
        self.notified_status = Some(self.status.clone());
        if self.status.is_empty() || self.status.as_ref() == "Ready" {
            return;
        }
        self.notification_close_task = Some(crate::ui::notify(
            if self.error_status.take().as_ref() == Some(&self.status) {
                Notification::error(self.status.clone())
            } else {
                Notification::info(self.status.clone())
            },
            window,
            cx,
        ));
    }
}
