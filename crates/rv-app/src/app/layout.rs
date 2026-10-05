//! Address-book shell: compose modules without connection or persistence logic.
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{StyledExt, TitleBar, h_flex, v_flex};

use super::{AddressBookApp, Modal};
use crate::{actions::*, theme};

impl Render for AddressBookApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.visible(cx);
        let sidebar_width = if self.collapsed {
            px(48.)
        } else {
            theme::sidebar_width()
        };
        let content_width = window.viewport_size().width - sidebar_width - px(40.);

        v_flex()
            .id("address-book")
            .relative()
            .size_full()
            .bg(theme::surface(cx))
            .text_color(theme::ink(cx))
            .text_sm()
            .key_context("AddressBook")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_connect))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_prefs))
            .on_action(cx.listener(Self::on_toggle_view))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_properties))
            .on_action(cx.listener(Self::on_duplicate))
            .on_action(cx.listener(Self::on_close_modal))
            .on_action(|_: &QuitApp, _, cx| cx.quit())
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .justify_center()
                        .font_semibold()
                        .child("RV"),
                ),
            )
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .id("content")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .px_5()
                            .pt_4()
                            .gap_4()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.selected = None;
                                this.focus.focus(window, cx);
                                cx.notify();
                            }))
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_3()
                                    .child(
                                        div().text_lg().font_semibold().child(self.filter.title()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::muted(cx))
                                            .child(format!("{} 个连接", items.len())),
                                    ),
                            )
                            .when(
                                matches!(self.modal, Modal::None)
                                    && self.status.as_ref() != "Ready",
                                |el| el.child(self.render_status(cx)),
                            )
                            .child(self.render_content(&items, content_width, cx)),
                    ),
            )
            .when(!matches!(self.modal, Modal::None), |el| {
                el.child(self.render_modal(cx))
            })
    }
}
