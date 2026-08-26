-- Persist the exact URL reference which yielded each media identity. Existing rows
-- predate that evidence and are marked legacy for transactional re-derivation.
ALTER TABLE post_media
    ADD COLUMN reference_kind TEXT NOT NULL DEFAULT 'legacy',
    ADD COLUMN reference_form TEXT NOT NULL DEFAULT '';
ALTER TABLE post_media
    DROP CONSTRAINT post_media_pkey,
    ADD PRIMARY KEY (post_id, source, sha256, filename, reference_kind, reference_form);
DROP INDEX idx_post_media_lookup;
CREATE INDEX idx_post_media_lookup
    ON post_media (sha256, filename, source, reference_kind, reference_form);

CREATE TABLE instance_identity (
    singleton   SMALLINT PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL
);
