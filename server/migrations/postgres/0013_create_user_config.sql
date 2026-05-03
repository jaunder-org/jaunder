CREATE TABLE IF NOT EXISTS user_config (
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (user_id, key)
);
