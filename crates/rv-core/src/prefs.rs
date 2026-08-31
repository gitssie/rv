use serde::{Deserialize, Serialize};

use crate::ScaleMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub theme: ThemePref,
    #[serde(default)]
    pub hide_screenshots: bool,
    #[serde(default = "default_menu_key")]
    pub menu_key: String,
    #[serde(default)]
    pub default_scale: ScaleMode,
    #[serde(default = "default_true")]
    pub pin_toolbar: bool,
}

fn default_menu_key() -> String {
    "f8".into()
}

fn default_true() -> bool {
    true
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            hide_screenshots: false,
            menu_key: default_menu_key(),
            default_scale: ScaleMode::Fit,
            pin_toolbar: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }
}
