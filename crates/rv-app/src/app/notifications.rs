//! Transient application feedback, separate from the page layout.
use super::AddressBookApp;
use gpui::{Context, Window};
use gpui_component::notification::Notification;

impl AddressBookApp {
    pub(super) fn notify_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notified_status.as_ref() == Some(&self.status) {
            return;
        }
        self.notified_status = Some(self.status.clone());
        if self.status.is_empty() || self.status.as_ref() == "Ready" {
            return;
        }
        self.notification_close_task = Some(crate::ui::notify(
            Notification::info(self.status.clone()),
            window,
            cx,
        ));
    }
}
