BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS app_execution (
    app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK(mode IN ('rv','to_phone','phone')),
    device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    operation_id TEXT NOT NULL,
    state_sha256 TEXT,
    updated_at INTEGER NOT NULL
);
PRAGMA user_version=11;
COMMIT;
