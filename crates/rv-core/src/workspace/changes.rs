//! SQLite row changes are buffered per connection and published only after commit.
use crate::{StoreError, events::*};
use rusqlite::{Connection, OptionalExtension};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
type Rows = BTreeSet<(String, i64)>;
#[derive(Clone, Default)]
pub(super) struct ChangeTracker {
    rows: Arc<Mutex<Rows>>,
}
fn domain(table: &str) -> Option<ChangeDomain> {
    use ChangeDomain::*;
    Some(match table {
        "tasks" => Tasks,
        "app_instances"
        | "app_backups"
        | "app_proxy_bindings"
        | "socks_proxies"
        | "proxy_engine_settings" => Apps,
        "app_auth" | "app_states" | "app_request_sessions" | "app_token_sources" => Credentials,
        "app_execution" => Execution,
        "chat_conversations" => Conversations,
        "chat_messages" | "chat_outbox" | "chat_outbox_images" => Messages,
        "chat_customers" => Customers,
        "vinted_products" => Products,
        "vinted_orders" => Orders,
        "vinted_notifications" => Notifications,
        _ => return None,
    })
}
impl ChangeTracker {
    pub fn install(db: &Connection) -> Result<Self, StoreError> {
        let tracker = Self::default();
        let rows = tracker.rows.clone();
        db.update_hook(Some(move |_, database: &str, table: &str, row: i64| {
            if database == "main" && domain(table).is_some() {
                rows.lock().unwrap().insert((table.to_string(), row));
            }
        }))?;
        let rows = tracker.rows.clone();
        db.rollback_hook(Some(move || rows.lock().unwrap().clear()))?;
        Ok(tracker)
    }
    pub fn checkpoint(&self) -> Rows {
        self.rows.lock().unwrap().clone()
    }
    pub fn restore(&self, checkpoint: Rows) {
        *self.rows.lock().unwrap() = checkpoint;
    }
    pub fn publish(&self, db: &Connection, bus: &EventBus) {
        if !db.is_autocommit() {
            return;
        }
        let rows = std::mem::take(&mut *self.rows.lock().unwrap());
        let mut changes = BTreeSet::new();
        for (table, row) in rows {
            let Some(domain) = domain(&table) else {
                continue;
            };
            let mut change = EntityChange {
                domain,
                app_id: None,
                conversation_id: None,
                task_id: None,
            };
            // Deleted rows intentionally produce a domain-wide invalidation.
            // Queries run after commit, never from SQLite's update/commit hooks.
            let sql = match table.as_str() {
                "socks_proxies" => "SELECT NULL,NULL,NULL FROM socks_proxies WHERE rowid=?1",
                "proxy_engine_settings" => {
                    "SELECT NULL,NULL,NULL FROM proxy_engine_settings WHERE rowid=?1"
                }
                "tasks" => "SELECT app_id,NULL,id FROM tasks WHERE rowid=?1",
                "app_instances" => "SELECT id,NULL,NULL FROM app_instances WHERE rowid=?1",
                "chat_conversations" => {
                    "SELECT app_id,id,NULL FROM chat_conversations WHERE rowid=?1"
                }
                "chat_messages" => {
                    "SELECT c.app_id,c.id,NULL FROM chat_messages m JOIN chat_conversations c ON c.id=m.conversation_id WHERE m.rowid=?1"
                }
                "chat_outbox" => {
                    "SELECT c.app_id,c.id,NULL FROM chat_outbox q JOIN chat_conversations c ON c.id=q.conversation_id WHERE q.rowid=?1"
                }
                "chat_outbox_images" => {
                    "SELECT c.app_id,c.id,NULL FROM chat_outbox_images i JOIN chat_outbox q ON q.id=i.outbox_id JOIN chat_conversations c ON c.id=q.conversation_id WHERE i.rowid=?1"
                }
                _ => "",
            };
            let resolved = if sql.is_empty() {
                // Table names come only from the static allowlist above.
                db.query_row(
                    &format!("SELECT app_id,NULL,NULL FROM {table} WHERE rowid=?1"),
                    [row],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()
            } else {
                db.query_row(sql, [row], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .optional()
            };
            if let Ok(Some((app, conversation, task))) = resolved {
                change.app_id = app;
                change.conversation_id = conversation;
                change.task_id = task;
            }
            changes.insert(change);
        }
        if !changes.is_empty() {
            bus.post(WorkspaceChanged {
                changes: changes.into_iter().collect(),
            });
        }
    }
}
