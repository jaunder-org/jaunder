-- UTC is an explicit fallback, not an inferred historical editing zone.
ALTER TABLE posts ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE post_revisions ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
