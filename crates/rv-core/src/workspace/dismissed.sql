BEGIN IMMEDIATE;
-- Explicitly dismissed IDFV values must not return on the next phone sync.
-- Some version-3 installations already have this table from fresh bootstrap.
CREATE TABLE IF NOT EXISTS app_dismissed (id TEXT PRIMARY KEY NOT NULL);
PRAGMA user_version=4;
COMMIT;
