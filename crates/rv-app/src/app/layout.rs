//! Address-book shell: compose modules without connection or persistence logic.
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{TitleBar, h_flex, v_flex};

use super::{AddressBookApp, Modal};
use crate::{actions::*, theme};

impl Render for AddressBookApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_component_dialog(window, cx);
        self.notify_status(window, cx);
        self.sync_phone_controls(window, cx);
        self.apply_chat_input_result(window, cx);
        self.window_controls.sync(window, theme::is_dark(cx));
        let sidebar_width = if self.collapsed {
            px(0.)
        } else {
            theme::sidebar_width()
        };
        let content_width = window.viewport_size().width - sidebar_width - px(32.);

        v_flex()
            .id("address-book")
            .relative()
            .size_full()
            .bg(theme::surface(cx))
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
            .on_action(|_: &HomeFullscreen, window, _| window.toggle_fullscreen())
            .child(
                TitleBar::new()
                    .h(theme::toolbar_height())
                    .bg(theme::card(cx))
                    .border_color(theme::line(cx))
                    .when(self.settings.page.is_some(), |bar| {
                        bar.child(self.render_settings_toolbar(cx))
                    })
                    .when(self.settings.page.is_none(), |bar| {
                        bar.child(self.render_toolbar(cx))
                    }),
            )
            .when(self.settings.page.is_some(), |el| {
                el.child(self.render_settings(cx))
            })
            .when(self.settings.page.is_none(), |el| {
                el.child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .when(!self.collapsed, |el| el.child(self.render_sidebar(cx)))
                        .child(
                            v_flex()
                                .id("content")
                                .debug_selector(|| "home-content".into())
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .px_4()
                                .pt_3()
                                .gap_4()
                                .child(self.render_home(content_width, cx)),
                        ),
                )
            })
            .when(!matches!(self.modal, Modal::None), |el| {
                el.child(self.render_modal(cx))
            })
    }
}
