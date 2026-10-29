//! Device-owned HTTP control, independent of remote desktop windows.
use super::*;
use crate::session_window::{SessionOptions, SessionView};
use rv_core::{ConnectRequest, Connection};

pub(in crate::app) struct PhoneControl {
    pub view: Entity<SessionView>,
    endpoint: (String, u16, bool, bool),
    _updates: Option<Subscription>,
}
impl PhoneControl {
    pub(in crate::app) fn new(view: Entity<SessionView>, connection: &Connection) -> Self {
        Self {
            view,
            endpoint: endpoint(connection),
            _updates: None,
        }
    }
}
fn endpoint(connection: &Connection) -> (String, u16, bool, bool) {
    (
        connection.host.clone(),
        connection.zxtouch_port.unwrap_or(6000),
        connection.view_only,
        connection.remember_password,
    )
}
impl AddressBookApp {
    pub(in crate::app) fn sync_phone_unlock_dialogs(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for control in self.home.controls.values() {
            control
                .view
                .update(cx, |view, cx| view.sync_unlock_dialog(window, cx));
        }
    }
    pub(in crate::app) fn sync_phone_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let devices = self.book.connections().to_vec();
        self.home.controls.retain(|id, control| {
            devices
                .iter()
                .any(|d| d.id == *id && endpoint(d) == control.endpoint)
        });
        self.home
            .control_loading
            .retain(|id, _| devices.iter().any(|d| d.id == *id));
        for device in devices {
            if self.home.controls.contains_key(&device.id)
                || self.home.control_loading.contains_key(&device.id)
            {
                continue;
            }
            self.home.control_revision += 1;
            let revision = self.home.control_revision;
            let id = device.id;
            self.home.control_loading.insert(id, revision);
            let paths = self.book.paths().clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        if device.remember_password {
                            paths.load_password(id)
                        } else {
                            Ok(None)
                        }
                    })
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if this.home.control_loading.get(&id) != Some(&revision) {
                        return;
                    }
                    this.home.control_loading.remove(&id);
                    let Some(current) = this.book.get(id).cloned() else {
                        return;
                    };
                    if endpoint(&current) != endpoint(&device) {
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(password) => {
                            let request = ConnectRequest::from_connection(&current, password);
                            this.start_phone_control(&current, request, window, cx);
                        }
                        Err(error) => {
                            // Keep a failed credential read pending until the record is edited;
                            // rendering must not repeatedly hit a damaged local password store.
                            this.home.control_loading.insert(id, revision);
                            this.set_status(format!("读取 ZXTouch 密码失败：{error}"), cx);
                        }
                    }
                });
            })
            .detach();
        }
    }
    pub(in crate::app) fn start_phone_control(
        &mut self,
        device: &Connection,
        request: ConnectRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefs = self.book.prefs();
        let options = SessionOptions {
            scale: prefs.default_scale,
            pin_toolbar: false,
            menu_key: prefs.menu_key.clone(),
            hide_shots: true,
            thumb_path: None,
            app_icon_cache: Some(self.book.paths().clone()),
            address_book: Some(cx.entity().downgrade()),
            transfer_folders: Default::default(),
            app_shortcuts: device.app_shortcuts.clone(),
            remember_password: device.remember_password,
        };
        self.home.control_loading.remove(&device.id);
        let view = cx.new(|cx| {
            SessionView::new_control(
                request,
                options,
                device.environment_apps.clone(),
                window,
                cx,
            )
        });
        let bundles = self
            .home
            .apps
            .iter()
            .map(|app| app.bundle_id.clone())
            .collect();
        view.update(cx, |view, cx| view.request_home_app_icons(bundles, cx));
        let updates = cx.observe(&view, |_, _, cx| cx.notify());
        let mut control = PhoneControl::new(view, device);
        control._updates = Some(updates);
        self.home.controls.insert(device.id, control);
        cx.notify();
    }
}
