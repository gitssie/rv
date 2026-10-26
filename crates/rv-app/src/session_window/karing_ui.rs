use super::*;
use rv_session::{
    AppCommand, KaringCatalogue, KaringRequest, KaringResponse, KaringSelection, karing_component,
};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone)]
struct Provider {
    name: String,
    label: String,
    nodes: Vec<String>,
    refreshable: bool,
}
enum Pending {
    Providers,
    Proxies,
    Refresh,
    Delay(String),
    NativeProfiles,
    Connection,
    Control,
    Selection(String),
    VerifySelection(String),
    PrepareRelay,
    RestoreSelection,
}
#[derive(Default)]
pub(super) struct KaringUI {
    sequence: u32,
    pending: HashMap<u32, Pending>,
    providers: Vec<Provider>,
    nodes: Vec<String>,
    current_node: Option<String>,
    nodes_loaded: bool,
    providers_loaded: bool,
    selected: Option<String>,
    reachable: bool,
    message: String,
    delays: Vec<String>,
    delay_successes: usize,
    load_error: Option<String>,
    show_providers: bool,
    show_results: bool,
    delay_queue: std::collections::VecDeque<String>,
    catalogue: KaringCatalogue,
    exit: Option<String>,
    chain: bool,
    show_exits: bool,
    vpn_connected: Option<bool>,
    selecting_best: bool,
    samples: HashMap<String, u64>,
    applied_selection: Option<KaringSelection>,
    restore_selection: Option<KaringSelection>,
}
impl KaringUI {
    pub(super) fn disconnected(&mut self) {
        // Keep the sequence monotonic so replies from the old session cannot match.
        let sequence = self.sequence;
        *self = Self {
            sequence,
            ..Self::default()
        };
    }
    fn provider(&self) -> Option<&Provider> {
        self.providers
            .iter()
            .find(|p| Some(&p.name) == self.selected.as_ref())
    }
    fn loaded_message(&mut self) {
        if self.load_error.is_none() && self.nodes_loaded && self.providers_loaded {
            self.message = if self.providers.is_empty() {
                format!("已读取 {} 个节点", self.nodes.len())
            } else {
                "配置已刷新".into()
            };
        }
    }
}

