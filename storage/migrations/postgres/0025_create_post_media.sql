-- What a post's rendered HTML points a reader at: one row per (post, media
-- reference). No user_id column — post_id already determines the author — and no FK
-- to `media`, whose key is (user_id, sha256, filename, source) and so cannot be
-- referenced by a URL-derived triple: a post may legitimately reference an entry
-- with no row.
--
-- `posts.post_id` is BIGSERIAL here, so the child column must be BIGINT (an INTEGER
-- column is a hard "key columns are of incompatible types" error). The FK declares
-- its own deferrability because 0024_defer_foreign_keys.sql was a one-shot pass over
-- the constraints that existed then: without it `every_foreign_key_is_deferrable`
-- fails, and a restore breaks for real — `backup_table_set` sorts alphabetically and
-- "post_media" < "posts", so the child loads before the parent under
-- SET CONSTRAINTS ALL DEFERRED.
CREATE TABLE post_media (
    post_id  BIGINT NOT NULL REFERENCES posts(post_id) DEFERRABLE INITIALLY IMMEDIATE,
    source   TEXT NOT NULL,
    sha256   TEXT NOT NULL,
    filename TEXT NOT NULL,
    PRIMARY KEY (post_id, source, sha256, filename)
);
-- The reclamation direction: "which posts reference this entry?" keyed by the
-- (sha256, filename, source) triple a media row is identified by.
CREATE INDEX idx_post_media_lookup ON post_media (sha256, filename, source);
