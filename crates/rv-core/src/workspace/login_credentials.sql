BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS app_login_credentials (
    app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,
    login_username TEXT NOT NULL CHECK(length(trim(login_username)) > 0),
    login_password TEXT
);
-- Existing verified shop metadata supplies usernames, never guessed passwords.
INSERT INTO app_login_credentials(app_id, login_username)
SELECT id, COALESCE(
    CASE WHEN json_type(metadata, '$.login') = 'text' THEN NULLIF(trim(json_extract(metadata, '$.login')), '') END,
    CASE WHEN json_type(metadata, '$.shop.login') = 'text' THEN NULLIF(trim(json_extract(metadata, '$.shop.login')), '') END
)
FROM app_instances
WHERE json_valid(metadata) AND COALESCE(
    CASE WHEN json_type(metadata, '$.login') = 'text' THEN NULLIF(trim(json_extract(metadata, '$.login')), '') END,
    CASE WHEN json_type(metadata, '$.shop.login') = 'text' THEN NULLIF(trim(json_extract(metadata, '$.shop.login')), '') END
) IS NOT NULL
ON CONFLICT(app_id) DO NOTHING;
PRAGMA user_version=15;
COMMIT;
