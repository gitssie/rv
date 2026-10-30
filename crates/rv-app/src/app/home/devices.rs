use super::*;
use crate::{actions::*, theme};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Icon, IconName, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::DropdownMenu,
    v_flex,
};
use rv_core::Connection;
impl AddressBookApp {
    pub(in crate::app) fn render_device_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.home.device_search.read(cx).value().to_lowercase();
        let devices: Vec<Connection> = self
            .book
            .connections()
            .iter()
            .filter(|d| {
                (!self.home.online_only || self.phone_online(d.id, cx))
                    && (query.is_empty()
                        || format!("{} {} {}", d.name, d.host, d.model.as_deref().unwrap_or(""))
                            .to_lowercase()
                            .contains(&query))
            })
            .cloned()
            .collect();
        v_flex()
            .w(theme::sidebar_width())
            .h_full()
            .flex_shrink_0()
            .bg(theme::sidebar(cx))
            .border_r_1()
            .border_color(theme::line(cx))
            .child(
                h_flex()
                    .px_3()
                    .py_3()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(
                        Input::new(&self.home.device_search).prefix(Icon::new(IconName::Search)),
                    ))
                    .child(
                        Button::new("device-add")
                            .debug_selector(|| "management-new".into())
                            .ghost()
                            .icon(IconName::Plus)
                            .tooltip("新增设备")
                            .accessibility_label("新增设备")
                            .disabled(self.credential_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new(&NewConnection, window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .child(
                        Button::new("devices-all")
                            .ghost()
                            .icon(IconName::LayoutDashboard)
                            .accessibility_label("全部")
                            .child(div().text_xs().child("全部"))
                            .selected(!self.home.online_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.home.online_only = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("devices-online")
                            .ghost()
                            .icon(IconName::CircleCheck)
                            .accessibility_label("已连接")
                            .child(div().text_xs().child("已连接"))
                            .selected(self.home.online_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.home.online_only = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                uniform_list(
                    "device-list",
                    devices.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .filter_map(|i| devices.get(i))
                            .map(|device| this.render_device(device, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
    fn render_device(&self, device: &Connection, cx: &mut Context<Self>) -> AnyElement {
        let id = device.id;
        let online = self.phone_online(id, cx);
        let lock = if let Some(control) = self.home.controls.get(&id) {
            control.view.update(cx, |view, cx| {
                view.render_home_lock_button(self.read_only, cx)
            })
        } else {
            Button::new(SharedString::from(format!("device-lock-unavailable-{id}")))
                .ghost()
                .icon(crate::assets::AppActionIcon::Lock)
                .tooltip("设备未连接，锁屏状态未知")
                .accessibility_label("锁屏状态未知")
                .disabled(true)
                .into_any_element()
        };
        let selected = self.selected == Some(id);
        let current = device.current_bundle_id.as_deref().map(|bundle| {
            let name = self
                .home
                .apps
                .iter()
                .find(|a| a.bundle_id == bundle)
                .map(|a| self.home.plugins.for_bundle(bundle).identity(a).0);
            name.unwrap_or_else(|| bundle.to_owned())
        });
        let model = device.model.as_deref().unwrap_or("型号待读取");
        let os = device.os_version.as_deref().unwrap_or("系统待读取");
        h_flex()
            .id(ElementId::from(format!("device-row-{id}")))
            .debug_selector(move || format!("device-row-{id}"))
            .h(px(72.))
            .w_full()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme::line(cx))
            .bg(if selected {
                theme::selected(cx)
            } else {
                theme::sidebar(cx)
            })
            .hover(|s| s.bg(theme::hover(cx)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(id, window, cx);
                if event.click_count() >= 2 {
                    this.connect_id(id, cx);
                }
            }))
            .child(
                div()
                    .w(px(18.))
                    .h(px(30.))
                    .rounded(theme::radius_small(cx))
                    .border_1()
                    .border_color(theme::muted(cx))
                    .flex_shrink_0(),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_ellipsis()
                                    .font_semibold()
                                    .child(device.name.clone()),
                            )
                            .child(
                                div()
                                    .id(ElementId::from(format!("device-status-{id}")))
                                    .debug_selector(move || {
                                        format!(
                                            "device-status-{}-{id}",
                                            if online { "online" } else { "offline" }
                                        )
                                    })
                                    .size(px(7.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(if online {
                                        theme::success(cx)
                                    } else {
                                        theme::muted(cx)
                                    }),
                            )
                            .child(
                                div()
                                    .id(ElementId::from(format!("device-lock-{id}")))
                                    .debug_selector(move || format!("device-lock-{id}"))
                                    .child(lock),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .text_ellipsis()
                            .child(format!("{model} · {os}")),
                    )
                    .when_some(current, |el, current| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .text_ellipsis()
                                .child(current),
                        )
                    }),
            )
            .child(
                Button::new(SharedString::from(format!("device-menu-{id}")))
                    .ghost()
                    .size(px(22.))
                    .icon(IconName::Ellipsis)
                    .debug_selector(move || format!("connection-menu-{id}"))
                    .accessibility_label("设备更多操作")
                    .dropdown_menu(super::super::connections::menu::connection_menu(
                        cx.entity(),
                        id,
                    )),
            )
            .into_any_element()
    }
}
