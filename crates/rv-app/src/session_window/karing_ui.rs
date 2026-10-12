use super::*;
use gpui_component::button::ButtonCustomVariant;
use rv_session::{AppCommand, KaringRequest, KaringResponse, karing_component};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone)]
struct Provider {
    name: String,
    nodes: Vec<String>,
    refreshable: bool,
}
enum Pending {
    Providers,
    Proxies,
    Refresh,
    Delay(String),
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
fn compact_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(theme::toolbar_fg())
                .hover(rgb(0x35404e).into())
                .active(rgb(0x3b4654).into()),
        )
        .bg(rgb(0x29313b))
        .small()
        .h(px(24.))
        .px_2()
        .label(label)
}
impl SessionView {
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
        self.karing_request(Pending::Providers, "GET", "/providers/proxies".into());
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
        if let Pending::Delay(node) = kind {
            self.karing.delays.push(format!("{node}: {message}"));
            self.karing.delay_queue.clear();
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
            Pending::Providers => {
                if let Ok(value) = value
                    && let Some(providers) = value["providers"].as_object()
                {
                    self.karing.providers = providers
                        .iter()
                        .map(|(name, p)| Provider {
                            name: name.clone(),
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
                compact_button(
                    SharedString::from(format!("karing-provider-{name}")),
                    format!("{} · {}", provider.name, provider.nodes.len()),
                    cx,
                )
                .w(px(if self.karing.providers.len() > 2 {
                    72.
                } else {
                    86.
                }))
                .overflow_hidden()
                .when(selected, |b| b.bg(rgb(0x17463f)).text_color(rgb(0x3cd7b4)))
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
                compact_button(
                    "karing-more",
                    format!("+{}", self.karing.providers.len() - 2),
                    cx,
                )
                .w(px(28.))
                .px_0()
                .debug_selector(|| "karing-more".into())
                .disabled(!enabled)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.karing.show_providers = !this.karing.show_providers;
                    cx.notify();
                })),
            );
        }
        if self.karing.providers.is_empty() {
            chips = chips.child(div().text_color(theme::toolbar_muted()).child(
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
            .text_size(px(12.))
            .child(
                h_flex()
                    .h(px(22.))
                    .items_center()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Karing"))
                    .child(div().size(px(6.)).rounded_full().bg(
                        if self.karing.reachable && self.karing_allowed() {
                            rgb(0x3cd7b4)
                        } else {
                            rgb(0x7e8998)
                        },
                    ))
                    .child(div().text_color(theme::toolbar_muted()).child(status)),
            )
            .child(
                h_flex()
                    .h(px(26.))
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .w(px(32.))
                            .text_color(theme::toolbar_muted())
                            .child("配置"),
                    )
                    .child(chips)
                    .child(
                        compact_button("karing-refresh-subscription", "↻", cx)
                            .w(px(24.))
                            .px_0()
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
                        compact_button("karing-test", "ϟ", cx)
                            .w(px(24.))
                            .px_0()
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
                                compact_button(
                                    SharedString::from(p.name.clone()),
                                    p.name.clone(),
                                    cx,
                                )
                                .w_full()
                                .debug_selector({
                                    let name = p.name.clone();
                                    move || format!("karing-more-provider-{name}")
                                })
                                .disabled(!enabled)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.karing.selected = Some(p.name.clone());
                                        this.karing.show_providers = false;
                                        cx.notify();
                                    },
                                ))
                            }),
                        )),
                )
            })
            .when_some(self.karing.current_node.clone(), |panel, node| {
                panel.child(
                    h_flex()
                        .h(px(22.))
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .w(px(32.))
                                .text_color(theme::toolbar_muted())
                                .child("当前"),
                        )
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
            .child(
                h_flex()
                    .h(px(26.))
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(32.))
                            .text_color(theme::toolbar_muted())
                            .child("链式"),
                    )
                    .child(
                        compact_button("karing-chain-toggle", "○", cx)
                            .disabled(true)
                            .tooltip("Karing 当前管理接口未提供链式代理配置"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(theme::toolbar_muted())
                            .child("未提供"),
                    )
                    .child(
                        compact_button("karing-chain-order", "⇅", cx)
                            .disabled(true)
                            .tooltip("Karing 当前管理接口未提供链式顺序修改"),
                    ),
            )
            .child(
                h_flex()
                    .border_t_1()
                    .border_color(theme::toolbar_line())
                    .pt_1()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(11.))
                            .text_color(theme::toolbar_muted())
                            .child(if self.karing.message.is_empty() {
                                "等待配置".to_string()
                            } else {
                                self.karing.message.clone()
                            }),
                    )
                    .child(
                        compact_button("karing-delay-results", "结果", cx)
                            .debug_selector(|| "karing-delay-results".into())
                            .disabled(self.karing.delays.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.karing.show_results = !this.karing.show_results;
                                cx.notify();
                            })),
                    )
                    .child(
                        compact_button("karing-refresh-nodes", "刷新节点", cx)
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
                                    .map(|result| div().text_size(px(11.)).child(result)),
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
                panel.size.width <= px(320.) && panel.size.height <= px(170.),
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
            panel.size.width <= px(320.) && panel.size.height <= px(170.),
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
