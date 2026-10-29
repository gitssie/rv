//! Passive, caption-sized expiry metadata. Tokens never enter UI state or text.
use super::*;
use crate::{assets::FunctionIcon, theme};
use chrono::{DateTime, Local};
use gpui_component::{Icon, Sizable, StyledExt, h_flex, tooltip::Tooltip};

fn expiry_text(name: &str, time: Option<i64>, now: i64) -> (String, String, bool) {
    let date = time
        .and_then(|v| DateTime::from_timestamp(v, 0))
        .map(|v| v.with_timezone(&Local));
    let expired = time.is_some_and(|v| v <= now);
    let label = date
        .map(|v| v.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into());
    let detail = date
        .map(|v| {
            format!(
                "{name} · {}{}",
                v.format("%Y-%m-%d %H:%M:%S %Z"),
                if expired { " · 已过期" } else { "" }
            )
        })
        .unwrap_or_else(|| format!("{name} · 未提供过期时间"));
    (label, detail, expired)
}
pub(super) fn token_indicator(
    id: SharedString,
    expiry: TokenExpirations,
    show_dates: bool,
    cx: &App,
) -> AnyElement {
    let now = super::super::unix_now();
    let (access, access_detail, access_expired) = expiry_text("Access Token", expiry.access, now);
    let (refresh, refresh_detail, refresh_expired) =
        expiry_text("Refresh Token", expiry.refresh, now);
    let tooltip = format!("{access_detail}\n{refresh_detail}\n到期时间不代表服务器已接受当前凭据");
    let icon_color = match expiry.access {
        Some(time) if time > now => theme::success(cx),
        Some(_) => theme::danger(cx),
        None => theme::muted(cx),
    };
    let mut indicator = h_flex()
        .id(id)
        .gap_1()
        .items_center()
        .flex_shrink_0()
        .text_xs()
        .font_normal()
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            Icon::new(FunctionIcon::Shield)
                .small()
                .text_color(icon_color),
        );
    if show_dates {
        indicator = indicator
            .child(
                crate::typography::caption(access, cx).text_color(if access_expired {
                    theme::danger(cx)
                } else {
                    theme::muted(cx)
                }),
            )
            .child(crate::typography::caption("|", cx))
            .child(
                crate::typography::caption(refresh, cx).text_color(if refresh_expired {
                    theme::danger(cx)
                } else {
                    theme::muted(cx)
                }),
            );
    }
    indicator.into_any_element()
}
impl AddressBookApp {
    pub(in crate::app) fn render_shop_token_expirations(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        token_indicator(
            "shop-token-expirations".into(),
            self.home.token_expirations,
            true,
            cx,
        )
    }
}