pub(super) fn is_karing(id: &str, name: &str) -> bool {
    id == "com.nebula.karing" || name.eq_ignore_ascii_case("karing")
}
impl SessionView {
    fn karing_native(&self) -> bool {
        self.apps
            .caps
            .as_ref()
            .is_some_and(|caps| caps.karing_node_control)
    }
    fn karing_native_request(&mut self, kind: Pending, operation: &str, body: Value) {
        self.karing.sequence = self.karing.sequence.wrapping_add(1);
        let id = self.karing.sequence;
        self.karing.pending.insert(id, kind);
        self.handle.app(AppCommand::Karing(KaringRequest {
            id,
            native: true,
            method: "POST".into(),
            path: format!("/{operation}"),
            body: body.to_string().into_bytes(),
        }));
    }
    fn connect_karing(&mut self, cx: &mut Context<Self>) {
        if !self.karing_allowed() || !self.karing.pending.is_empty() {
            return;
        }
        let operation = if self.karing.vpn_connected == Some(true) {
            "disconnect"
        } else {
            "connect"
        };
        self.karing.message = if operation == "connect" {
            "连接 VPN…"
        } else {
            "断开 VPN…"
        }
        .into();
        self.karing_native_request(Pending::Control, operation, serde_json::json!({}));
        cx.notify();
    }
    fn apply_karing_selection(&mut self, cx: &mut Context<Self>) {
        if !self.karing_allowed()
            || !self.karing.pending.is_empty()
            || self.karing.vpn_connected != Some(true)
        {
            return;
        }
        let Some(exit) = self.karing.exit.clone() else {
            return;
        };
        self.karing.samples.clear();
        self.karing.restore_selection = None;
        if self.karing.chain {
            let Some(upstream) = self.karing.selected.as_deref() else {
                return;
            };
            match self.karing.catalogue.relay_candidates(upstream, &exit) {
                Ok(nodes) => {
                    let Some(previous) = self.karing.applied_selection.clone() else {
                        self.karing.message = "请先读取配置；多跳或后置链需先应用单节点出口".into();
                        cx.notify();
                        return;
                    };
                    self.karing.delay_queue = nodes.into();
                    self.karing.selecting_best = true;
                    self.karing.delays.clear();
                    self.karing.delay_successes = 0;
                    self.karing.restore_selection = Some(previous.clone());
                    self.karing.message = "准备节点独立测速…".into();
                    // Existing global detours would bias relay measurements.
                    // Prepare direct candidates first, retaining the previous
                    // exit. If every candidate fails, restore the prior choice.
                    let direct = KaringSelection {
                        exit_tag: previous.exit_tag,
                        upstream_group: None,
                        relay_tag: None,
                    };
                    self.karing_native_request(
                        Pending::PrepareRelay,
                        "select",
                        serde_json::to_value(direct).unwrap(),
                    );
                }
                Err(error) => self.karing.message = error,
            }
        } else {
            self.submit_karing_selection();
        }
        cx.notify();
    }
    fn submit_karing_selection(&mut self) {
        self.karing.selecting_best = false;
        let Some(exit) = self.karing.exit.as_deref() else {
            return;
        };
        let upstream = self
            .karing
            .chain
            .then_some(self.karing.selected.as_deref())
            .flatten();
        if self.karing.chain && upstream.is_none() {
            self.karing.message = "请选择前置订阅".into();
            return;
        }
        match self
            .karing
            .catalogue
            .select(exit, upstream, &self.karing.samples)
        {
            Ok(selection) => {
                self.karing.message = "保存配置并重载…".into();
                self.karing_native_request(
                    Pending::Selection(selection.exit_tag.clone()),
                    "select",
                    serde_json::to_value(selection).unwrap(),
                );
            }
            Err(error) => {
                if let Some(previous) = self.karing.restore_selection.take() {
                    self.karing.message = format!("{error}；恢复原配置…");
                    self.karing_native_request(
                        Pending::RestoreSelection,
                        "select",
                        serde_json::to_value(previous).unwrap(),
                    );
                } else {
                    self.karing.message = error;
                }
            }
        }
    }
    fn karing_allowed(&self) -> bool {
        self.phase == Phase::Connected
            && !self.view_only()
            && self
                .apps
                .caps
                .as_ref()
                .is_some_and(|c| c.control && c.zx_control.is_some())
    }
    fn karing_request(&mut self, kind: Pending, method: &str, path: String) {
        self.karing.sequence = self.karing.sequence.wrapping_add(1);
        let id = self.karing.sequence;
        self.karing.pending.insert(id, kind);
        self.handle.app(AppCommand::Karing(KaringRequest {
            id,
            native: false,
            method: method.into(),
            path,
            body: vec![],
        }));
    }
    pub(super) fn refresh_karing(&mut self, cx: &mut Context<Self>) {
        if !self.karing_allowed() || !self.karing.pending.is_empty() {
            return;
        }
        self.karing.load_error = None;
        self.karing.nodes_loaded = false;
        self.karing.providers_loaded = false;
        self.karing.current_node = None;
        self.karing.message = "读取中…".into();
        if self.karing_native() {
            self.karing_native_request(Pending::NativeProfiles, "profiles", serde_json::json!({}));
            self.karing_native_request(Pending::Connection, "connection", serde_json::json!({}));
        } else {
            self.karing_request(Pending::Providers, "GET", "/providers/proxies".into());
        }
        self.karing_request(Pending::Proxies, "GET", "/proxies".into());
        cx.notify();
    }
    fn refresh_karing_subscription(&mut self, cx: &mut Context<Self>) {
        if !self.karing_allowed() || !self.karing.pending.is_empty() {
            return;
        }
        let Some(provider) = self.karing.provider().filter(|p| p.refreshable) else {
            return;
        };
        let path = format!("/providers/proxies/{}", karing_component(&provider.name));
        self.karing.message = "刷新订阅中…".into();
        self.karing_request(Pending::Refresh, "PUT", path);
        cx.notify();
    }
    pub(super) fn cancel_karing(&mut self, cx: &mut Context<Self>) {
        if !self.karing.pending.is_empty() {
            self.handle.app(AppCommand::CancelKaring);
            self.karing.pending.clear();
            self.karing.delay_queue.clear();
            self.karing.selecting_best = false;
            self.karing.applied_selection = None;
            self.karing.restore_selection = None;
            self.karing.message = "请求已停止；未确认的修改请在 Karing 检查".into();
        }
        self.karing.show_providers = false;
        self.karing.show_results = false;
        cx.notify();
    }
    fn test_karing(&mut self, cx: &mut Context<Self>) {
        if !self.karing_allowed() || !self.karing.pending.is_empty() {
            return;
        }
        let nodes = self
            .karing
            .provider()
            .map(|p| p.nodes.clone())
            .unwrap_or_else(|| self.karing.nodes.clone());
        if nodes.is_empty() {
            return;
        }
        if nodes.len() > 512 {
            self.karing.message = "节点较多，请选择单个订阅测速".into();
            cx.notify();
            return;
        }
        self.karing.delays.clear();
        self.karing.samples.clear();
        self.karing.selecting_best = false;
        self.karing.delay_successes = 0;
        self.karing.delay_queue = nodes.into();
        self.karing.message = "测速中…".into();
        self.next_karing_delay();
        cx.notify();
    }
    fn next_karing_delay(&mut self) {
        if !self.karing_allowed() {
            self.karing.delay_queue.clear();
            self.karing.message = "Karing 控制已停止".into();
            return;
        }
        if let Some(node) = self.karing.delay_queue.pop_front() {
            self.karing_request(Pending::Delay(node.clone()), "GET", format!(
                "/proxies/{}/delay?url=https%3A%2F%2Fwww.gstatic.com%2Fgenerate_204&timeout=5000", karing_component(&node)));
        } else {
            if self.karing.selecting_best {
                self.submit_karing_selection();
                return;
            }
            let count = self.karing.delays.len();
            self.karing.message = format!(
                "{} 成功 · {} 失败",
                self.karing.delay_successes,
                count - self.karing.delay_successes
            );
        }
    }
    pub(super) fn karing_failure(&mut self, id: u32, message: String, cx: &mut Context<Self>) {
        let Some(kind) = self.karing.pending.remove(&id) else {
            return;
        };
        if matches!(
            kind,
            Pending::Selection(_) | Pending::PrepareRelay | Pending::RestoreSelection
        ) {
            self.karing.applied_selection = None;
            self.karing.restore_selection = None;
            self.karing.selecting_best = false;
            self.karing.delay_queue.clear();
        }
        if let Pending::Delay(node) = kind {
            self.karing.delays.push(format!("{node}: {message}"));
            self.karing.delay_queue.clear();
            if self.karing.selecting_best {
                self.karing.selecting_best = false;
                if let Some(previous) = self.karing.restore_selection.take() {
                    self.karing.message = "测速请求中断，恢复原出口配置…".into();
                    self.karing_native_request(
                        Pending::RestoreSelection,
                        "select",
                        serde_json::to_value(previous).unwrap(),
                    );
                } else {
                    self.karing.applied_selection = None;
                    self.karing.message =
                        "测速请求中断，临时单节点配置的恢复待确认，请重新读取".into();
                }
                cx.notify();
                return;
            }
            self.karing.selecting_best = false;
        }
        self.karing.load_error = Some(message.clone());
        self.karing.message = message;
        cx.notify();
    }
    pub(super) fn apply_karing_response(
        &mut self,
        response: KaringResponse,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.karing.pending.remove(&response.id) else {
            return;
        };
        if !(200..300).contains(&response.status) {
            if matches!(
                kind,
                Pending::Selection(_) | Pending::PrepareRelay | Pending::RestoreSelection
            ) {
                self.karing.applied_selection = None;
                self.karing.restore_selection = None;
                self.karing.selecting_best = false;
                self.karing.delay_queue.clear();
            }
            let message = format!("Karing 返回 HTTP {}", response.status);
            if let Pending::Delay(node) = kind {
                self.karing.delays.push(format!("{node}: {message}"));
                self.next_karing_delay();
            } else {
                self.karing.load_error = Some(message.clone());
                self.karing.message = message;
            }
            cx.notify();
            return;
        }
        let value = serde_json::from_slice::<Value>(&response.body);
        match kind {
            Pending::NativeProfiles => {
                match value
                    .as_ref()
                    .ok()
                    .and_then(|v| v.get("data"))
                    .ok_or_else(|| "Karing 订阅返回格式不正确".to_string())
                    .and_then(|documents| {
                        KaringCatalogue::from_profiles(documents)
                            .map(|catalogue| (documents, catalogue))
                    }) {
                    Ok((documents, catalogue)) => {
                        self.karing.providers = catalogue
                            .subscriptions
                            .iter()
                            .map(|s| Provider {
                                name: s.id.clone(),
                                label: s.name.clone(),
                                nodes: s.nodes.clone(),
                                refreshable: false,
                            })
                            .collect();
                        if !catalogue
                            .nodes
                            .iter()
                            .any(|tag| Some(tag) == self.karing.exit.as_ref())
                        {
                            self.karing.exit =
                                documents["karing_subscribe_use.json"]["select_default"]
                                    .as_str()
                                    .filter(|tag| catalogue.nodes.iter().any(|node| node == tag))
                                    .map(str::to_owned);
                        }
                        let relay = documents["karing_setting.json"]["chain_proxy"]
                            .as_array()
                            .and_then(|chain| chain.first())
                            .and_then(Value::as_str);
                        self.karing.applied_selection = catalogue.saved_selection(documents);
                        self.karing.chain = relay.is_some();
                        if !catalogue
                            .subscriptions
                            .iter()
                            .any(|s| Some(&s.id) == self.karing.selected.as_ref())
                        {
                            self.karing.selected = catalogue
                                .subscriptions
                                .iter()
                                .find(|s| {
                                    relay.is_some_and(|tag| s.nodes.iter().any(|node| node == tag))
                                })
                                .or_else(|| catalogue.subscriptions.first())
                                .map(|s| s.id.clone());
                        }
                        self.karing.catalogue = catalogue;
                        self.karing.providers_loaded = true;
                        self.karing.loaded_message();
                    }
                    Err(error) => {
                        self.karing.load_error = Some(error.clone());
                        self.karing.message = error;
                    }
                }
            }
            Pending::Connection | Pending::Control => {
                let connected = value
                    .ok()
                    .and_then(|v| v["data"]["vpn_connected"].as_bool());
                self.karing.vpn_connected = connected;
                if connected.is_none() {
                    self.karing.message = "VPN 状态未确认，请重新读取".into();
                } else if matches!(kind, Pending::Control) {
                    self.karing.message = if connected == Some(true) {
                        "VPN 已连接"
                    } else {
                        "VPN 已断开"
                    }
                    .into();
                    if connected == Some(true) {
                        self.karing_request(Pending::Proxies, "GET", "/proxies".into());
                    }
                }
            }
            Pending::Selection(exit) => {
                if value
                    .ok()
                    .is_some_and(|v| v["outcome"] == "succeeded" && v["reloadVerified"] == true)
                {
                    self.karing.message = "重载完成，核对当前出口…".into();
                    self.karing_request(Pending::VerifySelection(exit), "GET", "/proxies".into());
                } else {
                    self.karing.message = "修改结果未确认，请读取实际配置后再操作".into();
                }
            }
            Pending::PrepareRelay => {
                if value
                    .ok()
                    .is_some_and(|v| v["outcome"] == "succeeded" && v["reloadVerified"] == true)
                {
                    self.karing.message = "选择最快中介…".into();
                    self.next_karing_delay();
                } else {
                    self.karing.selecting_best = false;
                    self.karing.delay_queue.clear();
                    self.karing.applied_selection = None;
                    self.karing.message = "测速准备结果未确认，请重新读取配置".into();
                }
            }
            Pending::RestoreSelection => {
                self.karing.message = if value
                    .ok()
                    .is_some_and(|v| v["outcome"] == "succeeded" && v["reloadVerified"] == true)
                {
                    "中介测速未完成，已恢复原出口配置".into()
                } else {
                    self.karing.applied_selection = None;
                    "中介测速失败，恢复结果未确认，请重新读取配置".into()
                };
            }
            Pending::VerifySelection(exit) => {
                self.karing.current_node = value
                    .ok()
                    .and_then(|v| v["proxies"].as_object().and_then(current_node));
                self.karing.message = if self.karing.current_node.as_deref() == Some(&exit) {
                    self.karing.applied_selection = self
                        .karing
                        .catalogue
                        .select(
                            &exit,
                            self.karing
                                .chain
                                .then_some(self.karing.selected.as_deref())
                                .flatten(),
                            &self.karing.samples,
                        )
                        .ok();
                    self.karing.restore_selection = None;
                    "出口切换完成".into()
                } else {
                    "配置已保存，但当前出口未确认，请重新读取".into()
                };
            }
            Pending::Providers => {
                if let Ok(value) = value
                    && let Some(providers) = value["providers"].as_object()
                {
                    self.karing.providers = providers
                        .iter()
                        .map(|(name, p)| Provider {
                            name: name.clone(),
                            label: name.clone(),
                            nodes: proxy_names(&p["proxies"]),
                            refreshable: p["vehicleType"].as_str() == Some("HTTP"),
                        })
                        .collect();
                    if !self
                        .karing
                        .providers
                        .iter()
                        .any(|p| Some(&p.name) == self.karing.selected.as_ref())
                    {
                        self.karing.selected =
                            self.karing.providers.first().map(|p| p.name.clone());
                    }
                    self.karing.reachable = true;
                    self.karing.providers_loaded = true;
                    self.karing.loaded_message();
                } else {
                    self.karing.load_error = Some("订阅配置返回格式不正确".into());
                    self.karing.message = "订阅配置返回格式不正确".into();
                }
            }
            Pending::Proxies => {
                if let Ok(value) = value
                    && let Some(proxies) = value["proxies"].as_object()
                {
                    self.karing.nodes = proxies
                        .iter()
                        .filter(|(_, p)| measurable(p))
                        .map(|(name, _)| name.clone())
                        .collect();
                    self.karing.current_node = current_node(proxies);
                    self.karing.nodes_loaded = true;
                    self.karing.reachable = true;
                    self.karing.loaded_message();
                } else {
                    self.karing.load_error = Some("节点返回格式不正确".into());
                    self.karing.message = "节点返回格式不正确".into();
                }
            }
            Pending::Refresh => {
                self.refresh_karing(cx);
            }
            Pending::Delay(node) => {
                self.karing.delays.push(match value {
                    Ok(value) if value["delay"].as_u64().is_some() => {
                        let delay = value["delay"].as_u64().unwrap();
                        if delay > 0 {
                            self.karing.samples.insert(node.clone(), delay);
                        }
                        self.karing.delay_successes += 1;
                        format!("{node}: {} ms", value["delay"])
                    }
                    _ => format!("{node}: 测速返回格式不正确"),
                });
                self.next_karing_delay();
            }
        }
        cx.notify();
    }
    pub(super) fn render_karing_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.karing_allowed() && self.karing.pending.is_empty();
        let nodes = self
            .karing
            .provider()
            .map(|p| p.nodes.len())
            .unwrap_or(self.karing.nodes.len());
        let status = if !self.karing_allowed() {
            "不可用"
        } else if self.karing.reachable {
            "可用"
        } else {
            "未读取"
        };
        let mut chips = h_flex().gap_1().flex_1().min_w_0().overflow_hidden();
        let mut visible = self.karing.providers.iter().collect::<Vec<_>>();
        visible.sort_by_key(|provider| self.karing.selected.as_ref() != Some(&provider.name));
        for provider in visible.into_iter().take(2) {
            let name = provider.name.clone();
            let selected = self.karing.selected.as_ref() == Some(&name);
            chips = chips.child(
                Button::new(SharedString::from(format!("karing-provider-{name}")))
                    .ghost()
                    .label(format!("{} · {}", provider.label, provider.nodes.len()))
                    .w(px(if self.karing.providers.len() > 2 {
                        72.
                    } else {
                        86.
                    }))
                    .overflow_hidden()
                    .selected(selected)
                    .tooltip("选择订阅作为刷新、测速的操作范围")
                    .disabled(!enabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.karing.selected = Some(name.clone());
                        cx.notify();
                    })),
            );
        }
        if self.karing.providers.len() > 2 {
            chips = chips.child(
                Button::new("karing-more")
                    .ghost()
                    .label(format!("+{}", self.karing.providers.len() - 2))
                    .debug_selector(|| "karing-more".into())
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.karing.show_providers = !this.karing.show_providers;
                        cx.notify();
                    })),
            );
        }
        if self.karing.providers.is_empty() {
            chips = chips.child(div().text_color(theme::muted(cx)).child(
                if self.karing.nodes_loaded {
                    format!("{} 个节点", self.karing.nodes.len())
                } else {
                    "未读取".into()
                },
            ));
        }
        v_flex()
            .id("karing-panel")
            .debug_selector(|| "karing-panel".into())
            .w(px(304.))
            .p_2()
            .gap_1()
            .text_sm()
            .child(
                h_flex()
                    .py_1()
                    .items_center()
                    .gap_2()
                    .child(div().font_semibold().child("Karing"))
                    .child(div().size(px(6.)).rounded_full().bg(
                        if self.karing.reachable && self.karing_allowed() {
                            theme::success(cx)
                        } else {
                            theme::muted(cx)
                        },
                    ))
                    .child(div().text_color(theme::muted(cx)).child(status))
                    .when(self.karing_native(), |row| {
                        row.child(
                            Button::new("karing-vpn-toggle")
                                .ghost()
                                .label(match self.karing.vpn_connected {
                                    Some(true) => "断开 VPN",
                                    Some(false) => "连接 VPN",
                                    None => "读取 VPN",
                                })
                                .disabled(!enabled || self.karing.vpn_connected.is_none())
                                .on_click(cx.listener(|this, _, _, cx| this.connect_karing(cx))),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(div().w(px(32.)).text_color(theme::muted(cx)).child("配置"))
                    .child(chips)
                    .child(
                        crate::ui::icon_button(
                            "karing-refresh-subscription",
                            crate::assets::FunctionIcon::Refresh,
                            "刷新订阅",
                        )
                        .debug_selector(|| "karing-refresh-subscription".into())
                        .tooltip("刷新所选订阅")
                        .disabled(
                            !enabled || !self.karing.provider().is_some_and(|p| p.refreshable),
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.refresh_karing_subscription(cx)),
                        ),
                    )
                    .child(
                        crate::ui::icon_button("karing-test", IconName::Network, "节点测速")
                            .debug_selector(|| "karing-test".into())
                            .tooltip("节点延迟测速")
                            .disabled(!enabled || nodes == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.test_karing(cx))),
                    ),
            )
            .when(self.karing.show_providers, |panel| {
                panel.child(
                    div()
                        .id("karing-providers-list")
                        .max_h(px(144.))
                        .overflow_y_scroll()
                        .child(v_flex().gap_1().children(
                            self.karing.providers.clone().into_iter().map(|p| {
                                Button::new(SharedString::from(p.name.clone()))
                                    .ghost()
                                    .label(p.label.clone())
                                    .w_full()
                                    .debug_selector({
                                        let name = p.name.clone();
                                        move || format!("karing-more-provider-{name}")
                                    })
                                    .disabled(!enabled)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.karing.selected = Some(p.name.clone());
                                        this.karing.show_providers = false;
                                        cx.notify();
                                    }))
                            }),
                        )),
                )
            })
            .when_some(self.karing.current_node.clone(), |panel, node| {
                panel.child(
                    h_flex()
                        .py_1()
                        .items_center()
                        .gap_1()
                        .child(div().w(px(32.)).text_color(theme::muted(cx)).child("当前"))
                        .child(
                            div()
                                .id("karing-current-node")
                                .debug_selector(|| "karing-current-node".into())
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .child(node),
                        ),
                )
            })
            .when(self.karing_native(), |panel| {
                panel
                    .child(
                        h_flex().gap_1().items_center().child(
                            Button::new("karing-chain-toggle")
                                .ghost()
                                .label(if self.karing.chain {
                                    "链式代理：开启"
                                } else {
                                    "链式代理：关闭"
                                })
                                .selected(self.karing.chain)
                                .disabled(!enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.karing.chain = !this.karing.chain;
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(self.karing.chain, |panel| {
                        panel.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .child("从上方所选订阅中测速选择最快中介，再经下方出口联网"),
                        )
                    })
                    .child(
                        Button::new("karing-exit-select")
                            .ghost()
                            .w_full()
                            .label(format!(
                                "出口：{}",
                                self.karing.exit.as_deref().unwrap_or("请选择节点")
                            ))
                            .disabled(!enabled || self.karing.catalogue.nodes.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.karing.show_exits = !this.karing.show_exits;
                                cx.notify();
                            })),
                    )
                    .when(self.karing.show_exits, |panel| {
                        panel.child(
                            div()
                                .id("karing-exit-list")
                                .max_h(px(160.))
                                .overflow_y_scroll()
                                .child(v_flex().gap_1().children(
                                    self.karing.catalogue.nodes.iter().cloned().map(|tag| {
                                        Button::new(SharedString::from(format!(
                                            "karing-exit-{tag}"
                                        )))
                                        .ghost()
                                        .label(tag.clone())
                                        .w_full()
                                        .selected(self.karing.exit.as_ref() == Some(&tag))
                                        .disabled(!enabled)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.karing.exit = Some(tag.clone());
                                                this.karing.show_exits = false;
                                                cx.notify();
                                            }),
                                        )
                                    }),
                                )),
                        )
                    })
                    .child(
                        Button::new("karing-apply-selection")
                            .primary()
                            .label("应用出口")
                            .disabled(
                                !enabled
                                    || self.karing.exit.is_none()
                                    || self.karing.vpn_connected != Some(true),
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.apply_karing_selection(cx)),
                            ),
                    )
            })
            .child(
                h_flex()
                    .border_t_1()
                    .border_color(theme::line(cx))
                    .pt_1()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .child(if self.karing.message.is_empty() {
                                "等待配置".to_string()
                            } else {
                                self.karing.message.clone()
                            }),
                    )
                    .child(
                        Button::new("karing-delay-results")
                            .ghost()
                            .label("结果")
                            .debug_selector(|| "karing-delay-results".into())
                            .disabled(self.karing.delays.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.karing.show_results = !this.karing.show_results;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("karing-refresh-nodes")
                            .ghost()
                            .label("刷新节点")
                            .debug_selector(|| "karing-refresh-nodes".into())
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_karing(cx))),
                    ),
            )
            .when(self.karing.show_results, |panel| {
                panel.child(
                    div()
                        .id("karing-delay-list")
                        .debug_selector(|| "karing-delay-list".into())
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .child(
                            v_flex().gap_1().children(
                                self.karing
                                    .delays
                                    .iter()
                                    .cloned()
                                    .map(|result| div().text_xs().child(result)),
                            ),
                        ),
                )
            })
            .into_any_element()
    }
}
fn measurable(proxy: &Value) -> bool {
    proxy["type"].as_str().is_some_and(|kind| {
        !matches!(
            kind.to_ascii_lowercase().as_str(),
            "direct"
                | "reject"
                | "rejectdrop"
                | "selector"
                | "urltest"
                | "fallback"
                | "loadbalance"
                | "load-balance"
        )
    })
}
fn current_node(proxies: &serde_json::Map<String, Value>) -> Option<String> {
    let mut name = proxies.get("GLOBAL")?["now"].as_str()?;
    for _ in 0..=proxies.len() {
        let proxy = proxies.get(name)?;
        if measurable(proxy) {
            return Some(name.to_owned());
        }
        name = proxy["now"].as_str()?;
    }
    None
}
fn proxy_names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| measurable(p))
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::setup;
    use super::{AppCommand, KaringRequest, KaringResponse, SessionView};
    use gpui::{
        Entity, Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext, px,
    };
    use rv_session::{AppCapabilities, AppEvent, RemoteApp, SessionCommand, SessionTestPeer};
    use serde_json::Value;

    fn enable(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            view.apps.shortcuts.push(rv_core::AppShortcut {
                bundle_id: "com.nebula.karing".into(),
                name: "Karing".into(),
            });
            let caps: AppCapabilities = serde_json::from_value(serde_json::json!({
                "control":true,"list":true,"launch":true,
                "zx_control":{"version":2,"port":6000,"token":"fixture-token"}
            }))
            .unwrap();
            view.apply_app_event(AppEvent::Capabilities(caps), cx);
            view.apply_app_event(
                AppEvent::List(vec![RemoteApp {
                    bundle_id: "com.nebula.karing".into(),
                    name: "Karing".into(),
                    can_launch: true,
                    can_terminate: true,
                }]),
                cx,
            );
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    fn open(cx: &mut VisualTestContext) {
        let bounds = cx.debug_bounds("shortcut-com.nebula.karing").unwrap();
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Right,
            position: bounds.center(),
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    fn requests(peer: &mut SessionTestPeer) -> Vec<KaringRequest> {
        std::iter::from_fn(|| peer.commands.try_recv().ok())
            .filter_map(|command| match command {
                SessionCommand::App(AppCommand::Karing(request)) => Some(request),
                _ => None,
            })
            .collect()
    }
    fn reply(
        view: &Entity<SessionView>,
        cx: &mut VisualTestContext,
        request: &KaringRequest,
        status: u16,
        body: Value,
    ) {
        view.update(cx, |view, cx| {
            view.apply_app_event(
                AppEvent::Karing(KaringResponse {
                    id: request.id,
                    status,
                    headers: vec![],
                    body: body.to_string().into_bytes(),
                }),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[gpui::test]
    fn offscreen_karing_empty_providers_show_real_running_nodes(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open(cx);
        let initial = requests(&mut peer);
        // The phone returns app-managed nodes while exposing no Clash providers.
        // Both arrival orders must retain the node information.
        for reverse in [false, true] {
            let mut ordered = initial.clone();
            if reverse {
                ordered.reverse();
            }
            view.update(cx, |view, _| {
                view.karing.pending.clear();
                view.karing.nodes_loaded = false;
                view.karing.providers_loaded = false;
                view.karing.current_node = None;
                for request in &ordered {
                    view.karing.pending.insert(
                        request.id,
                        if request.path == "/proxies" {
                            super::Pending::Proxies
                        } else {
                            super::Pending::Providers
                        },
                    );
                }
            });
            for request in &ordered {
                reply(
                    &view,
                    cx,
                    request,
                    200,
                    if request.path == "/providers/proxies" {
                        serde_json::json!({"providers":{}})
                    } else {
                        serde_json::json!({"proxies":{
                            "GLOBAL":{"type":"Fallback","now":"FR","all":["FR","node-a","urltest_out"]},
                            "FR":{"type":"SOCKS","history":[{"delay":386}]},
                            "node-a":{"type":"VLESS"},
                            "urltest_out":{"type":"URLTest","now":"node-a"},
                            "direct_out":{"type":"Direct"}
                        }})
                    },
                );
            }
            view.read_with(cx, |view, _| {
                assert_eq!(view.karing.message, "已读取 2 个节点");
                assert_eq!(view.karing.current_node.as_deref(), Some("FR"));
            });
            assert!(cx.debug_bounds("karing-current-node").is_some());
            let panel = cx.debug_bounds("karing-panel").unwrap();
            assert!(
                panel.size.width <= px(320.) && panel.size.height <= px(220.),
                "{panel:?}"
            );
        }
        view.update(cx, |view, cx| view.refresh_karing(cx));
        for request in requests(&mut peer) {
            reply(
                &view,
                cx,
                &request,
                if request.path == "/proxies" { 503 } else { 200 },
                serde_json::json!({"providers":{}}),
            );
        }
        view.read_with(cx, |view, _| {
            assert!(view.karing.current_node.is_none());
            assert_eq!(view.karing.message, "Karing 返回 HTTP 503");
        });
        assert!(cx.debug_bounds("karing-current-node").is_none());
    }

    #[gpui::test]
    fn offscreen_karing_is_compact_empty_providers_disable_refresh_and_test_current_nodes(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open(cx);
        let panel = cx.debug_bounds("karing-panel").unwrap();
        assert!(
            panel.size.width <= px(320.) && panel.size.height <= px(220.),
            "{panel:?}"
        );
        let initial = requests(&mut peer);
        assert_eq!(initial.len(), 2);
        for request in initial {
            reply(
                &view,
                cx,
                &request,
                200,
                if request.path == "/providers/proxies" {
                    serde_json::json!({"providers":{}})
                } else {
                    serde_json::json!({"proxies":{"DIRECT":{"type":"Direct"}, "自动":{"type":"URLTest"}, "FR/中文":{"type":"SOCKS"}}})
                },
            );
        }
        let refresh = cx.debug_bounds("karing-refresh-subscription").unwrap();
        cx.simulate_click(refresh.center(), Modifiers::default());
        assert!(requests(&mut peer).is_empty());
        let test = cx.debug_bounds("karing-test").unwrap();
        cx.simulate_click(test.center(), Modifiers::default());
        let test = requests(&mut peer);
        assert_eq!(test.len(), 1);
        assert_eq!(test[0].method, "GET");
        assert!(
            test[0]
                .path
                .starts_with("/proxies/FR%2F%E4%B8%AD%E6%96%87/delay?")
        );
        reply(
            &view,
            cx,
            &test[0],
            200,
            serde_json::json!({"delay":82,"future":"untouched"}),
        );
        view.read_with(cx, |view, _| {
            assert!(view.karing.providers.is_empty());
            assert_eq!(view.karing.message, "1 成功 · 0 失败");
            assert_eq!(view.karing.delays, ["FR/中文: 82 ms"]);
        });
        let results = cx.debug_bounds("karing-delay-results").unwrap();
        cx.simulate_click(results.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("karing-delay-list").is_some());
        assert!(
            cx.debug_bounds("karing-panel").is_some(),
            "showing results must keep panel open"
        );
    }

    #[gpui::test]
    fn offscreen_karing_refresh_uses_provider_and_http_failure_is_not_success(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open(cx);
        for request in requests(&mut peer) {
            reply(
                &view,
                cx,
                &request,
                200,
                if request.path == "/providers/proxies" {
                    serde_json::json!({"providers":{"M4/测试":{"vehicleType":"HTTP","proxies":[{"name":"node-a","type":"VLESS"}]}}})
                } else {
                    serde_json::json!({"proxies":{}})
                },
            );
        }
        let refresh = cx.debug_bounds("karing-refresh-subscription").unwrap();
        cx.simulate_click(refresh.center(), Modifiers::default());
        let refresh = requests(&mut peer);
        assert_eq!(refresh.len(), 1);
        assert_eq!(refresh[0].method, "PUT");
        assert_eq!(
            refresh[0].path,
            "/providers/proxies/M4%2F%E6%B5%8B%E8%AF%95"
        );
        reply(
            &view,
            cx,
            &refresh[0],
            404,
            serde_json::json!({"message":"unsupported"}),
        );
        assert!(
            requests(&mut peer).is_empty(),
            "must not retry rejected writes"
        );
        view.read_with(cx, |view, _| {
            assert_eq!(view.karing.message, "Karing 返回 HTTP 404")
        });
        // A subsequent explicit refresh must start a fresh read batch, even
        // after a previous error; neither old flags nor errors may leak in.
        view.update(cx, |view, cx| view.refresh_karing_subscription(cx));
        let retry = requests(&mut peer);
        assert_eq!(retry.len(), 1);
        reply(&view, cx, &retry[0], 204, Value::Null);
        let reads = requests(&mut peer);
        assert_eq!(reads.len(), 2);
        let provider = reads
            .iter()
            .find(|r| r.path == "/providers/proxies")
            .unwrap();
        reply(
            &view,
            cx,
            provider,
            200,
            serde_json::json!({"providers":{"M4/测试":{"vehicleType":"HTTP","proxies":[]}}}),
        );
        view.read_with(cx, |view, _| {
            assert_eq!(view.karing.message, "读取中…");
            assert!(!view.karing.nodes_loaded);
            assert!(view.karing.load_error.is_none());
        });
        let proxies = reads.iter().find(|r| r.path == "/proxies").unwrap();
        reply(&view, cx, proxies, 200, serde_json::json!({"proxies":{}}));
        view.read_with(cx, |view, _| assert_eq!(view.karing.message, "配置已刷新"));
    }

    #[gpui::test]
    fn offscreen_karing_read_only_sends_no_requests_and_close_cancels(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, true);
        enable(&view, cx);
        open(cx);
        assert!(requests(&mut peer).is_empty());
        view.update(cx, |view, cx| {
            view.req.view_only = false;
            view.refresh_karing(cx);
        });
        assert_eq!(requests(&mut peer).len(), 2);
        // Escape dismisses Popover and invokes the production close callback.
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        let sent = std::iter::from_fn(|| peer.commands.try_recv().ok()).collect::<Vec<_>>();
        assert!(
            sent.iter()
                .any(|c| matches!(c, SessionCommand::App(AppCommand::CancelKaring)))
        );
        view.read_with(cx, |view, _| assert!(view.karing.pending.is_empty()));
    }

    #[gpui::test]
    fn offscreen_karing_more_configs_remain_accessible_with_long_names(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open(cx);
        for request in requests(&mut peer) {
            reply(
                &view,
                cx,
                &request,
                200,
                if request.path == "/providers/proxies" {
                    serde_json::json!({"providers":{
                        "A very long subscription name":{"vehicleType":"HTTP","proxies":[]},
                        "B very long subscription name":{"vehicleType":"HTTP","proxies":[]},
                        "C very long subscription name":{"vehicleType":"HTTP","proxies":[]}
                    }})
                } else {
                    serde_json::json!({"proxies":{}})
                },
            );
        }
        let panel = cx.debug_bounds("karing-panel").unwrap();
        let more = cx.debug_bounds("karing-more").unwrap();
        assert!(more.left() >= panel.left() && more.right() <= panel.right());
        let refresh = cx.debug_bounds("karing-refresh-subscription").unwrap();
        assert!(
            more.right() <= refresh.left(),
            "more button must not clip into refresh"
        );
        cx.simulate_click(more.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let third = cx
            .debug_bounds("karing-more-provider-C very long subscription name")
            .unwrap();
        cx.simulate_click(third.center(), Modifiers::default());
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.karing.selected.as_deref(),
                Some("C very long subscription name")
            )
        });
    }

    #[gpui::test]
    fn offscreen_karing_control_loss_stops_delay_queue_and_preserves_load_errors(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open(cx);
        let initial = requests(&mut peer);
        let proxies = initial.iter().find(|r| r.path == "/proxies").unwrap();
        let providers = initial
            .iter()
            .find(|r| r.path == "/providers/proxies")
            .unwrap();
        reply(
            &view,
            cx,
            proxies,
            503,
            serde_json::json!({"message":"not ready"}),
        );
        reply(
            &view,
            cx,
            providers,
            200,
            serde_json::json!({"providers":{}}),
        );
        view.read_with(cx, |view, _| {
            assert_eq!(view.karing.message, "Karing 返回 HTTP 503")
        });
        view.update(cx, |view, cx| {
            view.karing.nodes = vec!["a".into(), "b".into()];
            view.test_karing(cx);
        });
        let delay = requests(&mut peer);
        assert_eq!(delay.len(), 1);
        view.update(cx, |view, cx| {
            view.apply_app_event(
                AppEvent::Failed {
                    command: AppCommand::Karing(delay[0].clone()),
                    message: "控制连接已停止".into(),
                },
                cx,
            );
            let mut caps = view.apps.caps.clone().unwrap();
            caps.zx_control = None;
            view.apply_app_event(AppEvent::Capabilities(caps), cx);
        });
        assert!(
            requests(&mut peer).is_empty(),
            "must not continue testing remaining nodes"
        );
        view.read_with(cx, |view, _| {
            assert!(view.karing.delay_queue.is_empty());
            assert!(!view.karing_allowed());
            assert!(!view.karing.reachable);
        });
    }
}
