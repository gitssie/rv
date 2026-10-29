//! Native chat window reuses the existing controller and three-column view.
use super::*;
use gpui_component::{Root,TitleBar,TITLE_BAR_HEIGHT,v_flex};
use crate::theme;

struct ChatWindow {
    owner:Entity<AddressBookApp>,
    _updates:Subscription,
    _appearance:Subscription,
    controls:crate::window_controls::WindowControls,
    focus:FocusHandle,
}
impl ChatWindow {
    fn new(owner:Entity<AddressBookApp>,window:&mut Window,cx:&mut Context<Self>)->Self {
        let updates=cx.observe(&owner,|_,_,cx|cx.notify());
        let themed=owner.clone();
        let appearance=window.observe_window_appearance(move|window,cx| {
            let pref=themed.read(cx).book.prefs().theme;theme::apply(pref,window,cx);
        });
        let released=owner.clone();let id=window.window_handle().window_id();
        cx.on_release(move|_,cx|cx.defer(move|cx| {
            released.update(cx,|app,cx| {
                if app.chat.window.is_some_and(|handle|handle.window_id()==id) {
                    app.chat.window=None;app.chat.input_subscription=None;app.chat.image_preparing=false;cx.notify();
                }
            });
        })).detach();
        let focus=cx.focus_handle();focus.focus(window,cx);
        Self{owner,_updates:updates,_appearance:appearance,controls:Default::default(),focus}
    }
}
impl Render for ChatWindow {
    fn render(&mut self,window:&mut Window,cx:&mut Context<Self>)->impl IntoElement {
        self.controls.sync(window,theme::is_dark(cx));
        let height=f32::from((window.viewport_size().height-TITLE_BAR_HEIGHT).max(px(100.)));
        let body=self.owner.update(cx,|app,cx| {
            app.notify_status(window,cx);
            app.apply_chat_input_result(window,cx);
            app.render_chat(height,cx)
        });
        v_flex().id("chat-window").debug_selector(||"chat-window".into())
            .size_full().relative().bg(theme::surface(cx)).text_sm()
            .key_context("ChatWindow").track_focus(&self.focus)
            .on_action(|_:&crate::actions::ChatClose,window,_|window.remove_window())
            .on_action(|_:&crate::actions::QuitApp,_,cx|cx.quit())
            .child(TitleBar::new().child("消息"))
            .child(body)
            .children(Root::render_dialog_layer(window,cx))
            .children(Root::render_notification_layer(window,cx))
    }
}
impl AddressBookApp {
    pub(super) fn is_chat_window(&self,window:&Window)->bool {
        self.chat.window.is_some_and(|handle|handle.window_id()==window.window_handle().window_id())
    }
    pub(super) fn open_chat_window(&mut self,target:Option<AppId>,window:&mut Window,cx:&mut Context<Self>) {
        if self.chat.opening{return;}
        self.chat.opening=true;
        let viewport=window.viewport_size();
        // Match the former dialog's body size, excluding its horizontal padding.
        let content=size(px((f32::from(viewport.width)-32.).min(1180.)-32.),px((f32::from(viewport.height)-140.).clamp(100.,680.)));
        let bounds=Bounds::centered(None,size(content.width,content.height+TITLE_BAR_HEIGHT),cx);
        let existing=self.chat.window;
        cx.notify();
        cx.spawn(async move|this,cx| {
            let Some(owner)=this.upgrade()else{return};
            if let Some(handle)=existing {
                if handle.update(cx,|_,window,cx| {
                    owner.update(cx,|app,cx| {
                        app.chat.opening=false;
                        if let Some(id)=target.clone(){app.chat.notifications_open=false;app.filter_chat_account(Some(id),window,cx);}
                        app.reload_chat(cx);cx.notify();
                    });
                    window.activate_window();
                }).is_ok(){return;}
            }
            let result=cx.open_window({
                let mut options=TitleBar::window_options();
                if let Some(titlebar)=options.titlebar.as_mut(){titlebar.title=Some("消息 · RV".into());}
                options.window_bounds=Some(WindowBounds::Windowed(bounds));
                options.window_min_size=Some(size(content.width.min(px(720.)),content.height.min(px(420.))+TITLE_BAR_HEIGHT));
                options.app_id=Some("app.rv.viewer".into());options
            },move|window,cx| {
                owner.update(cx,|app,cx| {
                    app.chat.window=Some(window.window_handle().downcast::<Root>().expect("chat Root window"));
                    app.chat.opening=false;
                    app.chat.image_preparing=false;
                    theme::apply(app.book.prefs().theme,window,cx);
                    let text=app.chat.selected.as_ref().and_then(|id|draft(app.book.paths(),id).ok()).unwrap_or_default();
                    app.chat.input_subscription=None;
                    app.chat.composer=cx.new(|cx| {
                        let mut state=TextareaState::new(window,cx).auto_grow(1,3).submit_on_enter(true).placeholder("输入消息…");
                        state.set_value(text,window,cx);state
                    });
                    app.subscribe_chat(window,cx);
                    if let Some(id)=target {app.chat.notifications_open=false;app.filter_chat_account(Some(id),window,cx);}
                    cx.notify();
                });
                let view=cx.new(|cx|ChatWindow::new(owner,window,cx));
                cx.new(|cx|Root::new(view,window,cx))
            });
            if let Err(error)=result {
                let _=this.update(cx,|app,cx| {app.chat.opening=false;app.set_error_status(format!("打开消息窗口失败：{error}"),cx);});
            }
        }).detach();
    }
}
