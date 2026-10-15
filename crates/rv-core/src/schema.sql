BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY NOT NULL,
    guid TEXT UNIQUE CHECK (guid IS NULL OR length(trim(guid)) > 0),
    name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL CHECK(port BETWEEN 1 AND 65535),
    model TEXT, os_version TEXT, current_bundle_id TEXT,
    username TEXT, remember_password INTEGER NOT NULL,
    encryption TEXT NOT NULL, quality TEXT NOT NULL, clipboard TEXT NOT NULL, local_cursor TEXT NOT NULL,
    view_only INTEGER NOT NULL, zxtouch_port INTEGER CHECK(zxtouch_port BETWEEN 1 AND 65535),
    shared INTEGER NOT NULL, last_connected INTEGER,
    labels TEXT NOT NULL, function_favorites TEXT NOT NULL, transfer_folders TEXT NOT NULL,
    app_shortcuts TEXT NOT NULL, environment_apps TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS preferences (
    id INTEGER PRIMARY KEY CHECK(id=1), theme TEXT NOT NULL, hide_screenshots INTEGER NOT NULL,
    menu_key TEXT NOT NULL, default_scale TEXT NOT NULL, pin_toolbar INTEGER NOT NULL
);
-- Ad-hoc connection credentials also live here, without a device foreign key.
CREATE TABLE IF NOT EXISTS credentials (
    id TEXT PRIMARY KEY NOT NULL, encrypted BLOB NOT NULL
);
-- Complete phone replies, including baselines, missing IDFV and future fields.
CREATE TABLE IF NOT EXISTS phone_app_state (
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    bundle_id TEXT NOT NULL, result TEXT NOT NULL, updated_at INTEGER NOT NULL,
    PRIMARY KEY(device_id,bundle_id)
);
-- Explicitly dismissed IDFV values must not return on the next phone sync.
CREATE TABLE IF NOT EXISTS app_dismissed (id TEXT PRIMARY KEY NOT NULL);
CREATE TABLE IF NOT EXISTS app_instances (
    -- id is the canonical real IDFV, never a generated local identifier.
    id TEXT PRIMARY KEY NOT NULL, bundle_id TEXT NOT NULL, title TEXT NOT NULL,
    identity_key TEXT, tags TEXT NOT NULL DEFAULT '[]', note TEXT NOT NULL DEFAULT '',
    metadata TEXT NOT NULL DEFAULT '{}', favorite INTEGER NOT NULL DEFAULT 0,
    pinned INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0, group_name TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, last_used_at INTEGER,
    revision INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS app_devices (
    app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    PRIMARY KEY(app_id,device_id)
);
CREATE TABLE IF NOT EXISTS app_backups (
    app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    backup_id TEXT NOT NULL, title TEXT NOT NULL, metadata TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY(app_id,device_id,backup_id)
);
CREATE VIRTUAL TABLE IF NOT EXISTS app_search USING fts5(
    app_id UNINDEXED, title, bundle_id, tags, note, metadata, group_name,
    tokenize='trigram'
);
CREATE TRIGGER IF NOT EXISTS app_search_insert AFTER INSERT ON app_instances BEGIN
    INSERT INTO app_search VALUES(new.id,new.title,new.bundle_id,new.tags,new.note,new.metadata,new.group_name);
END;
CREATE TRIGGER IF NOT EXISTS app_search_update AFTER UPDATE ON app_instances BEGIN
    DELETE FROM app_search WHERE app_id=old.id;
    INSERT INTO app_search VALUES(new.id,new.title,new.bundle_id,new.tags,new.note,new.metadata,new.group_name);
END;
CREATE TRIGGER IF NOT EXISTS app_search_delete AFTER DELETE ON app_instances BEGIN
    DELETE FROM app_search WHERE app_id=old.id;
END;
PRAGMA user_version=1;
COMMIT;
