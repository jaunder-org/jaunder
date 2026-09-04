-- Legacy cache rows predate persisted semantic identity and cannot safely retain
-- HTTP validators. Rebuild after invalidation so the new column has no default.
DROP TABLE IF EXISTS feed_cache_0032;
CREATE TABLE feed_cache_0032 (
    feed_url                       TEXT PRIMARY KEY,
    body                           TEXT NOT NULL,
    etag                           TEXT NOT NULL,
    content_type                   TEXT NOT NULL,
    representation_modified_at     TIMESTAMP NOT NULL,
    generated_at                   TIMESTAMP NOT NULL,
    semantic_fingerprint           TEXT NOT NULL
);
DROP TABLE feed_cache;
ALTER TABLE feed_cache_0032 RENAME TO feed_cache;
