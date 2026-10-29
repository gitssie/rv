//! Pick/preview an image, persist outbox intent, then kick the async sender.
use super::*;
use gpui_component::{WindowExt, dialog::DialogButtonProps};
use std::sync::Arc;
impl AddressBookApp {
    fn can_send_chat_image(&self, c: &Conversation) -> bool {
        !self.read_only
            && !self.chat.notifications_open
            && c.allow_reply == Some(true)
            && self
                .chat
                .accounts
                .iter()
                .any(|a| a.id == c.app_id && a.can_send)
            && strategy(&c.bundle).is_some_and(|plugin| plugin.capabilities().upload)
    }
    pub(super) fn pick_chat_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self
            .chat
            .selected()
            .cloned()
            .filter(|c| self.can_send_chat_image(c))
        else {
            return;
        };
        if self.chat.image_preparing {
            return;
        }
        self.chat.image_preparing = true;
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择聊天图片".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let path = paths[0].clone();
                    cx.background_executor()
                        .spawn(async move { rv_business::chat::images::prepare(&path).map(Some) })
                        .await
                }
                Ok(Ok(_)) => Ok(None),
                _ => Err("无法打开图片选择器".to_string()),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.chat.image_preparing = false;
                if !this.is_chat_window(window)
                    || this.chat.selected.as_ref() != Some(&c.id)
                    || !this.can_send_chat_image(&c)
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Some(image)) => this.preview_chat_image(c, image, window, cx),
                    Ok(None) => {}
                    Err(error) => this.chat_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn preview_chat_image(
        &mut self,
        c: Conversation,
        image: rv_business::chat::images::PreparedImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = Arc::new(Image::from_bytes(ImageFormat::Jpeg, image.bytes().to_vec()));
        let owner = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let owner = owner.clone();
            let image = image.clone();
            let c = c.clone();
            alert
                .title("发送图片")
                .description(format!("发送给 {}", c.title))
                .child(
                    gpui_component::h_flex().justify_center().child(
                        img(source.clone())
                            .w(px(240.))
                            .h(px(240.))
                            .object_fit(ObjectFit::Contain),
                    ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text("发送图片")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        if !this.is_chat_window(window)
                            || this.chat.selected.as_ref() != Some(&c.id)
                            || !this.can_send_chat_image(&c)
                        {
                            return true;
                        }
                        match rv_business::chat::images::queue(
                            this.book.paths(),
                            &c.app_id,
                            &c.id,
                            &image,
                        ) {
                            Ok(_) => {
                                this.reload_chat_thread(cx);
                                this.run_chat_sync(c.app_id.clone(),ChatSync::SendPending(c.id.clone()),false,cx);
                                true
                            }
                            Err(error) => {
                                this.chat_error(error, cx);
                                false
                            }
                        }
                    })
                })
        });
    }
}
