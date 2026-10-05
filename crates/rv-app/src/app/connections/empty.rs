//! Empty/search result states, without placeholder device cards.
use super::super::{AddressBookApp, SidebarFilter};
use crate::{actions::NewConnection, theme};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable, StyledExt, button::Button, v_flex,
};

impl AddressBookApp {
    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query(cx);
        let (title, hint) = if !query.trim().is_empty() {
            (
                format!("没有匹配“{}”的连接", query.trim()),
                "试试其他名称、地址或标签。",
            )
        } else if self.book.connections().is_empty() {
            (
                "还没有连接".into(),
                "添加一台设备的 VNC 地址，开始远程连接。",
            )
        } else {
            match &self.filter {
                SidebarFilter::Recents => {
                    ("暂无最近使用的连接".into(), "连接过的设备会显示在这里。")
                }
                SidebarFilter::Label(label) => (
                    format!("标签“{label}”下暂无连接"),
                    "可以在编辑连接时设置标签。",
                ),
                SidebarFilter::All => ("暂无连接".into(), ""),
            }
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(IconName::Network)
                    .size(px(32.))
                    .text_color(theme::muted(cx)),
            )
            .child(div().font_semibold().child(title))
            .child(div().text_sm().text_color(theme::muted(cx)).child(hint))
            .when(query.trim().is_empty(), |el| {
                el.child(
                    Button::new("empty-new")
                        .outline()
                        .small()
                        .h(px(28.))
                        .icon(IconName::Plus)
                        .label("新增连接")
                        .disabled(self.credential_busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.on_new(&NewConnection, window, cx);
                        })),
                )
            })
    }
}
