use super::*;
use crate::{AddressBook, Connection as Device, StorePaths};

fn fixture() -> (WorkspaceStore, AppId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    let mut book = AddressBook::load(paths.clone()).unwrap();
    let device = Device::new("phone", "phone", 5900);
    book.upsert(device.clone());
    book.save().unwrap();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    store.sync_backups(device.id,"lt.manodrabuziai.fr",&json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"idfv":id.as_str(),"backup_id":"snapshot:1","kind":"snapshot"}]})).unwrap();
    (store, id, dir)
}
fn queued(store: &mut WorkspaceStore, id: &AppId, repeat: bool, attempts: u32) -> AppTask {
    let timestamp = now() - 2;
    let cron = if repeat {
        "* * * * * * *".into()
    } else {
        task_cron_at(timestamp).unwrap()
    };
    let mut spec = TaskSpec::new("products", cron);
    spec.max_attempts = attempts;
    store.enqueue_task_at(id, &spec, timestamp).unwrap()
}
#[test]
fn cron_alone_determines_finite_and_repeated_schedules() {
    let time = Local
        .with_ymd_and_hms(2030, 10, 27, 9, 30, 0)
        .single()
        .unwrap()
        .timestamp();
    let cron = task_cron_at(time).unwrap();
    assert_eq!(next_task_time(&cron, time - 1).unwrap(), Some(time));
    assert_eq!(next_task_time(&cron, time).unwrap(), None);
    assert_eq!(
        next_task_time("0,10 30 9 27 10 * 2030", time).unwrap(),
        Some(time + 10)
    );
    assert_eq!(
        next_task_time("0,10 30 9 27 10 * 2030", time + 10).unwrap(),
        None
    );
    assert!(next_task_time("0 30 9 * * *", time).unwrap().unwrap() > time);
    assert!(next_task_time("not cron", time).is_err());
}
#[test]
fn one_row_is_reused_and_latest_result_is_retained() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 1);
    assert_eq!(store.claim_task_when(&task, |_| Ok(())).unwrap(), task.id);
    store
        .finish_task(&task.id, "completed", &json!({"count":6}))
        .unwrap();
    let saved = store.task(&task.id).unwrap();
    assert_eq!(saved.status, "pending");
    assert_eq!(saved.last_run_status.as_deref(), Some("completed"));
    assert!(saved.next_run_at.unwrap() > now() - 1);
    assert_eq!(store.tasks(&id).unwrap().len(), 1);
    assert_eq!(saved.result.unwrap()["count"], 6);
    assert_eq!(saved.attempt_count, 0);
}
#[test]
fn finite_cron_finishes_without_another_row() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, false, 1);
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    store
        .finish_task(&task.id, "completed", &json!({}))
        .unwrap();
    let done = store.task(&task.id).unwrap();
    assert_eq!(done.status, "completed");
    assert!(done.next_run_at.is_none());
}
#[test]
fn retries_are_bounded_and_unknown_writes_never_repeat() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, false, 2);
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    let policy = TaskFinishPolicy {
        retryable: true,
        stop_on_failure: false,
    };
    store
        .finish_task_with_policy(&task.id, "unknown", &json!({"error":"timeout"}), policy)
        .unwrap();
    let pending = store.task(&task.id).unwrap();
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.attempt_count, 1);
    assert!(pending.next_run_at.unwrap() >= now() + 29);
    store
        .db
        .execute(
            "UPDATE tasks SET next_run_at=?2 WHERE id=?1",
            params![task.id, now() - 1],
        )
        .unwrap();
    let pending = store.task(&task.id).unwrap();
    store.claim_task_when(&pending, |_| Ok(())).unwrap();
    store
        .finish_task_with_policy(&task.id, "unknown", &json!({}), policy)
        .unwrap();
    assert_eq!(store.task(&task.id).unwrap().status, "unknown");
    let task = queued(&mut store, &id, true, 1);
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    store.finish_task(&task.id, "unknown", &json!({})).unwrap();
    assert!(store.task(&task.id).unwrap().next_run_at.is_some());
}
#[test]
fn cancellation_archive_and_stale_snapshots_prevent_claims() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 1);
    store.cancel_task(&task.id).unwrap();
    assert!(store.claim_task_when(&task, |_| Ok(())).is_err());
    let task = queued(&mut store, &id, true, 1);
    store.reschedule_task(&task.id, "0 0 0 * * *").unwrap();
    assert!(store.claim_task_when(&task, |_| Ok(())).is_err());
    let mut app = store.load(&id).unwrap();
    app.archived = true;
    store.save(&app).unwrap();
    assert!(store.due_tasks(now()).unwrap().is_empty());
    assert_eq!(store.task(&task.id).unwrap().status, "cancelled");
}
#[test]
fn restart_keeps_pending_and_reschedules_interrupted_recurring_work() {
    let (mut store, id, dir) = fixture();
    let pending = queued(&mut store, &id, false, 1);
    let running = queued(&mut store, &id, true, 1);
    store.claim_task_when(&running, |_| Ok(())).unwrap();
    store
        .db
        .execute("UPDATE tasks SET deadline=0 WHERE id=?1", [&running.id])
        .unwrap();
    drop(store);
    let mut store = WorkspaceStore::open(&StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(
        store
            .due_tasks(now())
            .unwrap()
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        [pending.id]
    );
    let recovered=store.task(&running.id).unwrap();assert_eq!(recovered.status,"pending");assert!(recovered.next_run_at.is_some());assert_eq!(recovered.last_run_status.as_deref(),Some("unknown"));
}
#[test]
fn schema_has_no_classification_timezone_or_execution_details() {
    let (store, _, _) = fixture();
    let mut stmt = store.db.prepare("PRAGMA table_info(tasks)").unwrap();
    let fields = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for unwanted in ["timezone", "schedule_type", "max_runs", "run_count"] {
        assert!(!fields.iter().any(|field| field == unwanted));
    }
    let legacy: bool = store
        .db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='app_tasks')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!legacy);
}
#[test]
fn short_request_budget_is_not_recovered_before_result_storage() {
    let (mut store, id, _dir) = fixture();
    let mut spec = TaskSpec::new("session", "* * * * * * *");
    spec.timeout_seconds = 1;
    let task = store.enqueue_task(&id, &spec).unwrap();
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    let active = store.task(&task.id).unwrap();
    assert!(active.deadline.unwrap() >= active.started_at.unwrap() + 16);
    store.due_tasks(active.started_at.unwrap() + 1).unwrap();
    assert_eq!(store.task(&task.id).unwrap().status, "running");
    store
        .finish_task(&task.id, "completed", &json!({"saved":true}))
        .unwrap();
    assert_eq!(
        store.task(&task.id).unwrap().last_run_status.as_deref(),
        Some("completed")
    );
}
#[test]
fn preflight_failure_advances_cron_but_cannot_replace_an_edited_schedule() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 1);
    store.fail_pending_task(&task, "missing auth").unwrap();
    let updated = store.task(&task.id).unwrap();
    assert_eq!(updated.status, "pending");
    assert_eq!(updated.attempt_count, 0);
    assert_eq!(updated.last_run_status.as_deref(), Some("failed"));
    store.reschedule_task(&task.id, "0 0 0 * * *").unwrap();
    let edited = store.task(&task.id).unwrap();
    assert!(store.fail_pending_task(&updated, "stale callback").is_err());
    assert_eq!(store.task(&task.id).unwrap(), edited);
}

