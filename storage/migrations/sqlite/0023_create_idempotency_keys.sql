CREATE TABLE idempotency_keys (
    idempotency_key_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(user_id),
    key TEXT NOT NULL,
    post_id INTEGER NOT NULL REFERENCES posts(post_id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, key)
);
