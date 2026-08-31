//! RealVNC-inspired palette (original colors, not trademarked assets).

use gpui::{Pixels, Rgba, px, rgb};

pub fn accent() -> Rgba {
    rgb(0x0B6BCB)
}

pub fn surface() -> Rgba {
    rgb(0xF4F6F9)
}

pub fn card() -> Rgba {
    rgb(0xFFFFFF)
}

pub fn ink() -> Rgba {
    rgb(0x1B2430)
}

pub fn muted() -> Rgba {
    rgb(0x5C6773)
}

pub fn line() -> Rgba {
    rgb(0xE2E6EA)
}

pub fn danger() -> Rgba {
    rgb(0xC0362C)
}

pub fn desktop() -> Rgba {
    rgb(0x111418)
}

pub fn toolbar() -> Rgba {
    rgb(0x1E252E)
}

pub fn toolbar_fg() -> Rgba {
    rgb(0xE8EEF4)
}

pub fn sidebar() -> Rgba {
    rgb(0xEEF2F6)
}

pub fn selected() -> Rgba {
    rgb(0xE6F1FB)
}

pub fn thumb_bg() -> Rgba {
    rgb(0xD9DEE5)
}

pub fn sidebar_width() -> Pixels {
    px(220.)
}

pub fn titlebar_height() -> Pixels {
    px(42.)
}

pub fn toolbar_height() -> Pixels {
    px(40.)
}
