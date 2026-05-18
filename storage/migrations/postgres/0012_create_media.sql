CREATE TABLE IF NOT EXISTS media (
    user_id BIGINT NOT NULL REFERENCES users(user_id),
    sha256 TEXT NOT NULL,
    filename TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('upload', 'cached')),
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    source_url TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, sha256, filename, source)
);
