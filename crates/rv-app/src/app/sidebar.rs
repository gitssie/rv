//! Independently scrolling device sidebar; no navigation section headings.
use super::AddressBookApp;
use gpui::*;
impl AddressBookApp {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_device_sidebar(cx)
    }
}
