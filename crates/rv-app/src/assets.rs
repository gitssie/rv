use gpui::{AssetSource, Result, SharedString};
use gpui_component::IconNamed;
use std::borrow::Cow;

const ICONS: [(&str, &[u8]); 13] = [
    (
        "rv-icons/home.svg",
        include_bytes!("../../../assets/icons/home.svg"),
    ),
    (
        "rv-icons/camera.svg",
        include_bytes!("../../../assets/icons/camera.svg"),
    ),
    (
        "rv-icons/lock-keyhole.svg",
        include_bytes!("../../../assets/icons/lock-keyhole.svg"),
    ),
    (
        "rv-icons/lock-keyhole-open.svg",
        include_bytes!("../../../assets/icons/lock-keyhole-open.svg"),
    ),
    (
        "rv-icons/rotate-cw.svg",
        include_bytes!("../../../assets/icons/rotate-cw.svg"),
    ),
    (
        "rv-icons/power.svg",
        include_bytes!("../../../assets/icons/power.svg"),
    ),
    (
        "rv-icons/pin-off.svg",
        include_bytes!("../../../assets/icons/pin-off.svg"),
    ),
    (
        "rv-icons/log-out.svg",
        include_bytes!("../../../assets/icons/log-out.svg"),
    ),
    (
        "rv-icons/mail.svg",
        include_bytes!("../../../assets/icons/mail.svg"),
    ),
    (
        "rv-icons/shield.svg",
        include_bytes!("../../../assets/icons/shield.svg"),
    ),
    (
        "rv-icons/monitor.svg",
        include_bytes!("../../../assets/icons/monitor.svg"),
    ),
    (
        "rv-icons/wrench.svg",
        include_bytes!("../../../assets/icons/wrench.svg"),
    ),
    (
        "rv-icons/cloud.svg",
        include_bytes!("../../../assets/icons/cloud.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(*bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = gpui_component_assets::Assets.list(path)?;
        assets.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| (*name).into()),
        );
        Ok(assets)
    }
}

pub enum AppActionIcon {
    Home,
    Camera,
    Lock,
    Unlock,
    Restart,
    Terminate,
    RemoveShortcut,
}

impl IconNamed for AppActionIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Home => "rv-icons/home.svg",
            Self::Camera => "rv-icons/camera.svg",
            Self::Lock => "rv-icons/lock-keyhole.svg",
            Self::Unlock => "rv-icons/lock-keyhole-open.svg",
            Self::Restart => "rv-icons/rotate-cw.svg",
            Self::Terminate => "rv-icons/power.svg",
            Self::RemoveShortcut => "rv-icons/pin-off.svg",
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_and_component_icons_are_available() {
        for icon in [
            AppActionIcon::Lock,
            AppActionIcon::Unlock,
            AppActionIcon::Restart,
            AppActionIcon::Terminate,
            AppActionIcon::RemoveShortcut,
        ] {
            let path = icon.path();
            assert!(Assets.load(&path).unwrap().unwrap().starts_with(b"<svg"));
            assert!(Assets.list("rv-icons/").unwrap().contains(&path));
        }
        for icon in [
            FunctionIcon::Wrench,
            FunctionIcon::Cloud,
            FunctionIcon::Mail,
            FunctionIcon::LogOut,
            FunctionIcon::Shield,
            FunctionIcon::Monitor,
        ] {
            let path = icon.path();
            assert!(Assets.load(&path).unwrap().unwrap().starts_with(b"<svg"));
            assert!(Assets.list("rv-icons/").unwrap().contains(&path));
        }
        assert!(Assets.load("icons/plus.svg").unwrap().is_some());
    }
}

#[derive(Clone)]
pub enum FunctionIcon {
    User,
    LogOut,
    Mail,
    Shield,
    Monitor,
    Wrench,
    Cloud,
    Folder,
    Play,
    Search,
}
impl IconNamed for FunctionIcon {
    fn path(self) -> SharedString {
        match self {
            Self::User => "icons/user.svg",
            Self::LogOut => "rv-icons/log-out.svg",
            Self::Mail => "rv-icons/mail.svg",
            Self::Shield => "rv-icons/shield.svg",
            Self::Monitor => "rv-icons/monitor.svg",
            Self::Wrench => "rv-icons/wrench.svg",
            Self::Cloud => "rv-icons/cloud.svg",
            Self::Folder => "icons/folder.svg",
            Self::Play => "icons/play.svg",
            Self::Search => "icons/search.svg",
        }
        .into()
    }
}
