CREATE TABLE IF NOT EXISTS vinted_schema (singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL);
INSERT OR IGNORE INTO vinted_schema VALUES(1,1);
CREATE TABLE IF NOT EXISTS vinted_products (
 app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
 product_id TEXT NOT NULL,
 payload TEXT NOT NULL, fetched_at INTEGER NOT NULL,
 PRIMARY KEY(app_id,product_id)
);
CREATE TABLE IF NOT EXISTS vinted_orders (
 app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
 order_id TEXT NOT NULL, direction TEXT NOT NULL,
 payload TEXT NOT NULL, fetched_at INTEGER NOT NULL,
 PRIMARY KEY(app_id,order_id,direction)
);
CREATE INDEX IF NOT EXISTS vinted_products_updated ON vinted_products(app_id,fetched_at DESC);
CREATE INDEX IF NOT EXISTS vinted_orders_updated ON vinted_orders(app_id,direction,fetched_at DESC);
CREATE VIRTUAL TABLE IF NOT EXISTS vinted_product_search USING fts5(app_id UNINDEXED,product_id UNINDEXED,title,brand,payload,tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS vinted_product_insert AFTER INSERT ON vinted_products BEGIN
 INSERT INTO vinted_product_search(rowid,app_id,product_id,title,brand,payload) VALUES(new.rowid,new.app_id,new.product_id,json_extract(new.payload,'$.title'),json_extract(new.payload,'$.brand'),new.payload);
END;
CREATE TRIGGER IF NOT EXISTS vinted_product_update AFTER UPDATE ON vinted_products BEGIN
 DELETE FROM vinted_product_search WHERE rowid=old.rowid;
 INSERT INTO vinted_product_search(rowid,app_id,product_id,title,brand,payload) VALUES(new.rowid,new.app_id,new.product_id,json_extract(new.payload,'$.title'),json_extract(new.payload,'$.brand'),new.payload);
END;
CREATE TRIGGER IF NOT EXISTS vinted_product_delete AFTER DELETE ON vinted_products BEGIN
 DELETE FROM vinted_product_search WHERE rowid=old.rowid;
END;
CREATE VIRTUAL TABLE IF NOT EXISTS vinted_order_search USING fts5(app_id UNINDEXED,order_id UNINDEXED,title,status,payload,tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS vinted_order_insert AFTER INSERT ON vinted_orders BEGIN
 INSERT INTO vinted_order_search(rowid,app_id,order_id,title,status,payload) VALUES(new.rowid,new.app_id,new.order_id,json_extract(new.payload,'$.title'),json_extract(new.payload,'$.status'),new.payload);
END;
CREATE TRIGGER IF NOT EXISTS vinted_order_update AFTER UPDATE ON vinted_orders BEGIN
 DELETE FROM vinted_order_search WHERE rowid=old.rowid;
 INSERT INTO vinted_order_search(rowid,app_id,order_id,title,status,payload) VALUES(new.rowid,new.app_id,new.order_id,json_extract(new.payload,'$.title'),json_extract(new.payload,'$.status'),new.payload);
END;
CREATE TRIGGER IF NOT EXISTS vinted_order_delete AFTER DELETE ON vinted_orders BEGIN
 DELETE FROM vinted_order_search WHERE rowid=old.rowid;
END;