#[test]
fn preflight_failure_restores_attempt_budget_for_next_cron_occurrence() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 2);
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    store
        .finish_task_with_policy(
            &task.id,
            "failed",
            &json!({"http_status":503}),
            TaskFinishPolicy {
                retryable: true,
                stop_on_failure: false,
            },
        )
        .unwrap();
    let retry = store.task(&task.id).unwrap();
    assert_eq!(retry.attempt_count, 1);
    store.fail_pending_task(&retry, "missing auth").unwrap();
    let next = store.task(&task.id).unwrap();
    assert_eq!(next.attempt_count, 0);
    assert_eq!(next.status, "pending");
    store
        .db
        .execute(
            "UPDATE tasks SET next_run_at=?2 WHERE id=?1",
            params![task.id, now() - 1],
        )
        .unwrap();
    let next = store.task(&task.id).unwrap();
    store.claim_task_when(&next, |_| Ok(())).unwrap();
    assert_eq!(store.task(&task.id).unwrap().attempt_count, 1);
    store
        .finish_task_with_policy(
            &task.id,
            "failed",
            &json!({"http_status":503}),
            TaskFinishPolicy {
                retryable: true,
                stop_on_failure: false,
            },
        )
        .unwrap();
    let retried = store.task(&task.id).unwrap();
    assert_eq!(retried.attempt_count, 1);
    assert!(retried.next_run_at.unwrap() >= now() + 29);
}

