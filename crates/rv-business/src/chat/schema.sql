CREATE TABLE IF NOT EXISTS chat_schema(singleton INTEGER PRIMARY KEY CHECK(singleton=1),version INTEGER NOT NULL);
INSERT OR IGNORE INTO chat_schema VALUES(1,1);
CREATE TABLE IF NOT EXISTS chat_conversations(
 id TEXT PRIMARY KEY,app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
 remote_id TEXT NOT NULL,title TEXT NOT NULL,preview TEXT NOT NULL,updated_at INTEGER NOT NULL,
 unread INTEGER NOT NULL CHECK(unread IN(0,1)),allow_reply INTEGER NOT NULL CHECK(allow_reply IN(0,1)),
 detail_loaded INTEGER NOT NULL DEFAULT 0,data TEXT NOT NULL,UNIQUE(app_id,remote_id)
);
CREATE INDEX IF NOT EXISTS chat_account_updated ON chat_conversations(app_id,updated_at DESC);
CREATE TABLE IF NOT EXISTS chat_messages(
 id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
 remote_id TEXT NOT NULL,created_at INTEGER NOT NULL,body TEXT NOT NULL,data TEXT NOT NULL,
 UNIQUE(conversation_id,remote_id)
);
CREATE INDEX IF NOT EXISTS chat_message_time ON chat_messages(conversation_id,created_at);
CREATE TABLE IF NOT EXISTS chat_drafts(
 conversation_id TEXT PRIMARY KEY REFERENCES chat_conversations(id) ON DELETE CASCADE,body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_outbox(
 id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
 body TEXT NOT NULL,created_at INTEGER NOT NULL,status TEXT NOT NULL CHECK(status IN('pending','sending','submitted','failed','unknown','cancelled')),
 remote_id TEXT,
 prepared_body TEXT,
 source_language TEXT,
 target_language TEXT,
 available_at INTEGER NOT NULL DEFAULT 0,
 updated_at INTEGER NOT NULL DEFAULT 0,
 started_at INTEGER,
 task_id TEXT,
 error TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS chat_one_pending_send ON chat_outbox(conversation_id) WHERE status IN('sending','unknown','submitted');
DROP TRIGGER IF EXISTS chat_message_insert;
DROP TRIGGER IF EXISTS chat_message_update;
DROP TRIGGER IF EXISTS chat_message_delete;
DROP TABLE IF EXISTS chat_message_search;
CREATE INDEX IF NOT EXISTS chat_outbox_ready ON chat_outbox(status,available_at,created_at);
CREATE TABLE IF NOT EXISTS chat_outbox_images(
 outbox_id TEXT PRIMARY KEY REFERENCES chat_outbox(id) ON DELETE CASCADE,
 jpeg BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_customers(
 id TEXT PRIMARY KEY,app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
 remote_user_id TEXT NOT NULL,username TEXT NOT NULL,display_name TEXT NOT NULL,
 avatar_url TEXT,country_code TEXT,preferred_language TEXT,language_source TEXT,
 tags TEXT NOT NULL DEFAULT '[]',note TEXT NOT NULL DEFAULT '',data TEXT NOT NULL,
 created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,UNIQUE(app_id,remote_user_id)
);
CREATE TABLE IF NOT EXISTS chat_conversation_customers(
 conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
 customer_id TEXT NOT NULL REFERENCES chat_customers(id) ON DELETE CASCADE,
 role TEXT NOT NULL DEFAULT 'counterparty',PRIMARY KEY(conversation_id,customer_id)
);
CREATE TABLE IF NOT EXISTS chat_view_state(
 app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,
 conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE SET NULL
);
CREATE TABLE IF NOT EXISTS chat_read_requests(
 conversation_id TEXT PRIMARY KEY REFERENCES chat_conversations(id) ON DELETE CASCADE,
 requested_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chat_detail_sync(
 conversation_id TEXT PRIMARY KEY REFERENCES chat_conversations(id) ON DELETE CASCADE,
 inbox_updated_at INTEGER NOT NULL, fetched_at INTEGER NOT NULL
);
