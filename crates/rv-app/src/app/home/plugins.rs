//! UI plugins own business-specific rendering; the common card knows no Bundle IDs.
use crate::{assets::FunctionIcon, theme};
use gpui::*;
use gpui_component::{Icon, IconName, Sizable, StyledExt, h_flex, tooltip::Tooltip, v_flex};
use rv_core::workspace::{
    AppInstance,
    strategies::{AppStrategy, EnvironmentStrategy, VintedStrategy},
};
use std::collections::HashMap;

pub(in crate::app) trait AppRenderer {
    fn details(
        &self,
        app: &AppInstance,
        backups: usize,
        expiry: rv_core::workspace::TokenExpirations,
        cx: &App,
    ) -> AnyElement;
    fn identity(&self, app: &AppInstance) -> (String, &'static str, u32, &'static str);
    fn display_title(&self, app: &AppInstance) -> String {
        app.title.clone()
    }
    fn avatar_path(
        &self,
        _app: &AppInstance,
        _paths: &rv_core::StorePaths,
    ) -> Option<std::path::PathBuf> {
        None
    }

    fn profile_details(
        &self,
        _app: &AppInstance,
        _paths: &rv_core::StorePaths,
        _cx: &App,
    ) -> AnyElement {
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
    fn details(
        &self,
        app: &AppInstance,
        backups: usize,
        expiry: rv_core::workspace::TokenExpirations,
        cx: &App,
    ) -> AnyElement {
        let _ = expiry;
        let p = EnvironmentStrategy.project(app, backups);
        h_flex()
            .gap_4()
            .children(p.facts.into_iter().map(|f| fact(f.label, f.value, cx)))
            .into_any_element()
    }
}
impl AppRenderer for VintedRenderer {
    fn display_title(&self, app: &AppInstance) -> String {
        rv_business::shop::ShopProfile::from_app(app)
            .and_then(|p| p.login)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "Vinted".into())
    }
    fn avatar_path(
        &self,
        app: &AppInstance,
        paths: &rv_core::StorePaths,
    ) -> Option<std::path::PathBuf> {
        rv_business::shop::ShopProfile::from_app(app)?
            .avatar_url
            .as_deref()
            .and_then(|url| rv_business::assets::cached(paths, url))
    }
    fn profile_details(
        &self,
        app: &AppInstance,
        _paths: &rv_core::StorePaths,
        _cx: &App,
    ) -> AnyElement {
        let Some(profile) = rv_business::shop::ShopProfile::from_app(app) else {
            return div().into_any_element();
        };
        let country = profile
            .country_code
            .or(profile.country_title)
            .unwrap_or_else(|| "—".into());
        let score = profile
            .feedback_reputation
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into());
        let feedback = profile
            .feedback_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into());
        let items = profile
            .item_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into());
        h_flex()
            .gap_2()
            .flex_wrap()
            .items_center()
            .child(metric(
                "shop-country".into(),
                gpui_component::IconName::Globe.into(),
                country,
                "国家",
                _cx,
            ))
            .child(metric(
                "shop-score".into(),
                gpui_component::IconName::Star.into(),
                score,
                "评分",
                _cx,
            ))
            .child(metric(
                "shop-feedback".into(),
                FunctionIcon::Chat.into(),
                feedback,
                "评价数量",
                _cx,
            ))
            .child(metric(
                "shop-items".into(),
                FunctionIcon::Products.into(),
                items,
                "商品数量",
                _cx,
            ))
            .into_any_element()
    }

    fn business_row(&self, row: &rv_business::data::BusinessRow, cx: &App) -> AnyElement {
        vinted_product(row, 64., false, cx)
    }
    fn card_preview(&self, row: Option<&rv_business::data::BusinessRow>, cx: &App) -> AnyElement {
        if let Some(row) = row {
            return vinted_product(row, 72., true, cx);
        }
        div().into_any_element()
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
        ]
    }
    fn identity(&self, app: &AppInstance) -> (String, &'static str, u32, &'static str) {
        let p = VintedStrategy.project(app, 0);
        (p.app_name, p.symbol, p.accent, p.action_label)
    }
    fn details(
        &self,
        app: &AppInstance,
        backups: usize,
        expiry: rv_core::workspace::TokenExpirations,
        cx: &App,
    ) -> AnyElement {
        let _ = backups;
        vinted_status(app, expiry, cx)
    }
}
fn vinted_product(row: &rv_business::data::BusinessRow, size: f32, compact: bool, cx: &App) -> AnyElement {
    let photo:AnyElement=if let Some(path)=&row.visual.image_path {
        img(path.clone())
            .size(px(size))
            .flex_shrink_0()
            .object_fit(ObjectFit::Cover)
            .rounded(theme::radius_large(cx))
            .into_any_element()
    } else {
        div().size(px(size)).flex_shrink_0()
            .rounded(theme::radius_large(cx))
            .bg(theme::selected(cx))
            .flex().items_center().justify_center()
            .child(Icon::new(FunctionIcon::Products).text_color(theme::muted(cx)))
            .into_any_element()
    };
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
                            let (icon,color)=product_status(label,cx);
                            let tooltip=label.clone();
                            let mut status=h_flex()
                                .id(SharedString::from(format!("product-status-{}-{label}",row.id)))
                                .items_center()
                                .gap_1()
                                .text_color(color)
                                .tooltip(move |window,cx|Tooltip::new(tooltip.clone()).build(window,cx))
                                .child(icon.small());
                            if !compact {status=status.child(div().text_xs().child(label.clone()));}
                            status
                        })),
                ),
        )
        .into_any_element()
}
fn product_status(label:&str,cx:&App)->(Icon,Hsla) {
    let icon=match label {
        "隐藏"=>Icon::new(IconName::EyeOff),
        "不可交易"=>Icon::new(IconName::CircleX),
        "延迟发布"=>Icon::new(FunctionIcon::Recent),
        "草稿"=>Icon::new(FunctionIcon::Edit),
        "预留"=>Icon::new(IconName::Pause),
        "处理中"=>Icon::new(IconName::LoaderCircle),
        "已关闭"=>Icon::new(FunctionIcon::Archive),
        "已完成"=>Icon::new(IconName::CircleCheck),
        "待发货"=>Icon::new(FunctionIcon::Products),
        "卖出"=>Icon::new(IconName::ArrowUp),
        "买入"=>Icon::new(IconName::ArrowDown),
        _=>Icon::new(IconName::Info),
    };
    let color=match label {
        "隐藏"=>theme::muted(cx),
        "不可交易"=>theme::warning(cx),
        "延迟发布"|"处理中"|"待发货"=>theme::warning(cx),
        "预留"|"已完成"=>theme::success(cx),
        "已关闭"|"草稿"=>theme::muted(cx),
        _=>theme::accent(cx),
    };
    (icon,color)
}
fn metric(
    id: SharedString,
    icon: Icon,
    value: String,
    tooltip: &'static str,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(id)
        .gap_1()
        .items_center()
        .text_xs()
        .text_color(theme::muted(cx))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(icon)
        .child(crate::typography::caption(value, cx))
        .into_any_element()
}
fn vinted_status(
    app: &AppInstance,
    expiry: rv_core::workspace::TokenExpirations,
    cx: &App,
) -> AnyElement {
    let details = VintedStrategy.project(app, 0);
    h_flex()
        .gap_2()
        .items_center()
        .child(super::token_expiry::token_indicator(
            format!("card-token-{}", app.id.as_str()).into(),
            expiry,
            false,
            cx,
        ))
        .child(metric(
            format!("card-unread-{}", app.id.as_str()).into(),
            FunctionIcon::Chat.into(),
            details.facts[1].value.clone(),
            "未读消息",
            cx,
        ))
        .into_any_element()
}
