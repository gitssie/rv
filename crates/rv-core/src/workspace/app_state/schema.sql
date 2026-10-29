BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS app_states (
    app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,
    source_device_id TEXT REFERENCES devices(id) ON DELETE SET NULL,
    manifest_blob BLOB NOT NULL,
    archive_blob BLOB NOT NULL,
    keychain_blob BLOB NOT NULL,
    identity_blob BLOB NOT NULL,
    preferences_blob BLOB NOT NULL,
    device_context_blob BLOB NOT NULL,
    archive_sha256 TEXT NOT NULL,
    archive_size INTEGER NOT NULL,
    imported_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
PRAGMA user_version=9;
COMMIT;
