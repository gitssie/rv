//! RV theme policy. Component defaults own control appearance and interaction states.
use gpui::{App, Hsla, Pixels, Window, hsla, px, rgb};
use gpui_component::{ActiveTheme as _, Theme, ThemeMode};
use rv_core::ThemePref;

// text_sm is 0.875rem; text_xs is 0.75rem. Set this once for every Root.
const BODY_FONT_PX: f32 = 13.;
const UI_REM_PX: f32 = BODY_FONT_PX / 0.875;

pub fn surface(cx: &App) -> Hsla {
    cx.theme().background
}
pub fn card(cx: &App) -> Hsla {
    cx.theme().popover
}
pub fn ink(cx: &App) -> Hsla {
    cx.theme().foreground
}
pub fn muted(cx: &App) -> Hsla {
    cx.theme().muted_foreground
}
pub fn line(cx: &App) -> Hsla {
    cx.theme().border
}
pub fn sidebar(cx: &App) -> Hsla {
    cx.theme().sidebar
}
pub fn selected(cx: &App) -> Hsla {
    cx.theme().list_active
}
pub fn hover(cx: &App) -> Hsla {
    cx.theme().list_hover
}
pub fn accent(cx: &App) -> Hsla {
    cx.theme().primary
}
pub fn accent_fg(cx: &App) -> Hsla {
    cx.theme().primary_foreground
}
pub fn success(cx: &App) -> Hsla {
    cx.theme().success
}
pub fn danger(cx: &App) -> Hsla {
    cx.theme().danger
}
pub fn warning(cx: &App) -> Hsla {
    cx.theme().warning
}
pub fn radius(cx: &App) -> Pixels {
    cx.theme().radius
}
pub fn radius_small(cx: &App) -> Pixels {
    cx.theme().radius / 2.
}
pub fn radius_large(cx: &App) -> Pixels {
    cx.theme().radius_lg
}
pub fn is_dark(cx: &App) -> bool {
    cx.theme().mode.is_dark()
}
pub fn row_background(selected: bool, index: usize, cx: &App) -> Hsla {
    if selected {
        cx.theme().list_active
    } else if index % 2 == 1 {
        cx.theme().list_even
    } else {
        cx.theme().colors.list
    }
}

// Brand identity is data, distinct from application interaction/status colors.
pub fn brand_color(value: u32) -> Hsla {
    rgb(value).into()
}

// The remote framebuffer alone keeps a neutral dark surround in either theme.
pub fn desktop() -> Hsla {
    rgb(0x111418).into()
}
pub fn scrim() -> Hsla {
    hsla(0., 0., 0., 0.55)
}
pub fn tool_panel_padding() -> Pixels {
    px(8.)
}
pub fn control_window_size() -> gpui::Size<Pixels> {
    gpui::size(px(640.), px(800.))
}
pub fn sidebar_width() -> Pixels {
    px(244.)
}
pub fn toolbar_height() -> Pixels {
    px(40.)
}

pub fn apply(pref: ThemePref, window: &mut Window, cx: &mut App) {
    let mode = match pref {
        ThemePref::System => ThemeMode::from(window.appearance()),
        ThemePref::Light => ThemeMode::Light,
        ThemePref::Dark => ThemeMode::Dark,
    };
    // Reapply the complete library palette; never leave a prior mode's overrides behind.
    Theme::change(mode, Some(window), cx);
    configure(Theme::global_mut(cx));
    Theme::sync_base(cx);
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, _| window.refresh());
    }
}

fn configure(theme: &mut Theme) {
    theme.font_size = px(UI_REM_PX);
    theme.mono_font_size = px(12.);
    // RV's only control color override. Keep legacy and resolved tokens aligned.
    let palette = &mut theme.colors;
    palette.primary = rgb(0x3389f5).into();
    palette.primary_foreground = rgb(0xffffff).into();
    palette.primary_hover = rgb(0x4598ff).into();
    palette.primary_active = rgb(0x2273d5).into();
    palette.ring = palette.primary;
    palette.button_primary = palette.primary;
    palette.button_primary_foreground = palette.primary_foreground;
    palette.button_primary_hover = palette.primary_hover;
    palette.button_primary_active = palette.primary_active;
    let tokens = &mut theme.tokens;
    tokens.primary = palette.primary.into();
    tokens.primary_foreground = palette.primary_foreground.into();
    tokens.primary_hover = palette.primary_hover.into();
    tokens.primary_active = palette.primary_active.into();
    tokens.ring = palette.ring.into();
    tokens.button_primary = palette.button_primary.into();
    tokens.button_primary_foreground = palette.button_primary_foreground.into();
    tokens.button_primary_hover = palette.button_primary_hover.into();
    tokens.button_primary_active = palette.button_primary_active.into();
}

#[cfg(test)]
mod tests {
    use super::{UI_REM_PX, configure};
    use gpui::{Hsla, px};
    use gpui_component::{Theme, ThemeMode};

    #[test]
    fn rv_theme_keeps_resolved_colors_and_component_density_in_sync() {
        let mut theme = Theme::default();
        for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::Light] {
            theme.mode = mode;
            let config = if mode.is_dark() {
                theme.dark_theme.clone()
            } else {
                theme.light_theme.clone()
            };
            theme.apply_config(&config);
            let background = theme.background;
            let radius = theme.radius;
            let family = theme.font_family.clone();
            configure(&mut theme);
            assert_eq!(theme.font_size, px(UI_REM_PX));
            assert_eq!(theme.mono_font_size, px(12.));
            assert_eq!(theme.background, background);
            assert_eq!(theme.radius, radius);
            assert_eq!(theme.font_family, family);
            assert_eq!(Hsla::from(theme.tokens.background), theme.background);
            assert_eq!(Hsla::from(theme.tokens.primary), theme.primary);
            assert_eq!(
                Hsla::from(theme.tokens.primary_foreground),
                theme.primary_foreground
            );
            assert_eq!(
                Hsla::from(theme.tokens.button_primary),
                theme.button_primary
            );
            assert_eq!(
                Hsla::from(theme.tokens.button_primary_hover),
                theme.button_primary_hover
            );
            assert_eq!(
                Hsla::from(theme.tokens.button_primary_active),
                theme.button_primary_active
            );
            assert_eq!(Hsla::from(theme.tokens.ring), theme.ring);
            assert_eq!(theme.button_primary_foreground, theme.primary_foreground);
        }
    }
}
