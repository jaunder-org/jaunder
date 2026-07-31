-- What a post's rendered HTML points a reader at: one row per (post, media
-- reference). No user_id column — post_id already determines the author — and no FK
-- to `media`, whose key is (user_id, sha256, filename, source) and so cannot be
-- referenced by a URL-derived triple: a post may legitimately reference an entry
-- with no row.
CREATE TABLE post_media (
    post_id  INTEGER NOT NULL REFERENCES posts(post_id),
    source   TEXT NOT NULL,
    sha256   TEXT NOT NULL,
    filename TEXT NOT NULL,
    PRIMARY KEY (post_id, source, sha256, filename)
);
-- The reclamation direction: "which posts reference this entry?" keyed by the
-- (sha256, filename, source) triple a media row is identified by.
CREATE INDEX idx_post_media_lookup ON post_media(sha256, filename, source);
