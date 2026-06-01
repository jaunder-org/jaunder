UPDATE sessions SET label = 'Unknown device' WHERE label IS NULL;
CREATE TABLE sessions_new (
    token_hash   TEXT PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES users(user_id),
    label        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    last_used_at TEXT NOT NULL
);
INSERT INTO sessions_new SELECT token_hash, user_id, label, created_at, last_used_at FROM sessions;
DROP TABLE sessions;
ALTER TABLE sessions_new RENAME TO sessions;
