-- Persist validated package members with the mutable draft for portable import/export.
CREATE TABLE theme_draft_assets (
    theme_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    mime TEXT NOT NULL,
    bytes BLOB NOT NULL,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    PRIMARY KEY (theme_id, path),
    FOREIGN KEY (theme_id) REFERENCES theme_drafts(theme_id) ON DELETE CASCADE
);
