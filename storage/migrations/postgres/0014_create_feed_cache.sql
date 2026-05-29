CREATE TABLE feed_cache (
    feed_url     TEXT PRIMARY KEY,
    body         TEXT NOT NULL,
    etag         TEXT NOT NULL,
    content_type TEXT NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL
);
