BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS app_auth (
    app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,
    encrypted BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS app_tasks (
    id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT,
    created_at INTEGER NOT NULL,
    finished_at INTEGER,
    deadline INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS app_task_active ON app_tasks(app_id) WHERE status='running';
PRAGMA user_version=2;
COMMIT;
