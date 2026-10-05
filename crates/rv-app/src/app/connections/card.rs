//! Stored desktop preview and selection-local actions.
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
    pub(super) fn render_card(
        &self,
        conn: &Connection,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = conn.id;
        let selected = self.selected == Some(id);
        let group: SharedString = format!("connection-{id}").into();
        let thumb = self.book.paths().thumb_path(id);
        let has_preview = !self.book.prefs().hide_screenshots && thumb.exists();

        v_flex()
            .id(ElementId::from(format!("card-{id}")))
            .debug_selector(move || format!("card-{id}"))
            .group(group.clone())
            .w(width)
            .flex_shrink_0()
            .rounded_lg()
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
            .when(!selected, |el| {
                el.hover(|s| s.bg(theme::hover(cx)).border_color(theme::accent(cx)))
            })
            .overflow_hidden()
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
                div()
                    .relative()
                    .m_2()
                    .mb_0()
                    .h(px((f32::from(width) * 0.54).clamp(124., 170.)))
                    .rounded_md()
                    .overflow_hidden()
                    .w_auto()
                    .bg(theme::thumb_bg(cx))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(if has_preview {
                        img(thumb)
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                            .into_any_element()
                    } else {
                        Icon::new(IconName::Frame)
                            .size_8()
                            .text_color(theme::muted(cx))
                            .into_any_element()
                    })
                    .when(has_preview, |el| {
                        el.child(
                            div()
                                .absolute()
                                .bottom_1()
                                .left_1()
                                .px_1()
                                .rounded_sm()
                                .bg(rgba(0x10161ccc))
                                .text_color(rgb(0xc3cdd8))
                                .text_xs()
                                .child("上次画面"),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top_1()
                            .right_1()
                            .when(!selected, |el| {
                                el.invisible().group_hover(group.clone(), |s| s.visible())
                            })
                            .child(
                                Button::new(SharedString::from(format!("connection-menu-{id}")))
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
                    ),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .h(px(28.))
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_semibold()
                                    .text_ellipsis()
                                    .child(conn.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .w(px(76.))
                                    .h(px(28.))
                                    .when(!selected, |el| {
                                        el.invisible().group_hover(group, |s| s.visible())
                                    })
                                    .child(connect_button(id, self.credential_busy, cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .text_ellipsis()
                            .child(conn.server_display()),
                    )
                    .child(
                        h_flex()
                            .h(px(24.))
                            .gap_1()
                            .items_center()
                            .children(
                                conn.labels
                                    .iter()
                                    .take(1)
                                    .map(|label| label_chip(label, cx)),
                            )
                            .when(conn.labels.len() > 1, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted(cx))
                                        .child(format!("+{}", conn.labels.len() - 1)),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(theme::muted(cx))
                                    .child(
                                        conn.last_connected
                                            .map(relative_time)
                                            .unwrap_or_else(|| "尚未连接".into()),
                                    ),
                            ),
                    ),
            )
    }
}
