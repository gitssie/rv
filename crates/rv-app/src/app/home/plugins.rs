//! UI plugins own business-specific rendering; the common card knows no Bundle IDs.
use crate::{assets::FunctionIcon, theme};
use gpui::*;
use gpui_component::{
    Icon, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    skeleton::Skeleton,
    v_flex,
};
use rv_core::workspace::{
    AppInstance,
    strategies::{AppStrategy, EnvironmentStrategy, VintedStrategy},
};
use std::collections::HashMap;

pub(in crate::app) trait AppRenderer {
    fn details(&self, app: &AppInstance, backups: usize, cx: &App) -> AnyElement;
    fn identity(&self, app: &AppInstance) -> (String, &'static str, u32, &'static str);
    fn header_info(&self, _app: &AppInstance, _cx: &App) -> AnyElement {
        div().into_any_element()
    }
    fn offline_operations(&self) -> &'static [rv_business::Operation] {
        &[]
    }
    fn datasets(&self) -> &'static [rv_business::data::Dataset] {
        &[]
    }
    fn card_preview(&self, row: Option<&rv_business::data::BusinessRow>, cx: &App) -> AnyElement {
        row.map(|row| self.business_row(row, cx))
            .unwrap_or_else(|| div().into_any_element())
    }
    fn detail_placeholder(&self, _cx: &App) -> AnyElement {
        div().into_any_element()
    }
    fn business_row(&self, row: &rv_business::data::BusinessRow, cx: &App) -> AnyElement {
        v_flex()
            .gap_1()
            .child(div().text_ellipsis().child(row.title.clone()))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child(row.summary.clone()),
            )
            .into_any_element()
    }
}
pub(in crate::app) struct RenderRegistry {
    plugins: HashMap<String, Box<dyn AppRenderer>>,
    fallback: Box<dyn AppRenderer>,
}
impl Default for RenderRegistry {
    fn default() -> Self {
        let mut plugins: HashMap<String, Box<dyn AppRenderer>> = HashMap::new();
        plugins.insert("lt.manodrabuziai.fr".into(), Box::new(VintedRenderer));
        Self {
            plugins,
            fallback: Box::new(EnvironmentRenderer),
        }
    }
}
impl RenderRegistry {
    pub fn for_bundle(&self, bundle: &str) -> &dyn AppRenderer {
        self.plugins
            .get(bundle)
            .map(|v| v.as_ref())
            .unwrap_or(self.fallback.as_ref())
    }
}
struct EnvironmentRenderer;
struct VintedRenderer;
fn fact(label: String, value: String, cx: &App) -> Div {
    h_flex()
        .gap_1p5()
        .min_w_0()
        .text_xs()
        .child(div().text_color(theme::muted(cx)).child(label))
        .child(div().text_ellipsis().child(value))
}
impl AppRenderer for EnvironmentRenderer {
    fn identity(&self, app: &AppInstance) -> (String, &'static str, u32, &'static str) {
        let p = EnvironmentStrategy.project(app, 0);
        (p.app_name, p.symbol, p.accent, p.action_label)
    }
    fn details(&self, app: &AppInstance, backups: usize, cx: &App) -> AnyElement {
        let p = EnvironmentStrategy.project(app, backups);
        h_flex()
            .gap_4()
            .children(p.facts.into_iter().map(|f| fact(f.label, f.value, cx)))
            .into_any_element()
    }
}
impl AppRenderer for VintedRenderer {
    fn header_info(&self, app: &AppInstance, cx: &App) -> AnyElement {
        let data: serde_json::Value = serde_json::from_str(&app.metadata).unwrap_or_default();
        let state = data["session_status"].as_str().unwrap_or("尚未检测");
        let market = data["market"]
            .as_str()
            .or_else(|| data["profile"]["market"].as_str())
            .unwrap_or("");
        h_flex()
            .gap_1()
            .text_xs()
            .text_color(theme::muted(cx))
            .child(market.to_owned())
            .child(div().size(px(5.)).rounded_full().bg(if state == "有效" {
                theme::success(cx)
            } else {
                theme::muted(cx)
            }))
            .child(state.to_owned())
            .into_any_element()
    }
    fn business_row(&self, row: &rv_business::data::BusinessRow, cx: &App) -> AnyElement {
        vinted_product(row, 64., cx)
    }
    fn detail_placeholder(&self, cx: &App) -> AnyElement {
        v_flex()
            .id("shop-detail-skeleton")
            .debug_selector(|| "shop-detail-skeleton".into())
            .children((0..4).map(|_| {
                div()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::line(cx))
                    .child(self.card_preview(None, cx))
            }))
            .into_any_element()
    }
    fn card_preview(&self, row: Option<&rv_business::data::BusinessRow>, cx: &App) -> AnyElement {
        if let Some(row) = row {
            return vinted_product(row, 72., cx);
        }
        h_flex()
            .id("shop-product-skeleton")
            .debug_selector(|| "shop-product-skeleton".into())
            .gap_3()
            .items_center()
            .py_1()
            .child(
                Skeleton::new()
                    .bg(theme::line(cx))
                    .w(px(72.))
                    .h(px(72.))
                    .flex_shrink_0()
                    .rounded(theme::radius(cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        Skeleton::new()
                            .bg(theme::line(cx))
                            .w(relative(0.85))
                            .h(px(14.))
                            .rounded(theme::radius_small(cx)),
                    )
                    .child(
                        Skeleton::new()
                            .bg(theme::line(cx))
                            .w(px(58.))
                            .h(px(12.))
                            .rounded(theme::radius_small(cx)),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Skeleton::new()
                                    .bg(theme::line(cx))
                                    .secondary()
                                    .w(px(44.))
                                    .h(px(20.))
                                    .rounded(theme::radius_small(cx)),
                            )
                            .child(
                                Skeleton::new()
                                    .bg(theme::line(cx))
                                    .secondary()
                                    .w(px(60.))
                                    .h(px(20.))
                                    .rounded(theme::radius_small(cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
    fn datasets(&self) -> &'static [rv_business::data::Dataset] {
        &[
            rv_business::data::Dataset::Products,
            rv_business::data::Dataset::Orders,
        ]
    }
    fn offline_operations(&self) -> &'static [rv_business::Operation] {
        &[
            rv_business::Operation::Session,
            rv_business::Operation::Inbox,
            rv_business::Operation::NotificationStats,
            rv_business::Operation::Refresh,
        ]
    }
    fn identity(&self, app: &AppInstance) -> (String, &'static str, u32, &'static str) {
        let p = VintedStrategy.project(app, 0);
        (p.app_name, p.symbol, p.accent, p.action_label)
    }
    fn details(&self, app: &AppInstance, backups: usize, cx: &App) -> AnyElement {
        vinted_status(app, backups, cx)
    }
}
fn vinted_product(row: &rv_business::data::BusinessRow, size: f32, cx: &App) -> AnyElement {
    let mut photo = div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(theme::radius(cx))
        .overflow_hidden()
        .bg(theme::selected(cx));
    if let Some(path) = &row.visual.image_path {
        photo = photo.child(img(path.clone()).size_full().object_fit(ObjectFit::Cover));
    }
    h_flex()
        .gap_3()
        .items_center()
        .py_1()
        .child(photo)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_normal()
                        .text_ellipsis()
                        .child(row.title.clone()),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(div().text_xs().mr_1().child(row.visual.price.clone()))
                        .children(row.visual.badges.iter().map(|label| {
                            let tag = match label.as_str() {
                                "隐藏" | "不可交易" => gpui_component::tag::Tag::danger(),
                                "延迟发布" | "处理中" | "待发货" => {
                                    gpui_component::tag::Tag::warning()
                                }
                                "预留" | "已完成" => gpui_component::tag::Tag::success(),
                                "已关闭" => gpui_component::tag::Tag::secondary(),
                                _ => gpui_component::tag::Tag::info(),
                            };
                            tag.child(label.clone())
                        })),
                ),
        )
        .into_any_element()
}
fn vinted_status(app: &AppInstance, backups: usize, cx: &App) -> AnyElement {
    let details = VintedStrategy.project(app, backups);
    let state = details.facts[0].value.clone();
    let unread = details.facts[1].value.clone();
    let color = if state == "有效" {
        theme::success(cx)
    } else if state == "尚未检测" {
        theme::muted(cx)
    } else {
        theme::danger(cx)
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(
            Button::new(format!("card-session-{}", app.id.as_str()))
                .debug_selector({
                    let id = app.id.clone();
                    move || format!("card-session-{}", id.as_str())
                })
                .ghost()
                .icon(Icon::new(FunctionIcon::Shield))
                .text_color(color)
                .tooltip(format!("会话：{state}"))
                .accessibility_label(format!("会话：{state}")),
        )
        .child(
            Button::new(format!("card-unread-{}", app.id.as_str()))
                .ghost()
                .icon(Icon::new(FunctionIcon::Chat))
                .label(unread)
                .text_color(theme::muted(cx))
                .tooltip("未读消息")
                .accessibility_label("未读消息"),
        )
        .into_any_element()
}
