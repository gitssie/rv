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
CREATE TABLE IF NOT EXISTS chat_participants(
 app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,remote_id TEXT NOT NULL,name TEXT NOT NULL,data TEXT NOT NULL,
 PRIMARY KEY(app_id,remote_id)
);
CREATE TABLE IF NOT EXISTS chat_drafts(
 conversation_id TEXT PRIMARY KEY REFERENCES chat_conversations(id) ON DELETE CASCADE,body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_outbox(
 id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
 body TEXT NOT NULL,created_at INTEGER NOT NULL,status TEXT NOT NULL CHECK(status IN('sending','submitted','failed','unknown')),
 remote_id TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS chat_one_pending_send ON chat_outbox(conversation_id) WHERE status IN('sending','unknown','submitted');
CREATE VIRTUAL TABLE IF NOT EXISTS chat_message_search USING fts5(message_id UNINDEXED,conversation_id UNINDEXED,body,tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS chat_message_insert AFTER INSERT ON chat_messages BEGIN
 INSERT INTO chat_message_search(rowid,message_id,conversation_id,body) VALUES(new.rowid,new.id,new.conversation_id,new.body);
END;
CREATE TRIGGER IF NOT EXISTS chat_message_update AFTER UPDATE ON chat_messages BEGIN
 DELETE FROM chat_message_search WHERE rowid=old.rowid;
 INSERT INTO chat_message_search(rowid,message_id,conversation_id,body) VALUES(new.rowid,new.id,new.conversation_id,new.body);
END;
CREATE TRIGGER IF NOT EXISTS chat_message_delete AFTER DELETE ON chat_messages BEGIN
 DELETE FROM chat_message_search WHERE rowid=old.rowid;
END;
