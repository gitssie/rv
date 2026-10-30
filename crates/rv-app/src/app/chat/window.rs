//! Native chat window reuses the existing controller and three-column view.
use super::*;
use gpui_component::{Root,TitleBar,TITLE_BAR_HEIGHT,v_flex,Sizable};
use crate::theme;
use crate::assets::FunctionIcon;
use gpui_component::dock::{BasePanel, Panel, PanelEvent, PanelControl, DockArea, DockLayout, panel_handle};

#[derive(Clone, Copy)]
enum ChatPane { Stores, Conversations, Messages }
struct ChatDockPanel {
    owner: WeakEntity<AddressBookApp>,
    pane: ChatPane,
    focus: FocusHandle,
    _updates: Subscription,
}
impl EventEmitter<PanelEvent> for ChatDockPanel {}
impl Focusable for ChatDockPanel {
    fn focus_handle(&self, _: &App)->FocusHandle { self.focus.clone() }
}
impl BasePanel for ChatDockPanel {
    fn panel_name(&self)->&'static str { match self.pane { ChatPane::Stores=>"ChatStores",ChatPane::Conversations=>"ChatConversations",ChatPane::Messages=>"ChatMessages" } }
    fn closable(&self,_:&App)->bool { false }
}
impl Panel for ChatDockPanel {
    fn title(&mut self,_:&mut Window,_:&mut Context<Self>)->impl IntoElement {
        div()
    }
    fn tab_name(&self,_:&App)->Option<SharedString> {
        Some(match self.pane { ChatPane::Stores=>"店铺",ChatPane::Conversations=>"会话 / 通知",ChatPane::Messages=>"消息" }.into())
    }
    fn inner_padding(&self,_:&App)->bool { false }
    fn zoom_control(&self,_:&App)->Option<PanelControl> { None }
}
impl Render for ChatDockPanel {
    fn render(&mut self,_:&mut Window,cx:&mut Context<Self>)->impl IntoElement {
        let pane=self.pane;
        let content=self.owner.update(cx,|app,cx| match pane {
            ChatPane::Stores=>app.render_chat_stores(cx),
            ChatPane::Conversations=>app.render_chat_conversations(cx),
            ChatPane::Messages=>app.render_chat_content(cx),
        }).unwrap_or_else(|_|div().into_any_element());
        let panel=v_flex().id(self.panel_name()).role(Role::Group)
            .aria_label(match pane {ChatPane::Stores=>"店铺列表",ChatPane::Conversations=>"会话与通知",ChatPane::Messages=>"消息内容"})
            .track_focus(&self.focus).size_full().min_w_0().min_h_0().overflow_hidden();
        let panel=match pane {
            ChatPane::Stores=>panel.p_2(),
            ChatPane::Conversations|ChatPane::Messages=>panel,
        };
        panel.child(content)
    }
}

struct ChatWindow {
    dock: Entity<DockArea>,
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
        // Native undecorated DockArea keeps split resizing without redundant panel title bars.
        let dock=cx.new(|cx|DockArea::new("chat-dock",None,window,cx));
        let panels=[ChatPane::Stores,ChatPane::Conversations,ChatPane::Messages].map(|pane| {
            cx.new(|cx| ChatDockPanel {
                owner:owner.downgrade(),pane,focus:cx.focus_handle(),
                _updates:cx.observe(&owner,|_,_,cx|cx.notify()),
            })
        });
        dock.update(cx,|area,cx| {
            let mut layout=DockLayout::h_split();
            for (panel,width) in panels.into_iter().zip([Some(px(190.)),Some(px(260.)),None]) {
                layout=layout.child(DockLayout::tabs().panel_view(panel_handle(panel),cx),width);
            }
            area.set_center(layout,window,cx);
        });
        Self{dock,owner,_updates:updates,_appearance:appearance,controls:Default::default(),focus}
    }
}
impl Render for ChatWindow {
    fn render(&mut self,window:&mut Window,cx:&mut Context<Self>)->impl IntoElement {
        self.controls.sync(window,theme::is_dark(cx));
        self.owner.update(cx,|app,cx| {
            app.notify_status(window,cx);
            app.apply_chat_input_result(window,cx);
        });
        v_flex().id("chat-window").role(Role::Group).aria_label("消息窗口").debug_selector(||"chat-window".into())
            .size_full().relative().bg(theme::surface(cx)).text_sm()
            .key_context("ChatWindow").track_focus(&self.focus)
            .on_action(|_:&crate::actions::ChatClose,window,_|window.remove_window())
            .on_action(|_:&crate::actions::QuitApp,_,cx|cx.quit())
            .child(TitleBar::new().child(gpui_component::Icon::new(FunctionIcon::Chat).small()))
            .child(div().flex_1().min_h_0().w_full().child(self.dock.clone()))
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
