CREATE TABLE sessions (
    token_hash   TEXT PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(user_id),
    label        TEXT,
    created_at   TEXT NOT NULL,
    last_used_at TEXT NOT NULL
);
