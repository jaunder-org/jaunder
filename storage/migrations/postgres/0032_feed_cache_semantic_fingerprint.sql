-- Legacy cache rows predate persisted semantic identity and cannot safely retain
-- HTTP validators. Clear them before making the fingerprint required.
DELETE FROM feed_cache;
ALTER TABLE feed_cache RENAME COLUMN updated_at TO representation_modified_at;
ALTER TABLE feed_cache ADD COLUMN semantic_fingerprint TEXT NOT NULL;
