//! Chat controller: local projections and immediate async operations; durable sends use outbox.
mod images;
mod notifications;
mod plugins;
#[cfg(test)]
pub(super) mod test_support;
#[cfg(test)]
mod tests;
mod view;
mod window;
use super::AddressBookApp;
use gpui::*;
use gpui_component::input::{InputEvent, TextareaState};
use rv_business::chat::*;
use rv_business::chat::sync::ChatSync;
use rv_core::workspace::AppId;
pub(super) struct ChatState {
    pub accounts: Vec<ChatAccount>,
    pub conversations: Vec<Conversation>,
    pub account: Option<AppId>,
    pub unread_only: bool,
    pub selected: Option<String>,
    pub thread: Thread,
    pub composer: Entity<TextareaState>,
    pub clear_composer: Option<(String, String)>,
    pub loading: bool,
    pub thread_loading: bool,
    pub syncs: std::collections::HashMap<(AppId,ChatSync),usize>,
    pub thread_sequence: u64,
    data_sequences: [u64; 3],
    registry: plugins::Registry,
    pub scroll: Entity<rv_gpui_chat::message_scroller::MessageScrollerState>,
    pub remembered: std::collections::HashMap<AppId, String>,
    pub notifications_open: bool,
    pub notifications: Vec<rv_business::notifications::Notification>,
    pub remembered_notifications: std::collections::HashMap<AppId, String>,
    pub image_preparing: bool,
    pub window: Option<WindowHandle<gpui_component::Root>>,
    pub opening: bool,
    input_subscription: Option<Subscription>,
}
impl ChatState {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            accounts: vec![],
            conversations: vec![],
            account: None,
            unread_only: false,
            selected: None,
            thread: Thread::default(),
            composer: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(1, 3)
                    .submit_on_enter(true)
                    .placeholder("输入消息…")
            }),
            clear_composer: None,
            loading: false,
            thread_loading: false,
            syncs: Default::default(),
            thread_sequence: 0,
            data_sequences: [0; 3],
            registry: plugins::Registry::default(),
            scroll: cx.new(|cx| rv_gpui_chat::message_scroller::MessageScrollerState::new(0, cx)),
            remembered: Default::default(),
            notifications_open: false,
            notifications: vec![],
            remembered_notifications: Default::default(),
            image_preparing: false,
            window: None,
            opening: false,
            input_subscription: None,
        }
    }
    fn selected(&self) -> Option<&Conversation> {
        self.selected
            .as_ref()
            .and_then(|id| self.conversations.iter().find(|c| c.id == *id))
    }
    fn visible(&self) -> impl Iterator<Item = &Conversation> {
        self.conversations.iter().filter(|c| {
            self.account.as_ref().is_none_or(|id| c.app_id == *id)
                && (!self.unread_only || c.unread)
        })
    }
}
impl AddressBookApp {
    pub(super) fn subscribe_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat.input_subscription=Some(cx.subscribe_in(
            &self.chat.composer,
            window,
            |this, input, event, window, cx| match event {
                InputEvent::PressEnter { shift, .. } if !shift => this.send_chat(window, cx),
                InputEvent::Change => {
                    if !this.read_only {
                        if let Some(id) = &this.chat.selected {
                            let body = input.read(cx).value().to_string();
                            if let Err(error) = save_draft(this.book.paths(), id, &body) {
                                this.chat_error(error, cx);
                            }
                        }
                    }
                }
                _ => {}
            },
        ));
        self.reload_chat(cx);
    }
    pub(super) fn open_chat(&mut self,window:&mut Window,cx:&mut Context<Self>) {
        self.open_chat_window(None,window,cx);
    }
    pub(in crate::app) fn open_chat_for(&mut self,id:AppId,window:&mut Window,cx:&mut Context<Self>) {
        self.open_chat_window(Some(id),window,cx);
    }
    pub(super) fn reload_chat(&mut self, cx: &mut Context<Self>) {
        self.reload_chat_data([true; 3], true, cx);
    }
    /// Local projections have independent generations so concurrent domain events
    /// cannot discard another projection's in-flight initial load.
    pub(super) fn reload_chat_data(
        &mut self,
        scopes: [bool; 3],
        thread: bool,
        cx: &mut Context<Self>,
    ) {
        for (index, enabled) in scopes.iter().enumerate() {
            if *enabled {
                self.chat.data_sequences[index] += 1;
            }
        }
        let sequences = self.chat.data_sequences;
        let paths = self.book.paths().clone();
        if scopes[1] {
            self.chat.loading = true;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let accounts = scopes[0].then(|| accounts(&paths)).transpose()?;
                    let conversations = if scopes[1] {
                        let mut rows = conversations(&paths)?;
                        local_images(&paths, &mut rows, &mut Thread::default());
                        Some(rows)
                    } else {
                        None
                    };
                    let notifications = scopes[2]
                        .then(|| rv_business::notifications::list(&paths))
                        .transpose()?;
                    Ok::<_, String>((accounts, conversations, notifications))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !scopes.iter().enumerate().any(|(index, enabled)| {
                    *enabled && this.chat.data_sequences[index] == sequences[index]
                }) {
                    return;
                }
                if scopes[1] && this.chat.data_sequences[1] == sequences[1] {
                    this.chat.loading = false;
                }
                match result {
                    Ok((accounts, conversations, notifications)) => {
                        if this.chat.data_sequences[0] == sequences[0] {
                            if let Some(accounts) = accounts {
                                this.chat.accounts = accounts;
                                if this.chat.account.as_ref().is_none_or(|id| {
                                    !this.chat.accounts.iter().any(|a| a.id == *id)
                                }) {
                                    this.chat.account =
                                        this.chat.accounts.first().map(|a| a.id.clone());
                                }
                            }
                        }
                        if this.chat.data_sequences[1] == sequences[1] {
                            if let Some(conversations) = conversations {
                                this.chat.conversations = conversations;
                                if this.chat.selected.as_ref().is_some_and(|id| {
                                    !this.chat.conversations.iter().any(|c| c.id == *id)
                                }) {
                                    this.chat.selected = None;
                                    this.chat.thread = Thread::default();
                                    this.chat.scroll.update(cx, |scroll,cx| scroll.reset(0,cx));
                                    this.chat.thread_sequence += 1;
                                }
                            }
                        }
                        if this.chat.data_sequences[2] == sequences[2] {
                            if let Some(notifications) = notifications {
                                this.chat.notifications = notifications;
                            }
                        }
                    }
                    Err(error) => this.chat_error(error, cx),
                }
                if thread && this.chat.selected.is_some() {
                    this.reload_chat_thread(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(in crate::app) fn select_chat(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat.scroll.update(cx, |scroll,cx| scroll.scroll_to_end(cx));
        self.chat.selected = Some(id.clone());
        self.chat.thread = Thread::default();
            self.chat.scroll.update(cx, |scroll,cx| scroll.reset(0,cx));
        let text = draft(self.book.paths(), &id).unwrap_or_default();
        self.chat
            .composer
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.reload_chat_thread(cx);
        if let Some(c) = self.chat.selected().cloned() {
            self.chat.remembered.insert(c.app_id.clone(), c.id.clone());
        }
        cx.notify();
    }
    pub(super) fn reload_chat_thread(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.chat.selected.clone() else {
            return;
        };
        self.chat.thread_sequence += 1;
        let sequence = self.chat.thread_sequence;
        let paths = self.book.paths().clone();
        self.chat.thread_loading =
            self.chat.thread.messages.is_empty() && self.chat.thread.pending.is_empty();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut thread = thread(&paths, &id)?;
                    local_images(&paths, &mut [], &mut thread);
                    Ok::<_, String>(thread)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.chat.thread_sequence != sequence {
                    return;
                }
                this.chat.thread_loading = false;
                match result {
                    Ok(thread) => {
                        let old: Vec<_> = this.chat.thread.messages.iter().chain(&this.chat.thread.pending).collect();
                        let new: Vec<_> = thread.messages.iter().chain(&thread.pending).collect();
                        let prefix = old.iter().zip(&new).take_while(|(a,b)| a.id == b.id).count();
                        let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev())
                            .take_while(|(a,b)| a.id == b.id).count();
                        let fresh = old.is_empty();
                        this.chat.scroll.update(cx, |scroll,cx| {
                            let follow = fresh || !scroll.is_scrolled_up();
                            if prefix + suffix < old.len() || prefix + suffix < new.len() {
                                scroll.splice(prefix..old.len()-suffix, new.len()-prefix-suffix, cx);
                            }
                            scroll.remeasure_items(0..new.len(),cx);
                            if follow { scroll.scroll_to_end(cx); }
                        });
                        let read = !this.read_only && this.chat.window.is_some()
                            && !this.chat.notifications_open && !thread.messages.is_empty();
                        this.chat.thread = thread;
                        if let Some(c)=this.chat.selected().filter(|c|read && c.unread && this.chat.account.as_ref()==Some(&c.app_id)).cloned() {
                            let paths=this.book.paths().clone();
                            cx.spawn(async move |this,cx| {
                                let result=cx.background_executor().spawn(async move {
                                    rv_business::chat::request_read(&paths,&c.app_id,&c.id)
                                }).await;
                                let _=this.update(cx,|this,cx|match result {
                                    Ok(())=>this.reload_chat_data([false,true,false],false,cx),
                                    Err(error)=>this.chat_error(error,cx),
                                });
                            }).detach();
                        }
                    }
                    Err(error) => this.chat_error(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn chat_error(&mut self,error:String,cx:&mut Context<Self>) {
        tracing::error!(app_id=?self.chat.account.as_ref().map(|id|id.as_str()),%error,"Chat action failed");
        self.set_error_status(error,cx);
    }
    fn chat_syncing(&self,id:&AppId,operation:&ChatSync)->bool {self.chat.syncs.contains_key(&(id.clone(),operation.clone()))}
    fn run_chat_sync(&mut self,id:AppId,operation:ChatSync,notify:bool,cx:&mut Context<Self>) {
        if self.read_only{return;}
        let key=(id.clone(),operation.clone());
        if !matches!(operation,ChatSync::SendPending(_)) && self.chat.syncs.contains_key(&key){return;}
        *self.chat.syncs.entry(key.clone()).or_default()+=1;
        let paths=self.book.paths().clone();cx.notify();
        cx.spawn(async move |this,cx| {
            let result=rv_business::chat::sync::synchronize(paths,id,operation.clone()).await;
            let _=this.update(cx,|this,cx| {
                if let Some(count)=this.chat.syncs.get_mut(&key){*count-=1;if *count==0{this.chat.syncs.remove(&key);}}
                match result {
                    Ok(())=>{if notify{this.set_status(format!("{}完成",operation.label()),cx);}},
                    Err(error)=>this.chat_error(format!("{}失败：{error}",operation.label()),cx),
                }
                cx.notify();
            });
        }).detach();
    }
    fn refresh_chat(&mut self,cx:&mut Context<Self>) {
        if let Some(id)=self.chat.account.clone() {
            self.run_chat_sync(id.clone(),ChatSync::Inbox,true,cx);
        }
    }
    fn send_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.notifications_open {
            return;
        }
        let Some(c) = self.chat.selected().cloned() else {
            return;
        };
        let text = self.chat.composer.read(cx).value().to_string();
        let capability = rv_business::chat::strategy(&c.bundle)
            .map(|p| p.capabilities())
            .unwrap_or_default();
        if self.read_only
            || c.allow_reply != Some(true)
            || !capability.text
            || text.trim().is_empty()
            || !self
                .chat
                .accounts
                .iter()
                .any(|a| a.id == c.app_id && a.can_send)
        {
            return;
        }
        match queue_message(self.book.paths(), &c.app_id, &c.id, &text) {
            Ok(_) => {
                self.chat.clear_composer = Some((c.id.clone(), text));
                self.reload_chat_thread(cx);
                self.run_chat_sync(c.app_id.clone(),ChatSync::SendPending(c.id),false,cx);
            }
            Err(error) => self.chat_error(error, cx),
        }
        cx.notify();
    }
    pub(super) fn apply_chat_input_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, text)) = self.chat.clear_composer.take() {
            if self.chat.selected.as_ref() == Some(&id)
                && self.chat.composer.read(cx).value().as_ref() == text
            {
                self.chat.scroll.update(cx, |scroll,cx| scroll.scroll_to_end(cx));
                self.chat
                    .composer
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
        }
    }
    fn filter_chat_account(
        &mut self,
        id: Option<AppId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat.account = id;
        if self
            .chat
            .selected()
            .is_some_and(|c| self.chat.account.as_ref().is_some_and(|id| c.app_id != *id))
        {
            self.chat.selected = None;
            self.chat.thread = Thread::default();
            self.chat.scroll.update(cx, |scroll,cx| scroll.reset(0,cx));
            self.chat.thread_sequence += 1;
        }
        if let Some(id) = self
            .chat
            .account
            .as_ref()
            .and_then(|id| self.chat.remembered.get(id))
            .cloned()
            .filter(|_| !self.chat.notifications_open)
        {
            self.select_chat(id, window, cx);
        }
        cx.notify();
    }
}
