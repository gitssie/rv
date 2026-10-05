//! Navigation by all connections, recents and saved labels.
use super::{AddressBookApp, SidebarFilter};
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Icon, IconName, h_flex,
    sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenu, SidebarMenuItem},
};

fn count(count: usize, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(theme::muted(cx))
        .child(count.to_string())
}

impl AddressBookApp {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.book.connections().len();
        let recent = self.book.recents(20).len();
        let labels = self.book.labels();
        Sidebar::new("rv-sidebar")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.collapsed)
            .w(theme::sidebar_width())
            .bg(theme::sidebar(cx))
            .border_r_1()
            .border_color(theme::line(cx))
            .child(
                SidebarGroup::new("连接").child(
                    SidebarMenu::new().children([
                        SidebarMenuItem::new("全部连接")
                            .icon(IconName::LayoutDashboard)
                            .suffix(move |_, cx| count(total, cx))
                            .active(self.filter == SidebarFilter::All)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.filter = SidebarFilter::All;
                                this.selected = None;
                                cx.notify();
                            })),
                        SidebarMenuItem::new("最近使用")
                            .icon(IconName::Calendar)
                            .suffix(move |_, cx| count(recent, cx))
                            .active(self.filter == SidebarFilter::Recents)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.filter = SidebarFilter::Recents;
                                this.selected = None;
                                cx.notify();
                            })),
                    ]),
                ),
            )
            .when(!labels.is_empty(), |side| {
                side.child(SidebarGroup::new("标签").child(SidebarMenu::new().children(
                    labels.into_iter().map(|label| {
                        let active = matches!(&self.filter, SidebarFilter::Label(l) if l == &label);
                        let n = self.book.filtered("", Some(&label)).len();
                        SidebarMenuItem::new(label.clone())
                            .icon(IconName::Star)
                            .suffix(move |_, cx| count(n, cx))
                            .active(active)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.filter = SidebarFilter::Label(label.clone());
                                this.selected = None;
                                cx.notify();
                            }))
                    }),
                )))
            })
            .footer(
                h_flex()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .size_4()
                            .text_color(theme::muted(cx)),
                    )
                    .when(!self.collapsed, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .child("本地连接簿"),
                        )
                    }),
            )
    }
}