#[test]
fn workflow_dependencies_gate_dispatch_and_stable_keys_survive_cancellation() {
    let (mut store, id, _dir) = fixture();
    let time = now();
    let spec = TaskSpec::new("session", "0 */30 * * * *");
    let parent = store
        .ensure_task(&id, "profile", &spec, None, time)
        .unwrap();
    let child = store
        .ensure_task(
            &id,
            "products",
            &TaskSpec::new("products", "0 */15 * * * *"),
            Some(&parent.id),
            time,
        )
        .unwrap();
    let due = store.due_tasks(time).unwrap();
    assert_eq!(due.iter().map(|t| &t.id).collect::<Vec<_>>(), [&parent.id]);
    assert!(store.claim_task_when(&child, |_| Ok(())).is_err());
    let claimed = store.claim_task_when(&parent, |_| Ok(())).unwrap();
    store
        .finish_task(&claimed, "completed", &json!({"ok":true}))
        .unwrap();
    assert!(
        store
            .due_tasks(time)
            .unwrap()
            .iter()
            .any(|t| t.id == child.id)
    );
    store.cancel_task(&child.id).unwrap();
    let repeated = store
        .ensure_task(
            &id,
            "products",
            &TaskSpec::new("products", "* * * * * *"),
            Some(&parent.id),
            time,
        )
        .unwrap();
    assert_eq!(repeated.id, child.id);
    assert_eq!(repeated.status, "cancelled");
    assert_eq!(repeated.cron_expression, "0 */15 * * * *");
    let resumed = store.run_task_now(&child.id).unwrap();
    assert_eq!(resumed.cron_expression, child.cron_expression);
    assert_eq!(resumed.status, "pending");
    assert_eq!(store.query_tasks("products").unwrap().len(), 1);
    assert_eq!(store.query_tasks("not-present").unwrap().len(), 0);
}

#[test]
fn stopped_tasks_keep_user_cron_and_running_completion_cannot_restart_them() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 2);
    let running = store.claim_task_when(&task, |_| Ok(())).unwrap();
    store.set_task_enabled(&running, false).unwrap();
    store
        .finish_task(&running, "completed", &json!({}))
        .unwrap();
    let stopped = store.task(&running).unwrap();
    assert!(!stopped.enabled);
    assert_eq!(stopped.next_run_at, None);
    assert!(store.due_tasks(now() + 1000).unwrap().is_empty());
    let changed = store.reschedule_task(&running, "0 */20 * * * *").unwrap();
    assert!(!changed.enabled);
    assert_eq!(changed.next_run_at, None);
    let restarted = store.set_task_enabled(&running, true).unwrap();
    assert!(restarted.enabled);
    assert_eq!(restarted.status, "pending");
    assert_eq!(restarted.cron_expression, "0 */20 * * * *");
    assert!(restarted.next_run_at.unwrap() <= now());
    let paused = store.set_task_enabled(&running, false).unwrap();
    assert!(store.claim_task_when(&paused, |_| Ok(())).is_err());
}

fn previous_task_schema() -> String {
    include_str!("../tasks.sql")
        .replace("    task_key TEXT,\n","")
        .replace("    depends_on TEXT REFERENCES tasks(id),\n","")
        .replace("    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),\n","")
        .replace("CREATE UNIQUE INDEX IF NOT EXISTS tasks_key ON tasks(app_id,task_key) WHERE task_key IS NOT NULL;\n","")
}
#[test]
fn opening_version_nine_reinitializes_empty_task_schema_and_preserves_apps() {
    let (store, id, dir) = fixture();
    let paths = StorePaths::in_dir(dir.path().into());
    let before = store.load(&id).unwrap();
    store.db.execute_batch("DROP TABLE tasks;").unwrap();
    store.db.execute_batch(&previous_task_schema()).unwrap();
    store.db.pragma_update(None, "user_version", 9).unwrap();
    drop(store);
    let reopened = WorkspaceStore::open(&paths).unwrap();
    assert!(reopened.configured_tasks(&id).unwrap().is_empty());
    assert!(reopened.query_tasks("").unwrap().is_empty());
    assert_eq!(reopened.load(&id).unwrap().title, before.title);
    let version: i64 = reopened
        .db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 15);
}
#[test]
fn incompatible_nonempty_task_schema_is_not_deleted() {
    let (store, id, dir) = fixture();
    let paths = StorePaths::in_dir(dir.path().into());
    store.db.execute_batch("DROP TABLE tasks;").unwrap();
    store.db.execute_batch(&previous_task_schema()).unwrap();
    store.db.execute("INSERT INTO tasks(id,app_id,kind,cron_expression,next_run_at,created_at,updated_at) VALUES('existing',?1,'products','* * * * * *',1,1,1)",[id.as_str()]).unwrap();
    store.db.pragma_update(None, "user_version", 9).unwrap();
    drop(store);
    assert!(WorkspaceStore::open(&paths).is_err());
    let db = rusqlite::Connection::open(&paths.database).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM tasks WHERE id='existing'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
}

