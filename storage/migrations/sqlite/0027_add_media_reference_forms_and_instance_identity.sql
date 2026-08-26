-- Persist the exact URL reference which yielded each media identity. Existing rows
-- predate that evidence and are marked legacy for transactional re-derivation.
CREATE TABLE post_media_new (
    post_id        INTEGER NOT NULL REFERENCES posts(post_id),
    source         TEXT NOT NULL,
    sha256         TEXT NOT NULL,
    filename       TEXT NOT NULL,
    reference_kind TEXT NOT NULL DEFAULT 'legacy',
    reference_form TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (post_id, source, sha256, filename, reference_kind, reference_form)
);
INSERT INTO post_media_new (post_id, source, sha256, filename)
SELECT post_id, source, sha256, filename FROM post_media;
DROP TABLE post_media;
ALTER TABLE post_media_new RENAME TO post_media;
CREATE INDEX idx_post_media_lookup
    ON post_media (sha256, filename, source, reference_kind, reference_form);

CREATE TABLE instance_identity (
    singleton   INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL
);
