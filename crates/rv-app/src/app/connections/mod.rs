//! Connection collection layout; card, list row, and menus have separate owners.
mod card;
mod empty;
mod menu;
mod row;
mod widgets;

use super::{AddressBookApp, ViewMode};
use gpui::*;
use gpui_component::v_flex;
use rv_core::Connection;

/// Distribute columns across the available width, including the inter-card gaps.
fn card_width(available: Pixels) -> Pixels {
    let width = f32::from(available).max(1.);
    let columns = ((width + 16.) / (232. + 16.)).floor().clamp(1., 3.);
    px((width - (columns - 1.) * 16.) / columns)
}

impl AddressBookApp {
    pub(super) fn render_content(
        &self,
        items: &[Connection],
        available: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if items.is_empty() {
            return self.render_empty(cx).into_any_element();
        }
        match self.view_mode {
            ViewMode::Grid => div()
                .id("grid")
                .flex_1()
                .min_h_0()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .content_start()
                .gap_4()
                .pb_5()
                .overflow_y_scroll()
                .children(items.iter().map(|conn| {
                    self.render_card(conn, card_width(available), cx)
                        .into_any_element()
                }))
                .into_any_element(),
            ViewMode::List => v_flex()
                .id("list")
                .flex_1()
                .min_h_0()
                .w_full()
                .gap_2()
                .pb_5()
                .overflow_y_scroll()
                .children(
                    items
                        .iter()
                        .map(|conn| self.render_row(conn, cx).into_any_element()),
                )
                .into_any_element(),
        }
    }
}
