//! Shop avatar chooser and preview. Confirmation starts an immediate async operation.
use super::*;
use gpui_component::{
    Disableable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
};
use std::sync::Arc;
impl AddressBookApp {
    pub(super) fn render_set_avatar_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = self.home.editing.as_ref() else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        Button::new("shop-set-avatar")
            .debug_selector(|| "shop-set-avatar".into())
            .ghost()
            .icon(crate::assets::AppActionIcon::Camera)
            .tooltip("设置头像")
            .accessibility_label("设置头像")
            .loading(self.home.avatar_preparing.contains(&id)||self.home.avatar_setting.contains(&id))
            .disabled(
                self.read_only
                    || app.archived
                    || self.home.avatar_preparing.contains(&id)
                    || self.home.avatar_setting.contains(&id),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.pick_shop_avatar(id.clone(), window, cx)
                }),
            )
            .into_any_element()
    }
    fn avatar_target_is_open(&self, id: &AppId) -> bool {
        matches!(self.modal, Modal::Shop)
            && self.home.editing.as_ref().is_some_and(|app| app.id == *id)
    }
    fn pick_shop_avatar(&mut self, id: AppId, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only
            || !self.home.avatar_preparing.insert(id.clone())
        {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择店铺头像".into()),
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let path = paths[0].clone();
                    cx.background_executor()
                        .spawn(async move { rv_business::avatar::prepare(&path).map(Some) })
                        .await
                }
                Ok(Ok(_)) => Ok(None),
                _ => Err("无法打开图片选择器".to_string()),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.home.avatar_preparing.remove(&id);
                if !this.avatar_target_is_open(&id) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Some(image)) => this.preview_shop_avatar(id.clone(), image, window, cx),
                    Ok(None) => {}
                    Err(error) => this.set_error_status(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn preview_shop_avatar(
        &mut self,
        id: AppId,
        image: rv_business::avatar::PreparedAvatar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preview = Arc::new(Image::from_bytes(ImageFormat::Jpeg, image.bytes().to_vec()));
        let title = self
            .home
            .editing
            .as_ref()
            .map(|app| {
                self.home
                    .plugins
                    .for_bundle(&app.bundle_id)
                    .display_title(app)
            })
            .unwrap_or_default();
        let owner = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let id = id.clone();
            let image = image.clone();
            let owner = owner.clone();
            alert
                .title("设置店铺头像")
                .description(format!(
                    "将这张图片设为“{title}”的头像。图片已居中裁切为正方形。"
                ))
                .child(
                    gpui_component::h_flex().justify_center().child(
                        img(preview.clone())
                            .size(px(160.))
                            .object_fit(ObjectFit::Cover),
                    ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text("设置头像")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, _, cx| {
                    owner.update(cx, |this, cx| {
                        if this.read_only || !this.avatar_target_is_open(&id) {
                            return true;
                        }
                        this.set_shop_avatar(id.clone(),image.clone(),cx);
                        true
                    })
                })
        });
    }
    fn set_shop_avatar(&mut self,id:AppId,image:rv_business::avatar::PreparedAvatar,cx:&mut Context<Self>) {
        if self.read_only || !self.home.avatar_setting.insert(id.clone()){return;}
        self.home.jobs.insert(id.clone());let paths=self.book.paths().clone();cx.notify();
        cx.spawn(async move |this,cx| {
            let result=rv_business::avatar::set(paths,id.clone(),image).await;
            let _=this.update(cx,|this,cx| {
                this.home.avatar_setting.remove(&id);
                if !this.home.manual_syncs.iter().any(|(app,_)|app==&id){this.home.jobs.remove(&id);}
                match result {
                    Ok(())=>this.set_status("店铺头像已更新",cx),
                    Err(error)=>{tracing::error!(app_id=%id.as_str(),%error,"Avatar update failed");this.set_error_status(error,cx);},
                }
                cx.notify();
            });
        }).detach();
    }

}
