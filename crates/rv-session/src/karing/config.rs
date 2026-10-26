//! User-selected exits and subscription-scoped relay selection.
//! Profile credentials stay in the phone response; the catalogue retains labels only.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KaringSubscription {
    pub id: String,
    pub name: String,
    pub nodes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KaringCatalogue {
    pub subscriptions: Vec<KaringSubscription>,
    pub nodes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KaringSelection {
    pub exit_tag: String,
    /// None clears chaining while retaining the chosen exit.
    pub upstream_group: Option<String>,
    pub relay_tag: Option<String>,
}

impl KaringCatalogue {
    /// Only shapes that this two-hop controller can restore without loss.
    pub fn saved_selection(&self, documents: &Value) -> Option<KaringSelection> {
        let exit = documents["karing_subscribe_use.json"]["select_default"].as_str()?;
        if !self.nodes.iter().any(|node| node == exit) {
            return None;
        }
        let settings = &documents["karing_setting.json"];
        let chain = settings["chain_proxy"].as_array()?;
        if chain.len() > 1 || (!chain.is_empty() && settings["front"].as_bool() != Some(true)) {
            return None;
        }
        let relay = match chain.first() {
            Some(value) => Some(value.as_str()?),
            None => None,
        };
        let group = match relay {
            Some(tag) if tag != exit => Some(
                self.subscriptions
                    .iter()
                    .find(|s| s.nodes.iter().any(|node| node == tag))?
                    .id
                    .clone(),
            ),
            Some(_) => return None,
            None => None,
        };
        Some(KaringSelection {
            exit_tag: exit.into(),
            relay_tag: relay.map(str::to_owned),
            upstream_group: group,
        })
    }
    pub fn from_profiles(documents: &Value) -> Result<Self, String> {
        let groups = documents["karing_subscribe.json"]["items"]
            .as_array()
            .ok_or("Karing 订阅返回格式不正确")?;
        let mut catalogue = Self::default();
        let mut tags = HashSet::new();
        let mut ids = HashSet::new();
        for group in groups {
            if group["enable"].as_bool() == Some(false) {
                continue;
            }
            let id = group["groupid"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("Karing 订阅缺少标识")?;
            if !ids.insert(id.to_owned()) {
                return Err("Karing 订阅标识重复".into());
            }
            let servers = group["servers"].as_array().ok_or("Karing 节点格式不正确")?;
            let mut nodes = Vec::new();
            for server in servers {
                let tag = server["tag"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("Karing 节点缺少名称")?;
                if !tags.insert(tag.to_owned()) {
                    return Err("Karing 节点名称重复，无法确定出口".into());
                }
                nodes.push(tag.to_owned());
                catalogue.nodes.push(tag.to_owned());
            }
            if !nodes.is_empty() {
                catalogue.subscriptions.push(KaringSubscription {
                    id: id.into(),
                    name: group["remark"].as_str().unwrap_or(id).into(),
                    nodes,
                });
            }
        }
        Ok(catalogue)
    }

    pub fn relay_candidates(&self, group: &str, exit: &str) -> Result<Vec<String>, String> {
        if !self.nodes.iter().any(|node| node == exit) {
            return Err("所选出口节点已不存在，请重新读取配置".into());
        }
        let subscription = self
            .subscriptions
            .iter()
            .find(|item| item.id == group)
            .ok_or("所选前置订阅已不存在，请重新读取配置")?;
        let nodes: Vec<_> = subscription
            .nodes
            .iter()
            .filter(|node| node.as_str() != exit)
            .cloned()
            .collect();
        if nodes.is_empty() {
            return Err("前置订阅没有可用的中介候选节点".into());
        }
        Ok(nodes)
    }

    pub fn select(
        &self,
        exit: &str,
        upstream: Option<&str>,
        delays: &HashMap<String, u64>,
    ) -> Result<KaringSelection, String> {
        if !self.nodes.iter().any(|node| node == exit) {
            return Err("所选出口节点已不存在，请重新读取配置".into());
        }
        let relay_tag = match upstream {
            None => None,
            Some(group) => {
                let candidates = self.relay_candidates(group, exit)?;
                Some(
                    candidates
                        .into_iter()
                        .filter_map(|node| {
                            delays
                                .get(&node)
                                .filter(|delay| **delay > 0)
                                .map(|delay| (*delay, node))
                        })
                        .min()
                        .map(|(_, node)| node)
                        .ok_or("前置订阅测速全部失败，未修改配置")?,
                )
            }
        };
        Ok(KaringSelection {
            exit_tag: exit.into(),
            upstream_group: upstream.map(str::to_owned),
            relay_tag,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalogue() -> KaringCatalogue {
        KaringCatalogue::from_profiles(&serde_json::json!({
            "karing_subscribe.json": {"items": [
                {"groupid":"subscription-a","remark":"任意订阅","enable":true,
                 "servers":[{"tag":"A"},{"tag":"B"},{"tag":"C"}]},
                {"groupid":"subscription-b","servers":[{"tag":"出口"}]},
                {"groupid":"disabled","enable":false,"servers":[{"tag":"不可用"}]}
            ]}
        }))
        .unwrap()
    }
    #[test]
    fn single_mode_accepts_every_available_node_and_clears_relay() {
        let c = catalogue();
        for exit in &c.nodes {
            assert_eq!(
                c.select(exit, None, &HashMap::new()).unwrap(),
                KaringSelection {
                    exit_tag: exit.clone(),
                    upstream_group: None,
                    relay_tag: None,
                }
            );
        }
        assert!(c.select("不可用", None, &HashMap::new()).is_err());
    }
    #[test]
    fn chain_uses_fastest_successful_candidate_in_selected_subscription() {
        let delays = HashMap::from([
            ("A".into(), 100),
            ("B".into(), 30),
            ("C".into(), 0),
            ("出口".into(), 1),
        ]);
        let selected = catalogue()
            .select("出口", Some("subscription-a"), &delays)
            .unwrap();
        assert_eq!(selected.relay_tag.as_deref(), Some("B"));
        assert_eq!(selected.exit_tag, "出口");
        let selected = catalogue()
            .select("B", Some("subscription-a"), &delays)
            .unwrap();
        assert_eq!(selected.relay_tag.as_deref(), Some("A"));
    }
    #[test]
    fn chain_failure_does_not_fall_back_to_another_subscription_or_exit() {
        let c = catalogue();
        assert!(
            c.select("出口", Some("subscription-a"), &HashMap::new())
                .is_err()
        );
        assert!(
            c.select("出口", Some("subscription-b"), &HashMap::new())
                .is_err()
        );
        assert!(c.select("出口", Some("removed"), &HashMap::new()).is_err());
    }
    #[test]
    fn ambiguous_tags_are_rejected() {
        assert!(KaringCatalogue::from_profiles(&serde_json::json!({
            "karing_subscribe.json":{"items":[{"groupid":"x","servers":[{"tag":"a"},{"tag":"a"}]}]}
        })).is_err());
    }
    #[test]
    fn restore_rejects_multihop_backward_and_unknown_choices() {
        let c = catalogue();
        for (chain, front, valid) in [
            (serde_json::json!([]), false, true),
            (serde_json::json!(["A"]), true, true),
            (serde_json::json!(["A", "B"]), true, false),
            (serde_json::json!(["A"]), false, false),
            (serde_json::json!(["missing"]), true, false),
            (serde_json::json!(["出口"]), true, false),
        ] {
            let documents = serde_json::json!({
                "karing_setting.json":{"chain_proxy":chain,"front":front},
                "karing_subscribe_use.json":{"select_default":"出口"}
            });
            assert_eq!(c.saved_selection(&documents).is_some(), valid);
        }
    }
}
