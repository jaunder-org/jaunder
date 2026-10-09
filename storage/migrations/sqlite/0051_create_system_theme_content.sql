-- System-owned Theme Package metadata is deliberately distinct from the mutable
-- Site/Author catalog. It shares theme_content_eligibility and the immutable
-- digest filesystem; only ownership and current-release references are separate.
CREATE TABLE system_theme_revisions (
    theme_token TEXT NOT NULL CHECK (theme_token IN ('terminal', 'studio', 'reader')),
    revision_digest TEXT NOT NULL CHECK (length(revision_digest) = 64),
    source_digest TEXT NOT NULL CHECK (length(source_digest) = 64),
    stylesheet_digest TEXT NOT NULL CHECK (length(stylesheet_digest) = 64),
    manifest BLOB NOT NULL,
    PRIMARY KEY (theme_token, revision_digest)
);
CREATE TABLE system_theme_revision_assets (
    theme_token TEXT NOT NULL,
    revision_digest TEXT NOT NULL,
    path TEXT NOT NULL,
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    mime TEXT NOT NULL,
    PRIMARY KEY (theme_token, revision_digest, path),
    FOREIGN KEY (theme_token, revision_digest)
        REFERENCES system_theme_revisions(theme_token, revision_digest)
        ON DELETE RESTRICT
);
CREATE TABLE system_theme_current (
    theme_token TEXT PRIMARY KEY CHECK (theme_token IN ('terminal', 'studio', 'reader')),
    revision_digest TEXT NOT NULL CHECK (length(revision_digest) = 64),
    FOREIGN KEY (theme_token, revision_digest)
        REFERENCES system_theme_revisions(theme_token, revision_digest)
        ON DELETE RESTRICT
);
CREATE TABLE system_application_current (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    digest TEXT NOT NULL CHECK (length(digest) = 64),
    mime TEXT NOT NULL
);
-- Counts system references separately from custom quota charges. Publication and
-- collection must still update the shared eligibility live-reference count.
CREATE TABLE system_theme_content_references (
    digest TEXT PRIMARY KEY CHECK (length(digest) = 64),
    live_references INTEGER NOT NULL CHECK (live_references >= 0),
    retained_until_unix_seconds INTEGER NOT NULL,
    FOREIGN KEY (digest) REFERENCES theme_content_eligibility(digest) ON DELETE RESTRICT
);
-- Release admission reads only current roles; detached history must not extend
-- SQLite's write-lock hold as old releases accumulate.
CREATE INDEX system_theme_content_references_live_digest
    ON system_theme_content_references (digest)
    WHERE live_references > 0;