#[test]
fn task_list_can_find_shop_nickname_without_changing_backup_title() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 1);
    let mut app = store.load(&id).unwrap();
    let backup_title = app.title.clone();
    app.metadata = json!({"shop":{"login":"ShopNickname"}}).to_string();
    store.save(&app).unwrap();
    let rows = store.query_tasks("shopnickname").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].task.id, task.id);
    assert_eq!(store.load(&id).unwrap().title, backup_title);
}

#[test]
fn task_transitions_publish_before_store_is_dropped() {
    use crate::events::*;
    let (mut store, id, dir) = fixture();
    let paths = StorePaths::in_dir(dir.path().into());
    let events = workspace_bus(&paths)
        .unwrap()
        .subscribe::<WorkspaceChanged>();
    let task = queued(&mut store, &id, false, 1);
    assert!(
        events
            .try_recv()
            .unwrap()
            .changes
            .iter()
            .any(|c| c.task_id.as_deref() == Some(task.id.as_str()))
    );
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    assert!(
        events
            .try_recv()
            .unwrap()
            .changes
            .iter()
            .any(|c| c.domain == ChangeDomain::Tasks)
    );
    assert_eq!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .task(&task.id)
            .unwrap()
            .status,
        "running"
    );
    store
        .finish_task(&task.id, "completed", &json!({"count":3}))
        .unwrap();
    assert!(
        events
            .try_recv()
            .unwrap()
            .changes
            .iter()
            .any(|c| c.task_id.as_deref() == Some(task.id.as_str()))
    );
    assert_eq!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .task(&task.id)
            .unwrap()
            .result
            .unwrap()["count"],
        3
    );
}

#[test]
fn recurring_errors_and_interruption_keep_scheduling_until_user_stops() {
    let (mut store, id, _dir) = fixture();
    let task = queued(&mut store, &id, true, 1);
    store.claim_task_when(&task, |_| Ok(())).unwrap();
    store
        .finish_task_with_policy(
            &task.id,
            "failed",
            &json!({"http_status":401}),
            TaskFinishPolicy {
                retryable: false,
                stop_on_failure: true,
            },
        )
        .unwrap();
    let failed = store.task(&task.id).unwrap();
    assert!(failed.enabled);
    assert_eq!(failed.status, "pending");
    assert!(failed.next_run_at.is_some());
    let retry = store.run_task_now(&task.id).unwrap();
    store.claim_task_when(&retry, |_| Ok(())).unwrap();
    store
        .db
        .execute("UPDATE tasks SET deadline=0 WHERE id=?1", [&task.id])
        .unwrap();
    store.due_tasks(now()).unwrap();
    let recovered = store.task(&task.id).unwrap();
    assert!(recovered.enabled);
    assert_eq!(recovered.status, "pending");
    assert!(recovered.next_run_at.is_some());
    store.set_task_enabled(&task.id, false).unwrap();
    store.due_tasks(now() + 3600).unwrap();
    let stopped = store.task(&task.id).unwrap();
    assert!(!stopped.enabled);
    assert!(stopped.next_run_at.is_none());
}

#[test]
fn enabled_bootstrap_retries_after_errors_and_finishes_once_after_success() {
    let (mut store,id,_dir)=fixture();let timestamp=now()+10;
    let spec=TaskSpec::new("session",task_cron_at(timestamp).unwrap());
    let task=store.ensure_task(&id,"profile",&spec,None,timestamp).unwrap();
    let ready=store.run_task_now(&task.id).unwrap();store.claim_task_when(&ready,|_|Ok(())).unwrap();
    store.finish_task_with_policy(&task.id,"failed",&json!({"http_status":401}),TaskFinishPolicy{retryable:false,stop_on_failure:true}).unwrap();
    let failed=store.task(&task.id).unwrap();assert!(failed.enabled);assert_eq!(failed.status,"pending");assert!(failed.next_run_at.is_some());
    let ready=store.run_task_now(&task.id).unwrap();store.claim_task_when(&ready,|_|Ok(())).unwrap();store.finish_task(&task.id,"completed",&json!({})).unwrap();
    let done=store.task(&task.id).unwrap();assert_eq!(done.status,"completed");assert!(done.next_run_at.is_none());
}
