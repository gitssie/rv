//! Console diagnostics contain operation metadata, never request/result payloads.
use super::{FunctionEvent, uncertain_result, service_error_code};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct OperationLog {
    last: Value,
}

fn details(record: &Value) -> Value {
    json!({
        "operation_id": record["operation_id"].as_str(),
        "method": record["method"].as_str(),
        "status": record["status"].as_str(),
        "phase": record["progress"]["phase"].as_str(),
        "stage": record["progress"]["stage"].as_str(),
        "error_domain": record["error"]["domain"].as_str(),
        "error_code": record["error"]["code"].as_i64().map(Value::from)
            .or_else(|| record["error"]["code"].as_str().map(Value::from)),
        "service_error_code": service_error_code(record),
        "outcome_unknown": record["error"]["outcome_unknown"].as_bool(),
        "reason": record["error"]["reason"].as_str(),
        "message": record["error"]["message"].as_str(),
        "source_domain": record["error"]["source_domain"].as_str(),
        "source_code": record["error"]["source_code"].as_i64(),
        "posix_code": record["error"]["posix_code"].as_i64(),
        "state_may_be_partial": record["error"]["state_may_be_partial"].as_bool(),
        "archive_may_be_partial": record["error"]["archive_may_be_partial"].as_bool(),
    })
}

impl OperationLog {
    pub(super) fn record(&mut self, action: &str, event: &FunctionEvent) {
        if !action.starts_with("app-") {
            return;
        }
        match event {
            FunctionEvent::State { task: 32, record } => {
                let next = details(record);
                let changed = self.last != next;
                self.last = next;
                if record["status"] == "completed" {
                    if record.get("error").is_some() || uncertain_result(32, record) {
                        tracing::error!(action, details = %self.last, "ZXTouch App-state operation failed");
                    } else {
                        tracing::info!(action, details = %self.last, "ZXTouch App-state operation completed");
                    }
                    self.last = Value::Null;
                } else if changed {
                    tracing::info!(action, details = %self.last, "ZXTouch App-state operation stage");
                }
            }
            FunctionEvent::Error(message) | FunctionEvent::Uncertain(message) => {
                tracing::error!(action, details = %self.last, message = %message,
                    outcome_unknown = matches!(event, FunctionEvent::Uncertain(_)),
                    "ZXTouch App-state request failed");
                self.last = Value::Null;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Console(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Console {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn native_failure_and_transport_interruption_are_written_to_console() {
        let console = Console::default();
        let writer = console.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let mut log = OperationLog::default();
            log.record(
                "app-delete",
                &FunctionEvent::State {
                    task: 32,
                    record: json!({
                        "operation_id":"delete-1", "method":"delete", "status":"completed",
                        "error":{"domain":"TVAppStateServices","code":2,"posix_code":13},
                        "result":{"token":"do-not-log"}
                    }),
                },
            );
            log.record(
                "app-switch",
                &FunctionEvent::State {
                    task: 32,
                    record: json!({
                        "operation_id":"switch-2", "method":"switch", "status":"running",
                        "progress":{"phase":"restore", "stage":"copy"}
                    }),
                },
            );
            log.record(
                "app-switch",
                &FunctionEvent::Uncertain("手机 HTTP 回复中断".into()),
            );
        });
        let output = String::from_utf8(console.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("ERROR"));
        assert!(output.contains("delete-1") && output.contains("TVAppStateServices"));
        assert!(output.contains("switch-2") && output.contains("restore"));
        assert!(output.contains("手机 HTTP 回复中断"));
        assert!(!output.contains("do-not-log"));
    }

    #[test]
    fn error_diagnostics_keep_correlation_and_cause_without_payloads() {
        let record = json!({"operation_id":"op-1","method":"delete","status":"completed",
            "progress":{"phase":"delete","stage":"files","secret":"hidden-progress"},
            "error":{"domain":"TVAppStateServices","code":2,"error_code":"snapshot_busy","reason":"store_busy","posix_code":13,
                "archive_may_be_partial":true,"message":"Cannot remove selected archive","token":"hidden-token"},
            "params":{"password":"hidden-password"},"result":{"access_token":"hidden-result"}});
        let safe = details(&record);
        assert_eq!(safe["operation_id"], "op-1");
        assert_eq!(safe["error_code"], 2);
        assert_eq!(safe["service_error_code"], "snapshot_busy");
        assert_eq!(safe["message"], "Cannot remove selected archive");
        assert_eq!(safe["posix_code"], 13);
        assert_eq!(safe["archive_may_be_partial"], true);
        assert!(!safe.to_string().contains("hidden-"));
    }
}
