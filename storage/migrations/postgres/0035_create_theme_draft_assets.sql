-- Persist validated package members with the mutable draft for portable import/export.
CREATE TABLE theme_draft_assets (
    theme_id BIGINT NOT NULL REFERENCES theme_drafts(theme_id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    path TEXT NOT NULL,
    mime TEXT NOT NULL,
    bytes BYTEA NOT NULL,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    PRIMARY KEY (theme_id, path)
);
