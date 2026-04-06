CREATE TABLE invites (
    code       TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at    TEXT,
    used_by    INTEGER REFERENCES users(user_id) ON DELETE SET NULL
);
