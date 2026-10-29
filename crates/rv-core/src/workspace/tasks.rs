//! One row per task. Cron and all clock arithmetic live in Rust, never SQL.
use super::*;
use crate::StoreError;
use chrono::{Datelike, Local, TimeZone, Timelike};
use cron::Schedule;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct TaskSpec {
    pub kind: String,
    pub payload: Value,
    pub cron_expression: String,
    pub expires_at: Option<i64>,
    pub max_attempts: u32,
    pub timeout_seconds: u32,
}
impl TaskSpec {
    pub fn new(kind: impl Into<String>, cron: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            cron_expression: cron.into(),
            payload: json!({}),
            expires_at: None,
            max_attempts: 1,
            timeout_seconds: 90,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct AppTask {
    pub id: String,
    pub app_id: String,
    pub kind: String,
    pub task_key: Option<String>,
    pub depends_on: Option<String>,
    pub payload: Value,
    pub status: String,
    pub enabled: bool,
    pub cron_expression: String,
    pub next_run_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub timeout_seconds: u32,
    pub deadline: Option<i64>,
    pub result: Option<Value>,
    pub last_run_status: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}
#[derive(Clone, Debug)]
pub struct TaskEntry {
    pub task: AppTask,
    pub app_title: String,
    pub dependency_kind: Option<String>,
    pub dependency_status: Option<String>,
}
#[derive(Clone, Copy, Default)]
pub struct TaskFinishPolicy {
    pub retryable: bool,
    pub stop_on_failure: bool,
}

/// Six fields (seconds first), or seven with an explicit year. Uses RV's local clock.
pub fn next_task_time(expression: &str, after: i64) -> Result<Option<i64>, StoreError> {
    if expression.len() > 256 || !matches!(expression.split_whitespace().count(), 6 | 7) {
        return Err(StoreError::Invalid(
            "cron 需要六段或七段：秒 分 时 日 月 星期 [年]".into(),
        ));
    }
    let schedule = Schedule::from_str(expression)
        .map_err(|_| StoreError::Invalid("cron 表达式无效".into()))?;
    let instant = Local
        .timestamp_opt(after, 0)
        .single()
        .ok_or_else(|| StoreError::Invalid("任务时间超出范围".into()))?;
    Ok(schedule.after(&instant).next().map(|time| time.timestamp()))
}
/// A concrete local date with its year is an ordinary cron with no later match.
pub fn task_cron_at(timestamp: i64) -> Result<String, StoreError> {
    let time = Local
        .timestamp_opt(timestamp, 0)
        .single()
        .ok_or_else(|| StoreError::Invalid("任务时间超出范围".into()))?;
    Ok(format!(
        "{} {} {} {} {} * {}",
        time.second(),
        time.minute(),
        time.hour(),
        time.day(),
        time.month(),
        time.year()
    ))
}
const COLUMNS: &str = "id,app_id,kind,payload,status,cron_expression,next_run_at,expires_at,attempt_count,max_attempts,timeout_seconds,deadline,result,last_run_status,error_code,error_message,created_at,updated_at,started_at,finished_at,task_key,depends_on,enabled";
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AppTask> {
    fn value(r: &rusqlite::Row<'_>, n: usize) -> rusqlite::Result<Option<Value>> {
        let text: Option<String> = r.get(n)?;
        text.map(|s| {
            serde_json::from_str(&s).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    n,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()
    }
    Ok(AppTask {
        id: r.get(0)?,
        app_id: r.get(1)?,
        kind: r.get(2)?,
        payload: value(r, 3)?.unwrap_or(Value::Null),
        status: r.get(4)?,
        cron_expression: r.get(5)?,
        next_run_at: r.get(6)?,
        expires_at: r.get(7)?,
        attempt_count: r.get(8)?,
        max_attempts: r.get(9)?,
        timeout_seconds: r.get(10)?,
        deadline: r.get(11)?,
        result: value(r, 12)?,
        last_run_status: r.get(13)?,
        error_code: r.get(14)?,
        error_message: r.get(15)?,
        created_at: r.get(16)?,
        updated_at: r.get(17)?,
        started_at: r.get(18)?,
        finished_at: r.get(19)?,
        task_key: r.get(20)?,
        depends_on: r.get(21)?,
        enabled: r.get(22)?,
    })
}
fn get(db: &Connection, id: &str) -> Result<AppTask, StoreError> {
    Ok(db.query_row(
        &format!("SELECT {COLUMNS} FROM tasks WHERE id=?1"),
        [id],
        row,
    )?)
}
fn recover(db: &Connection, timestamp: i64) -> Result<(), StoreError> {
    let mut query=db.prepare(&format!("SELECT {COLUMNS} FROM tasks WHERE (status='running' AND deadline<=?1) OR (enabled=1 AND status IN ('unknown','failed') AND next_run_at IS NULL)"))?;
    let interrupted = query
        .query_map([timestamp], row)?
        .collect::<Result<Vec<_>, _>>()?;
    drop(query);
    for task in interrupted {
        let next=if task.enabled {
            next_task_time(&task.cron_expression,timestamp)?.or_else(||task.task_key.as_ref().map(|_|timestamp+30))
        }else{None};
        let was_running = task.status == "running";
        if !was_running && next.is_none() {
            continue;
        }
        let state = if next.is_some() {
            "pending"
        } else if was_running {
            "unknown"
        } else {
            task.status.as_str()
        };
        db.execute("UPDATE tasks SET status=?2,last_run_status=CASE WHEN ?4 THEN 'unknown' ELSE last_run_status END,next_run_at=?3,deadline=NULL,finished_at=CASE WHEN ?4 THEN ?5 ELSE finished_at END,updated_at=?5,error_code=CASE WHEN ?4 THEN 'interrupted' ELSE error_code END,error_message=CASE WHEN ?4 THEN '执行中断' ELSE error_message END WHERE id=?1 AND status=?6 AND enabled=?7 AND deadline IS ?8 AND updated_at=?9 AND next_run_at IS ?10 AND attempt_count=?11 AND (status<>'running' OR deadline<=?5)",params![task.id,state,next,was_running,timestamp,task.status,task.enabled,task.deadline,task.updated_at,task.next_run_at,task.attempt_count])?;
    }
    db.execute("UPDATE tasks SET status='cancelled',next_run_at=NULL,finished_at=?1,updated_at=?1,error_code='expired',error_message='任务已过期' WHERE status='pending' AND expires_at IS NOT NULL AND expires_at<=?1",[timestamp])?;
    db.execute("UPDATE tasks SET status='cancelled',next_run_at=NULL,finished_at=?1,updated_at=?1,error_code='archived',error_message='虚拟 App 已归档' WHERE status='pending' AND app_id IN (SELECT id FROM app_instances WHERE archived=1)",[timestamp])?;
    Ok(())
}

impl WorkspaceStore {
    pub fn enqueue_task(&mut self, id: &AppId, spec: &TaskSpec) -> Result<AppTask, StoreError> {
        self.enqueue_task_at(id, spec, now())
    }
    fn enqueue_task_at(
        &mut self,
        id: &AppId,
        spec: &TaskSpec,
        timestamp: i64,
    ) -> Result<AppTask, StoreError> {
        if self.load(id)?.archived {
            return Err(StoreError::Invalid("虚拟 App 已归档".into()));
        }
        if spec.kind.is_empty()
            || spec.kind.len() > 64
            || !spec.payload.is_object()
            || spec.payload.to_string().len() > 256 * 1024
            || !(1..=10).contains(&spec.max_attempts)
            || !(1..=3600).contains(&spec.timeout_seconds)
        {
            return Err(StoreError::Invalid("任务参数无效".into()));
        }
        let next = next_task_time(&spec.cron_expression, timestamp.saturating_sub(1))?
            .ok_or_else(|| StoreError::Invalid("cron 没有后续执行时间".into()))?;
        if spec.kind == "set_avatar"
            && (spec.max_attempts != 1 || next_task_time(&spec.cron_expression, next)?.is_some())
        {
            return Err(StoreError::Invalid(
                "设置头像只支持一次性执行，不自动重试".into(),
            ));
        }
        if spec.expires_at.is_some_and(|end| end <= next) {
            return Err(StoreError::Invalid("执行时间不早于任务过期时间".into()));
        }
        let task = uuid::Uuid::new_v4().to_string();
        self.db.execute("INSERT INTO tasks(id,app_id,kind,payload,cron_expression,next_run_at,expires_at,max_attempts,timeout_seconds,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",params![task,id.as_str(),spec.kind,spec.payload.to_string(),spec.cron_expression,next,spec.expires_at,spec.max_attempts,spec.timeout_seconds,timestamp])?;
        self.task(&task)
    }
    /// Stable keys make provisioning a workflow idempotent, including after restart.
    /// The first execution can precede the recurring cron's first match.
    pub fn ensure_task(
        &mut self,
        id: &AppId,
        key: &str,
        spec: &TaskSpec,
        parent: Option<&str>,
        first_run: i64,
    ) -> Result<AppTask, StoreError> {
        self.db.execute_batch("SAVEPOINT ensure_task")?;
        let checkpoint = self.changes.checkpoint();
        let result = (|| {
            use rusqlite::OptionalExtension;
            if let Some(existing) = self
                .db
                .query_row(
                    "SELECT id FROM tasks WHERE app_id=?1 AND task_key=?2",
                    params![id.as_str(), key],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                return self.task(&existing);
            }
            if let Some(parent) = parent {
                if self.task(parent)?.app_id != id.as_str() {
                    return Err(StoreError::Invalid("前置任务不属于此 App".into()));
                }
            }
            let task = self.enqueue_task(id, spec)?;
            self.db.execute(
                "UPDATE tasks SET task_key=?2,depends_on=?3,next_run_at=?4 WHERE id=?1",
                params![task.id, key, parent, first_run],
            )?;
            self.task(&task.id)
        })();
        match result {
            Ok(task) => {
                self.db.execute_batch("RELEASE ensure_task")?;
                self.publish_changes();
                Ok(task)
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO ensure_task; RELEASE ensure_task")?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
    pub fn query_tasks(&self, query: &str) -> Result<Vec<TaskEntry>, StoreError> {
        let columns = COLUMNS
            .split(',')
            .map(|c| format!("t.{c}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut stmt=self.db.prepare(&format!("SELECT {columns},a.title,p.kind,coalesce(p.last_run_status,p.status) FROM tasks t JOIN app_instances a ON a.id=t.app_id LEFT JOIN tasks p ON p.id=t.depends_on WHERE instr(lower(a.title||' '||coalesce(json_extract(a.metadata,'$.shop.login'),'')||' '||t.app_id||' '||t.kind||' '||t.status||' '||coalesce(t.error_message,'')),lower(?1))>0 ORDER BY t.created_at DESC,t.rowid DESC LIMIT 500"))?;
        Ok(stmt
            .query_map([query.trim()], |r| {
                Ok(TaskEntry {
                    task: row(r)?,
                    app_title: r.get(23)?,
                    dependency_kind: r.get(24)?,
                    dependency_status: r.get(25)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
    pub fn task_blocker(
        &mut self,
        expected: &AppTask,
        code: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), StoreError> {
        if expected.error_code.as_deref() == code && expected.error_message.as_deref() == message {
            return Ok(());
        }
        self.db.execute("UPDATE tasks SET error_code=?2,error_message=?3,updated_at=?4 WHERE id=?1 AND status='pending'",params![expected.id,code,message,now()])?;
        self.publish_changes();
        Ok(())
    }
    pub fn run_task_now(&mut self, id: &str) -> Result<AppTask, StoreError> {
        let task = self.task(id)?;
        if task.status == "running" {
            return Err(StoreError::Invalid("任务正在执行".into()));
        }
        if self.load(&AppId::from_idfv(&task.app_id)?)?.archived {
            return Err(StoreError::Invalid("虚拟 App 已归档".into()));
        }
        let timestamp = now();
        if task.expires_at.is_some_and(|end| end <= timestamp) {
            return Err(StoreError::Invalid("任务已过期".into()));
        }
        let first = next_task_time(&task.cron_expression, task.created_at.saturating_sub(1))?;
        let single = first.is_some_and(|first| {
            next_task_time(&task.cron_expression, first)
                .ok()
                .flatten()
                .is_none()
        });
        let cron = if single {
            task_cron_at(timestamp)?
        } else {
            task.cron_expression
        };
        self.db.execute("UPDATE tasks SET status='pending',enabled=1,next_run_at=?2,cron_expression=?3,attempt_count=0,error_code=NULL,error_message=NULL,updated_at=?2 WHERE id=?1 AND status!='running'",params![id,timestamp,cron])?;
        self.task(id)
    }
    pub fn task(&self, id: &str) -> Result<AppTask, StoreError> {
        self.publish_changes();
        get(&self.db, id)
    }
    pub fn tasks(&self, id: &AppId) -> Result<Vec<AppTask>, StoreError> {
        Ok(self.db.prepare(&format!("SELECT {COLUMNS} FROM tasks WHERE app_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 50"))?.query_map([id.as_str()],row)?.collect::<Result<Vec<_>,_>>()?)
    }
    pub fn configured_tasks(&self, id: &AppId) -> Result<Vec<AppTask>, StoreError> {
        Ok(self.db.prepare(&format!("SELECT {COLUMNS} FROM tasks WHERE app_id=?1 AND task_key IS NOT NULL ORDER BY created_at,rowid"))?.query_map([id.as_str()],row)?.collect::<Result<Vec<_>,_>>()?)
    }
    pub fn due_tasks(&mut self, timestamp: i64) -> Result<Vec<AppTask>, StoreError> {
        recover(&self.db, timestamp)?;
        self.publish_changes();
        Ok(self.db.prepare(&format!("SELECT {COLUMNS} FROM tasks WHERE enabled=1 AND status='pending' AND next_run_at<=?1 AND (depends_on IS NULL OR EXISTS(SELECT 1 FROM tasks parent WHERE parent.id=tasks.depends_on AND parent.last_run_status='completed')) AND NOT EXISTS(SELECT 1 FROM tasks active WHERE active.app_id=tasks.app_id AND active.status='running') ORDER BY next_run_at,created_at,id"))?.query_map([timestamp],row)?.collect::<Result<Vec<_>,_>>()?)
    }
    pub fn cancel_task(&mut self, id: &str) -> Result<(), StoreError> {
        if self.db.execute("UPDATE tasks SET status='cancelled',enabled=0,next_run_at=NULL,updated_at=?2,finished_at=?2 WHERE id=?1 AND status='pending'",params![id,now()])?!=1 {return Err(StoreError::Conflict);}
        self.publish_changes();
        Ok(())
    }
    /// Stopping preserves the cron. An already sent operation finishes its current
    /// run, but completion cannot schedule another run while disabled.
    pub fn set_task_enabled(&mut self, id: &str, enabled: bool) -> Result<AppTask, StoreError> {
        let task = self.task(id)?;
        if enabled {
            if self.load(&AppId::from_idfv(&task.app_id)?)?.archived {
                return Err(StoreError::Invalid("虚拟 App 已归档".into()));
            }
            let timestamp = now();
            if task.status == "running" {
                self.db.execute(
                    "UPDATE tasks SET enabled=1,updated_at=?2 WHERE id=?1",
                    params![id, timestamp],
                )?;
            } else {
                // Explicit start/retry runs now, retaining the saved cron for
                // subsequent runs. Unknown results never auto-requeue themselves.
                return self.run_task_now(id);
            }
        } else {
            self.db.execute("UPDATE tasks SET enabled=0,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END,next_run_at=NULL,updated_at=?2 WHERE id=?1",params![id,now()])?;
        }
        self.task(id)
    }
    pub fn reschedule_task(&mut self, id: &str, cron: &str) -> Result<AppTask, StoreError> {
        let task = self.task(id)?;
        let timestamp = now();
        let next = next_task_time(cron, timestamp.saturating_sub(1))?
            .ok_or_else(|| StoreError::Invalid("cron 没有后续执行时间".into()))?;
        if task.kind == "set_avatar" && next_task_time(cron, next)?.is_some() {
            return Err(StoreError::Invalid("设置头像不支持周期计划".into()));
        }
        if task.expires_at.is_some_and(|end| end <= next) {
            return Err(StoreError::Invalid("执行时间不早于任务过期时间".into()));
        }
        let schedule = if task.enabled
            && !matches!(task.status.as_str(), "running" | "unknown" | "cancelled")
        {
            Some(next)
        } else {
            None
        };
        // Editing does not start a task. Explicit start/retry is a separate action.
        self.db.execute("UPDATE tasks SET cron_expression=?2,next_run_at=?3,status=CASE WHEN enabled=1 AND status IN ('completed','failed') THEN 'pending' ELSE status END,attempt_count=CASE WHEN status='running' THEN attempt_count ELSE 0 END,updated_at=?4 WHERE id=?1",params![id,cron,schedule,timestamp])?;
        self.task(id)
    }
    pub fn has_running_task(&self, id: &AppId) -> Result<bool, StoreError> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND status='running')",
            [id.as_str()],
            |r| r.get(0),
        )?)
    }
    pub fn claim_task_when(
        &mut self,
        expected: &AppTask,
        admit: impl FnOnce(&Connection) -> Result<(), StoreError>,
    ) -> Result<String, StoreError> {
        let timestamp = now();
        recover(&self.db, timestamp)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = get(&tx, &expected.id)?;
        if current != *expected
            || current.status != "pending"
            || !current.enabled
            || current.next_run_at.is_none_or(|next| next > timestamp)
        {
            return Err(StoreError::Conflict);
        }
        if let Some(parent) = &current.depends_on {
            let ready: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND app_id=?2 AND last_run_status='completed')", params![parent,current.app_id], |r| r.get(0))?;
            if !ready {
                return Err(StoreError::Conflict);
            }
        }
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND status='running')",
            [&current.app_id],
            |r| r.get(0),
        )?;
        if busy {
            return Err(StoreError::Conflict);
        }
        let archived: bool = tx.query_row(
            "SELECT archived FROM app_instances WHERE id=?1",
            [&current.app_id],
            |r| r.get(0),
        )?;
        if archived {
            return Err(StoreError::Invalid("虚拟 App 已归档".into()));
        }
        admit(&tx)?;
        // The watchdog covers fractional-second rounding, HTTP timeout cleanup
        // and durable result storage; it must never race the request's timeout.
        tx.execute("UPDATE tasks SET status='running',next_run_at=NULL,deadline=?2,started_at=?3,updated_at=?3,finished_at=NULL,attempt_count=attempt_count+1 WHERE id=?1 AND status='pending'",params![current.id,timestamp+i64::from(current.timeout_seconds)+15,timestamp])?;
        tx.commit()?;
        self.publish_changes();
        Ok(current.id)
    }
    pub fn start_task(&mut self, id: &AppId, kind: &str) -> Result<String, StoreError> {
        self.start_task_when(id, kind, |_| Ok(()))
    }
    pub fn start_task_when(
        &mut self,
        id: &AppId,
        kind: &str,
        admit: impl FnOnce(&Connection) -> Result<(), StoreError>,
    ) -> Result<String, StoreError> {
        self.load(id)?;
        let timestamp = now();
        let cron = task_cron_at(timestamp)?;
        let task = uuid::Uuid::new_v4().to_string();
        recover(&self.db, timestamp)?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        admit(&tx)?;
        tx.execute("INSERT INTO tasks(id,app_id,kind,status,cron_expression,created_at,updated_at,started_at,deadline,attempt_count,timeout_seconds) VALUES(?1,?2,?3,'running',?4,?5,?5,?5,?6,1,75)",params![task,id.as_str(),kind,cron,timestamp,timestamp+90])?;
        tx.commit()?;
        self.publish_changes();
        Ok(task)
    }
    pub fn finish_task(
        &mut self,
        id: &str,
        status: &str,
        result: &Value,
    ) -> Result<(), StoreError> {
        self.finish_task_with_policy(id, status, result, TaskFinishPolicy::default())
    }
    /// Preflight never sent a request and does not spend an attempt. Compare
    /// the entire expected row under the same write transaction as completion.
    pub fn fail_pending_task(
        &mut self,
        expected: &AppTask,
        message: &str,
    ) -> Result<(), StoreError> {
        let timestamp = now();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = get(&tx, &expected.id)?;
        if current != *expected || current.status != "pending" {
            return Err(StoreError::Conflict);
        }
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND status='running')",
            [&current.app_id],
            |r| r.get(0),
        )?;
        if busy {
            return Err(StoreError::Conflict);
        }
        let mut next=next_task_time(&current.cron_expression,timestamp)?.or_else(||current.task_key.as_ref().map(|_|timestamp+30));
        if current
            .expires_at
            .is_some_and(|end| next.is_some_and(|time| time >= end))
        {
            next = None;
        }
        let status = if next.is_some() { "pending" } else { "failed" };
        tx.execute("UPDATE tasks SET status=?2,next_run_at=?3,attempt_count=0,last_run_status='failed',result=?4,error_code='preflight',error_message=?5,updated_at=?6,finished_at=?6 WHERE id=?1 AND status='pending'",params![current.id,status,next,json!({"error":message}).to_string(),message,timestamp])?;
        tx.commit()?;
        self.publish_changes();
        Ok(())
    }
    pub fn finish_task_with_policy(
        &mut self,
        id: &str,
        status: &str,
        result: &Value,
        policy: TaskFinishPolicy,
    ) -> Result<(), StoreError> {
        if !["completed", "failed", "unknown"].contains(&status)
            || result.to_string().len() > 1024 * 1024
        {
            return Err(StoreError::Invalid("任务结果无效".into()));
        }
        let task = self.task(id)?;
        if task.status != "running" {
            return Err(StoreError::Conflict);
        }
        let timestamp = now();
        let retry = policy.retryable
            && !policy.stop_on_failure
            && status != "completed"
            && task.attempt_count < task.max_attempts;
        let mut next = if retry {
            Some(timestamp + 30 * 4_i64.pow(task.attempt_count.saturating_sub(1).min(3)))
        } else {
            next_task_time(
                &task.cron_expression,
                timestamp.max(task.started_at.unwrap_or(timestamp)),
            )?
        };
        if status!="completed" && next.is_none() && task.task_key.is_some() {
            next=Some(timestamp+30);
        }
        if !task.enabled {next=None;}
        if task
            .expires_at
            .is_some_and(|end| next.is_some_and(|time| time >= end))
        {
            next = None;
        }
        let state = if next.is_some() { "pending" } else { status };
        let count = if next.is_some() && !retry {
            0
        } else {
            task.attempt_count
        };
        let error = result["error"].as_str().map(str::to_owned).or_else(|| {
            result["http_status"]
                .as_u64()
                .filter(|code| *code >= 400)
                .map(|code| format!("HTTP {code}"))
        });
        let code = result["error_code"].as_str().map(str::to_owned);
        if self.db.execute("UPDATE tasks SET status=?2,next_run_at=?3,result=?4,last_run_status=?5,error_code=?6,error_message=?7,updated_at=?8,finished_at=?8,deadline=NULL,attempt_count=?9 WHERE id=?1 AND status='running'",params![id,state,next,result.to_string(),status,code,error,timestamp,count])?!=1 {return Err(StoreError::Conflict);}
        self.publish_changes();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
