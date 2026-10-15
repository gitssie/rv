use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Icon, IconName, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};

impl AddressBookApp {
    pub(in crate::app) fn render_home(
        &self,
        available: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = f32::from(available).max(1.);
        let columns = ((width + 12.) / 272.).floor().clamp(1., 4.);
        let card_width = if self.view_mode == super::super::ViewMode::List {
            available
        } else {
            px(((width - (columns - 1.) * 12.) / columns).max(1.))
        };
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.search).prefix(Icon::new(IconName::Search))),
                    )
                    .child(
                        Button::new("home-sync-phone")
                            .outline()
                            .icon(FunctionIcon::Smartphone)
                            .disabled(self.selected.is_none() || self.credential_busy)
                            .tooltip("手机备份")
                            .accessibility_label("手机备份")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(id) = this.selected {
                                    this.show_phone_backups(id, None, window, cx);
                                }
                            })),
                    ),
            )
            .child(self.render_app_filters(cx))
            .when_some(self.home.error.clone(), |el, error| {
                el.child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(theme::danger(cx))
                        .child(error)
                        .child(
                            Button::new("home-retry")
                                .ghost()
                                .label("重试")
                                .on_click(cx.listener(|this, _, _, cx| this.reload_home(cx))),
                        ),
                )
            })
            .child(if self.home.apps.is_empty() {
                v_flex()
                    .id("app-empty-state")
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(if self.home.loading {
                        Icon::new(IconName::LoaderCircle)
                            .text_color(theme::muted(cx))
                            .into_any_element()
                    } else {
                        Icon::new(IconName::Inbox)
                            .text_color(theme::muted(cx))
                            .into_any_element()
                    })
                    .into_any_element()
            } else {
                div()
                    .id("app-grid")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .content_start()
                    .gap_3()
                    .pb_4()
                    .overflow_y_scroll()
                    .children(
                        self.home
                            .apps
                            .iter()
                            .map(|app| self.render_app_card(app, card_width, cx)),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }
    fn render_app_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let tags = self.home.available_tags.clone();
        h_flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .children(
                [
                    ("全部", AppFilter::All),
                    ("收藏", AppFilter::Favorites),
                    ("最近使用", AppFilter::Recent),
                    ("归档", AppFilter::Archived),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (name, filter))| {
                    Button::new(SharedString::from(format!("app-filter-{i}")))
                        .debug_selector(move || format!("app-filter-{i}"))
                        .ghost()
                        .icon(match filter {
                            AppFilter::All => FunctionIcon::All,
                            AppFilter::Favorites => FunctionIcon::Favorite,
                            AppFilter::Recent => FunctionIcon::Recent,
                            AppFilter::Archived => FunctionIcon::Archive,
                        })
                        .label(name)
                        .selected(
                            std::mem::discriminant(&self.home.filter)
                                == std::mem::discriminant(&filter),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.home.filter = filter;
                            this.reload_home(cx);
                        }))
                }),
            )
            .child(
                Button::new("app-tags")
                    .ghost()
                    .icon(FunctionIcon::Tags)
                    .label(self.home.tag.clone().unwrap_or("标签".into()))
                    .dropdown_menu({
                        let book = cx.entity();
                        move |menu, _, _| {
                            let reset = book.clone();
                            let mut menu = menu.item(PopupMenuItem::new("全部标签").on_click(
                                move |_, _, cx| {
                                    reset.update(cx, |this, cx| {
                                        this.home.tag = None;
                                        this.reload_home(cx);
                                    })
                                },
                            ));
                            for tag in &tags {
                                let book = book.clone();
                                let tag = tag.clone();
                                menu = menu.item(PopupMenuItem::new(tag.clone()).on_click(
                                    move |_, _, cx| {
                                        book.update(cx, |this, cx| {
                                            this.home.tag = Some(tag.clone());
                                            this.reload_home(cx);
                                        })
                                    },
                                ))
                            }
                            menu
                        }
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child(if self.home.loading {
                        "正在更新…".into()
                    } else {
                        format!("{} 项", self.home.apps.len())
                    }),
            )
            .child(
                Button::new("app-sort")
                    .ghost()
                    .icon(IconName::SortDescending)
                    .label(match self.home.sort {
                        AppSort::Updated => "最近更新",
                        AppSort::Title => "名称",
                        AppSort::LastUsed => "最近使用",
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.home.sort = match this.home.sort {
                            AppSort::Updated => AppSort::Title,
                            AppSort::Title => AppSort::LastUsed,
                            AppSort::LastUsed => AppSort::Updated,
                        };
                        this.reload_home(cx);
                    })),
            )
            .into_any_element()
    }
    fn render_app_card(
        &self,
        app: &AppInstance,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let links = self
            .home
            .links
            .get(&app.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let renderer = self.home.plugins.for_bundle(&app.bundle_id);
        let (name, _symbol, _accent, _action) = renderer.identity(app);
        let edit = app.id.clone();
        let favorite = app.id.clone();
        let menu_id = app.id.clone();
        let avatar = app.id.clone();
        let devices = links
            .iter()
            .filter_map(|b| self.book.get(b.device_id).map(|d| d.name.clone()))
            .collect::<std::collections::BTreeSet<_>>();
        let menu = Button::new(SharedString::from(format!("app-menu-{}", app.id.as_str())))
            .ghost()
            .icon(IconName::Ellipsis)
            .accessibility_label("App 更多操作")
            .dropdown_menu({
                let book = cx.entity();
                let pin = menu_id.clone();
                let archive = menu_id.clone();
                move |menu, _, _| {
                    let editor = book.clone();
                    let pinned = book.clone();
                    let archived = book.clone();
                    let deleted = book.clone();
                    let delete_id = menu_id.clone();
                    menu.item(PopupMenuItem::new("编辑备注与标签…").on_click({
                        let edit = edit.clone();
                        move |_, window, cx| {
                            editor
                                .update(cx, |this, cx| this.open_app_card(edit.clone(), window, cx))
                        }
                    }))
                    .item(PopupMenuItem::new("切换置顶").on_click({
                        let pin = pin.clone();
                        move |_, _, cx| {
                            pinned.update(cx, |this, cx| {
                                this.change_app(pin.clone(), |a| a.pinned = !a.pinned, cx)
                            })
                        }
                    }))
                    .item(PopupMenuItem::new("切换归档").on_click({
                        let archive = archive.clone();
                        move |_, _, cx| {
                            archived.update(cx, |this, cx| {
                                this.change_app(archive.clone(), |a| a.archived = !a.archived, cx)
                            })
                        }
                    }))
                    .item(PopupMenuItem::new("删除…").on_click(move |_, _, cx| {
                        deleted.update(cx, |this, cx| this.open_app_delete(delete_id.clone(), cx));
                    }))
                }
            });
        let header = h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .id(ElementId::from(format!("app-avatar-{}", app.id.as_str())))
                    .debug_selector({
                        let id = app.id.clone();
                        move || format!("app-avatar-{}", id.as_str())
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_app_detail(avatar.clone(), window, cx);
                    }))
                    .child(self.app_avatar(&app.bundle_id)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .font_semibold()
                            .text_ellipsis()
                            .child(app.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .text_ellipsis()
                            .child(name),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("favorite-{}", app.id.as_str())))
                    .ghost()
                    .icon(IconName::Star)
                    .selected(app.favorite)
                    .disabled(self.read_only)
                    .tooltip("收藏")
                    .accessibility_label("收藏")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.change_app(favorite.clone(), |a| a.favorite = !a.favorite, cx)
                    })),
            )
            .child(
                div()
                    .id(SharedString::from(format!(
                        "menu-container-{}",
                        app.id.as_str()
                    )))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(menu),
            );
        let body =
            v_flex()
                .flex_1()
                .p_3()
                .gap_2()
                .child(header)
                .when(!app.tags.is_empty(), |body| {
                    body.child(
                        h_flex()
                            .id(format!("app-card-tags-{}", app.id.as_str()))
                            .debug_selector({
                                let id = app.id.clone();
                                move || format!("app-card-tags-{}", id.as_str())
                            })
                            .gap_1()
                            .items_center()
                            .flex_wrap()
                            .child(Icon::new(FunctionIcon::Tags).text_color(theme::muted(cx)))
                            .children(app.tags.iter().map(|tag| {
                                gpui_component::tag::Tag::secondary().child(tag.clone())
                            })),
                    )
                })
                .child(renderer.card_preview(self.home.previews.get(&app.id), cx))
                .child(renderer.details(app, links.len(), cx))
                .when(!app.note.is_empty(), |body| {
                    body.child(
                        div()
                            .text_xs()
                            .text_ellipsis()
                            .text_color(theme::muted(cx))
                            .child(app.note.clone()),
                    )
                });
        let footer = h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_color(theme::line(cx))
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .text_ellipsis()
                    .child(if devices.is_empty() {
                        String::new()
                    } else {
                        devices.into_iter().collect::<Vec<_>>().join(" · ")
                    }),
            );
        v_flex()
            .id(ElementId::from(format!("app-card-{}", app.id.as_str())))
            .w(width)
            .min_w_0()
            .rounded(theme::radius_large(cx))
            .bg(theme::card(cx))
            .border_1()
            .border_color(theme::line(cx))
            .overflow_hidden()
            .child(body)
            .child(footer)
            .into_any_element()
    }
}
