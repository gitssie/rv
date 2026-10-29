BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    task_key TEXT,
    depends_on TEXT REFERENCES tasks(id),
    payload TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK(status IN ('pending','running','completed','failed','unknown','cancelled')),
    cron_expression TEXT NOT NULL,
    next_run_at INTEGER,
    expires_at INTEGER,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count>=0),
    max_attempts INTEGER NOT NULL DEFAULT 1 CHECK(max_attempts BETWEEN 1 AND 10),
    timeout_seconds INTEGER NOT NULL DEFAULT 90 CHECK(timeout_seconds BETWEEN 1 AND 3600),
    deadline INTEGER,
    result TEXT,
    last_run_status TEXT,
    error_code TEXT,
    error_message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    CHECK(status!='pending' OR next_run_at IS NOT NULL),
    CHECK(status!='running' OR deadline IS NOT NULL)
);
CREATE UNIQUE INDEX IF NOT EXISTS tasks_key ON tasks(app_id,task_key) WHERE task_key IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS tasks_active ON tasks(app_id) WHERE status='running';
CREATE INDEX IF NOT EXISTS tasks_due ON tasks(next_run_at) WHERE status='pending';
CREATE INDEX IF NOT EXISTS tasks_app_time ON tasks(app_id,created_at DESC);
PRAGMA user_version=8;
COMMIT;
