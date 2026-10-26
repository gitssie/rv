BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS apple_accounts (
 apple_id TEXT PRIMARY KEY NOT NULL COLLATE NOCASE,
 original_password TEXT NOT NULL,
 password TEXT NOT NULL,
 pending_password TEXT,
 verification_code_url TEXT,
 device_id TEXT,
 params TEXT NOT NULL DEFAULT '{}',
 init_status TEXT NOT NULL DEFAULT 'pending' CHECK(init_status IN ('pending','changing_password','ready','unknown')),
 account_status TEXT NOT NULL DEFAULT 'unknown' CHECK(account_status IN ('unknown','active','locked','disabled')),
 created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 first_login_at INTEGER, last_login_at INTEGER, password_changed_at INTEGER,
 last_error_domain TEXT, last_error_code TEXT, last_error_message TEXT, last_checked_at INTEGER,
 password_change_operation_id TEXT,
 note TEXT NOT NULL DEFAULT '', revision INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS apple_accounts_device ON apple_accounts(device_id);
PRAGMA user_version=7;
COMMIT;
