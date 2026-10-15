//! Saved-password presentation without putting a fake password in the editor.
use super::super::{AddressBookApp, Modal};
use gpui::{Context, Window};

impl AddressBookApp {
    pub(in crate::app) fn refresh_password_hint(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.password_hint_task = None;
        let Some(id) = self.editing else {
            return;
        };
        let paths = self.book.paths().clone();
        self.password_hint_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .load_password(id)
                        .map(|password| password.is_some_and(|password| !password.is_empty()))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.editing != Some(id) || !matches!(this.modal, Modal::Connection) {
                    return;
                }
                match result {
                    Ok(saved) => this.password_input.update(cx, |input, cx| {
                        input.set_placeholder(
                            if saved {
                                "********"
                            } else {
                                "密码（可选）"
                            },
                            window,
                            cx,
                        );
                    }),
                    Err(error) => {
                        this.connection_error = Some(format!("读取已保存密码失败：{error}").into());
                    }
                }
                cx.notify();
            });
        }));
    }
}
