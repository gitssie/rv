//! List view shares the same connection actions and input behavior as cards.
use super::super::{AddressBookApp, relative_time};
use super::{
    menu::connection_menu,
    widgets::{connect_button, label_chip},
};
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt, DropdownMenu},
    v_flex,
};
use rv_core::Connection;

impl AddressBookApp {
    pub(super) fn render_row(&self, conn: &Connection, cx: &mut Context<Self>) -> impl IntoElement {
        let id = conn.id;
        let selected = self.selected == Some(id);
        let group: SharedString = format!("connection-row-{id}").into();
        h_flex()
            .id(ElementId::from(format!("row-{id}")))
            .debug_selector(move || format!("row-{id}"))
            .group(group.clone())
            .w_full()
            .px_3()
            .py_2()
            .gap_3()
            .rounded_md()
            .items_center()
            .border_1()
            .border_color(if selected {
                theme::accent(cx)
            } else {
                theme::line(cx)
            })
            .bg(if selected {
                theme::selected(cx)
            } else {
                theme::card(cx)
            })
            .when(!selected, |el| el.hover(|s| s.bg(theme::hover(cx))))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| this.select(id, window, cx)),
            )
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.select(id, window, cx);
                if ev.click_count() >= 2 {
                    this.connect_id(id, cx);
                }
            }))
            .context_menu(connection_menu(cx.entity(), id))
            .child(
                Icon::new(IconName::Network)
                    .size_5()
                    .text_color(theme::muted(cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_semibold()
                            .text_ellipsis()
                            .child(conn.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .text_ellipsis()
                            .child(conn.server_display()),
                    ),
            )
            .child(
                h_flex().gap_1().children(
                    conn.labels
                        .iter()
                        .take(1)
                        .map(|label| label_chip(label, cx)),
                ),
            )
            .child(
                div()
                    .w(px(76.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_right()
                    .text_color(theme::muted(cx))
                    .child(conn.last_connected.map(relative_time).unwrap_or_default()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_shrink_0()
                    .when(!selected, |el| {
                        el.invisible().group_hover(group, |s| s.visible())
                    })
                    .child(connect_button(id, self.credential_busy, cx))
                    .child(
                        Button::new(SharedString::from(format!("row-menu-{id}")))
                            .debug_selector(move || format!("connection-menu-{id}"))
                            .ghost()
                            .size(px(28.))
                            .icon(IconName::Ellipsis)
                            .tooltip("更多操作")
                            .accessibility_label("连接的更多操作")
                            .disabled(self.credential_busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.selected = Some(id);
                                cx.notify();
                            }))
                            .dropdown_menu(connection_menu(cx.entity(), id)),
                    ),
            )
    }
}
