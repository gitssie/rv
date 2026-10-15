//! Transient application feedback, separate from the page layout.
use super::AddressBookApp;
use gpui::{Anchor, Context, Window};
use gpui_component::{WindowExt, notification::Notification};
use std::time::Duration;

impl AddressBookApp {
    pub(super) fn notify_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.notified_status.as_ref() == Some(&self.status) {
            return;
        }
        self.notified_status = Some(self.status.clone());
        if self.status.is_empty() || self.status.as_ref() == "Ready" {
            return;
        }
        let message = self.status.clone();
        window.defer(cx, move |window, cx| {
            window.push_notification(
                Notification::info(message)
                    .id::<AddressBookApp>()
                    .placement(Anchor::BottomRight)
                    .autohide(true),
                cx,
            );
        });
        // An explicit deadline also closes feedback while the window is inactive.
        // Replacing the task cancels the previous message deadline.
        self.notification_close_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update_in(cx, |_, window, cx| {
                window.remove_notification::<AddressBookApp>(cx);
            });
        }));
    }
}
