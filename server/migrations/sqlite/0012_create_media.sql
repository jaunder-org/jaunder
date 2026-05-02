CREATE TABLE IF NOT EXISTS media (
    user_id INTEGER NOT NULL REFERENCES users(user_id),
    sha256 TEXT NOT NULL,
    filename TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('upload', 'cached')),
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    source_url TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (user_id, sha256, filename, source)
);
