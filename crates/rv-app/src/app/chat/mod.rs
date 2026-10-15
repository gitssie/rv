//! Chat controller: host HTTP jobs, local drafts and guarded async view updates.
mod plugins;
mod view;
use super::{AddressBookApp, Modal};
use gpui::*;
use gpui_component::input::{InputEvent, TextareaState};
use rv_business::chat::Action as ChatAction;
use rv_business::chat::*;
use rv_core::workspace::AppId;
use std::{collections::HashSet, time::Duration};
pub(super) struct ChatState {
    pub accounts: Vec<ChatAccount>,
    pub conversations: Vec<Conversation>,
    pub account: Option<AppId>,
    pub unread_only: bool,
    pub selected: Option<String>,
    pub thread: Thread,
    pub composer: Entity<TextareaState>,
    pub clear_composer: Option<(String, String)>,
    pub refresh_on_load: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub busy: HashSet<AppId>,
    pub sequence: u64,
    pub thread_sequence: u64,
    registry: plugins::Registry,
    pub scroll: ScrollHandle,
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
            refresh_on_load: false,
            loading: false,
            error: None,
            busy: HashSet::new(),
            sequence: 0,
            thread_sequence: 0,
            registry: plugins::Registry::default(),
            scroll: ScrollHandle::new(),
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
        self._subscriptions.push(cx.subscribe_in(
            &self.chat.composer,
            window,
            |this, input, event, window, cx| match event {
                InputEvent::PressEnter { shift, .. } if !shift => this.send_chat(window, cx),
                InputEvent::Change => {
                    if !this.read_only {
                        if let Some(id) = &this.chat.selected {
                            let body = input.read(cx).value().to_string();
                            if let Err(error) = save_draft(this.book.paths(), id, &body) {
                                this.chat.error = Some(error);
                            }
                        }
                    }
                }
                _ => {}
            },
        ));
        self.reload_chat(cx);
        // No catch-up or jobs when RV is closed. Refresh only registered, credentialed accounts.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.read_only {
                            return;
                        }
                        let accounts = this
                            .chat
                            .accounts
                            .iter()
                            .map(|a| a.id.clone())
                            .collect::<Vec<_>>();
                        for id in accounts {
                            this.dispatch_chat(id, ChatAction::Inbox, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub(super) fn open_chat(&mut self, cx: &mut Context<Self>) {
        self.modal = Modal::Chat;
        self.chat.refresh_on_load = true;
        self.reload_chat(cx);
        cx.notify();
    }
    pub(super) fn reload_chat(&mut self, cx: &mut Context<Self>) {
        self.chat.sequence += 1;
        let sequence = self.chat.sequence;
        let paths = self.book.paths().clone();
        self.chat.loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let accounts = accounts(&paths)?;
                    let mut conversations = conversations(&paths)?;
                    local_images(&paths, &mut conversations, &mut Thread::default());
                    Ok::<_, String>((accounts, conversations))
                })
                .await;
            let _ =
                this.update(cx, |this, cx| {
                    if this.chat.sequence != sequence {
                        return;
                    }
                    this.chat.loading = false;
                    match result {
                        Ok((accounts, conversations)) => {
                            this.chat.accounts = accounts;
                            this.chat.conversations = conversations;
                            if this.chat.selected.as_ref().is_some_and(|id| {
                                !this.chat.conversations.iter().any(|c| c.id == *id)
                            }) {
                                this.chat.selected = None;
                                this.chat.thread = Thread::default();
                                this.chat.thread_sequence += 1;
                            }
                            this.load_chat_images(sequence, cx);
                            if this.chat.refresh_on_load {
                                this.chat.refresh_on_load = false;
                                let ids = this
                                    .chat
                                    .accounts
                                    .iter()
                                    .map(|a| a.id.clone())
                                    .collect::<Vec<_>>();
                                for id in ids {
                                    this.dispatch_chat(id, ChatAction::Inbox, cx);
                                }
                            }
                        }
                        Err(error) => this.chat.error = Some(error),
                    }
                    cx.notify();
                });
        })
        .detach();
    }
    fn load_chat_images(&mut self, sequence: u64, cx: &mut Context<Self>) {
        let paths = self.book.paths().clone();
        let conversations = self.chat.conversations.clone();
        cx.spawn(async move |this, cx| {
            let conversations = cx
                .background_executor()
                .spawn(async move {
                    let mut conversations = conversations;
                    hydrate_images(&paths, &mut conversations, &mut Thread::default(), None);
                    conversations
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.chat.sequence == sequence {
                    this.chat.conversations = conversations;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn select_chat(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.chat.scroll.scroll_to_bottom();
        self.chat.selected = Some(id.clone());
        self.chat.thread = Thread::default();
        self.chat.error = None;
        let text = draft(self.book.paths(), &id).unwrap_or_default();
        self.chat
            .composer
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.reload_chat_thread(cx);
        if let Some(c) = self.chat.selected().cloned() {
            self.dispatch_chat(c.app_id, ChatAction::Thread(c.remote_id), cx);
        }
        cx.notify();
    }
    fn reload_chat_thread(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.chat.selected.clone() else {
            return;
        };
        self.chat.thread_sequence += 1;
        let sequence = self.chat.thread_sequence;
        let paths = self.book.paths().clone();
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
                match result {
                    Ok(thread) => {
                        if this.chat.thread.messages.is_empty() {
                            this.chat.scroll.scroll_to_bottom();
                        }
                        this.chat.thread = thread;
                        this.load_thread_images(sequence, cx);
                    }
                    Err(error) => this.chat.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load_thread_images(&mut self, sequence: u64, cx: &mut Context<Self>) {
        let paths = self.book.paths().clone();
        let thread = self.chat.thread.clone();
        let thread_app = self
            .chat
            .selected()
            .map(|conversation| conversation.app_id.clone());
        cx.spawn(async move |this, cx| {
            let thread = cx
                .background_executor()
                .spawn(async move {
                    let mut thread = thread;
                    hydrate_images(&paths, &mut [], &mut thread, thread_app.as_ref());
                    thread
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.chat.thread_sequence == sequence {
                    this.chat.thread = thread;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn dispatch_chat(&mut self, id: AppId, action: ChatAction, cx: &mut Context<Self>) {
        if self.read_only || self.chat.busy.contains(&id) || self.home.jobs.contains(&id) {
            return;
        }
        // An account without credentials remains a visible empty state, not a polling error.
        if !rv_core::workspace::WorkspaceStore::open(self.book.paths())
            .and_then(|s| s.auth(&id))
            .is_ok_and(|auth| auth.is_some())
        {
            if matches!(action, ChatAction::Send { .. }) {
                self.chat.error = Some("请先读取此账号资料".into());
                cx.notify();
            }
            return;
        }
        self.chat.busy.insert(id.clone());
        self.chat.error = None;
        let paths = self.book.paths().clone();
        let target = id.clone();
        let remote = match &action {
            ChatAction::Thread(id) | ChatAction::MarkRead(id) => Some(id.clone()),
            ChatAction::Send { conversation, .. } => Some(conversation.clone()),
            ChatAction::Inbox => None,
        };
        let after_send = matches!(action, ChatAction::Send { .. });
        let send = match &action {
            ChatAction::Send { conversation, text } => Some((conversation.clone(), text.clone())),
            _ => None,
        };
        cx.spawn(async move |this, cx| {
            let (result, persisted_send) = cx
                .background_executor()
                .spawn(async move {
                    let result = rv_business::OfflineRunner::new(None)
                        .and_then(|runner| runner.chat_blocking(&paths, &target, action));
                    let persisted_send = send.and_then(|(remote, text)| {
                        let c = conversations(&paths)
                            .ok()?
                            .into_iter()
                            .find(|c| c.app_id == target && c.remote_id == remote)?;
                        let persisted = result.is_ok()
                            || thread(&paths, &c.id)
                                .ok()?
                                .pending
                                .iter()
                                .any(|m| m.body == text);
                        persisted.then_some((c.id, text))
                    });
                    (result, persisted_send)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.chat.busy.remove(&id);
                if persisted_send.is_some() {
                    this.chat.clear_composer = persisted_send;
                }
                if let Err(error) = result {
                    let title = this
                        .chat
                        .accounts
                        .iter()
                        .find(|a| a.id == id)
                        .map(|a| a.title.as_str())
                        .unwrap_or("账号");
                    this.chat.error = Some(format!("{title}：{error}"));
                }
                this.reload_chat(cx);
                if let Some(selected) = this.chat.selected().cloned().filter(|c| c.app_id == id) {
                    this.reload_chat_thread(cx);
                    // Readback is safe; replies are never automatically retried.
                    if after_send || remote.as_deref() != Some(selected.remote_id.as_str()) {
                        this.dispatch_chat(id.clone(), ChatAction::Thread(selected.remote_id), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn send_chat(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
            || self.chat.busy.contains(&c.app_id)
            || self.home.jobs.contains(&c.app_id)
            || self
                .chat
                .thread
                .pending
                .iter()
                .any(|m| ["sending", "unknown", "submitted"].contains(&m.status.as_str()))
        {
            return;
        }
        self.dispatch_chat(
            c.app_id,
            ChatAction::Send {
                conversation: c.remote_id,
                text,
            },
            cx,
        );
    }
    pub(super) fn apply_chat_input_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((id, text)) = self.chat.clear_composer.take() {
            if self.chat.selected.as_ref() == Some(&id)
                && self.chat.composer.read(cx).value().as_ref() == text
            {
                self.chat.scroll.scroll_to_bottom();
                self.chat
                    .composer
                    .update(cx, |state, cx| state.set_value("", window, cx));
            }
        }
    }
    fn filter_chat_account(&mut self, id: Option<AppId>, cx: &mut Context<Self>) {
        self.chat.account = id;
        if self
            .chat
            .selected()
            .is_some_and(|c| self.chat.account.as_ref().is_some_and(|id| c.app_id != *id))
        {
            self.chat.selected = None;
            self.chat.thread = Thread::default();
            self.chat.thread_sequence += 1;
        }
        cx.notify();
    }
}
