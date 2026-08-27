-- A revision is a complete immutable prior Post state. Existing partial revision rows
-- are deliberately not migrated: production has none, and compatibility would make
-- an incomplete snapshot appear reconstructible.
DROP TABLE post_revisions;
CREATE TABLE post_revisions (
    revision_id  INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id      INTEGER NOT NULL REFERENCES posts(post_id),
    user_id      INTEGER NOT NULL REFERENCES users(user_id),
    title        TEXT,
    slug         TEXT NOT NULL,
    body         TEXT NOT NULL,
    format       TEXT NOT NULL,
    rendered_html TEXT NOT NULL,
    summary      TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    published_at TEXT,
    deleted_at   TEXT,
    captured_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (post_id, revision_id)
);

-- Revision children copy their normalized values instead of pointing at mutable
-- lookups, so later tag/audience maintenance cannot rewrite or block history.
CREATE TABLE post_revision_tags (
    revision_id INTEGER NOT NULL REFERENCES post_revisions(revision_id),
    tag_slug    TEXT NOT NULL,
    tag_display TEXT NOT NULL,
    PRIMARY KEY (revision_id, tag_slug)
);
CREATE TABLE post_revision_audiences (
    revision_id  INTEGER NOT NULL REFERENCES post_revisions(revision_id),
    target_kind  TEXT NOT NULL CHECK (target_kind IN ('public', 'subscribers', 'named')),
    audience_id  INTEGER,
    CHECK (
        (target_kind = 'named' AND audience_id IS NOT NULL)
        OR (target_kind IN ('public', 'subscribers') AND audience_id IS NULL)
    )
);
CREATE UNIQUE INDEX post_revision_audiences_named_unique
    ON post_revision_audiences (revision_id, audience_id)
    WHERE target_kind = 'named';
CREATE UNIQUE INDEX post_revision_audiences_builtin_unique
    ON post_revision_audiences (revision_id, target_kind)
    WHERE target_kind IN ('public', 'subscribers');

-- Both current and historical references use one exact subject key. A non-null
-- sentinel keeps the portable primary key meaningful for current rows.
CREATE TABLE post_media_new (
    post_id        INTEGER NOT NULL REFERENCES posts(post_id),
    subject_kind   TEXT NOT NULL DEFAULT 'current'
        CHECK (subject_kind IN ('current', 'revision')),
    revision_id    INTEGER NOT NULL DEFAULT 0,
    source         TEXT NOT NULL,
    sha256         TEXT NOT NULL,
    filename       TEXT NOT NULL,
    reference_kind TEXT NOT NULL DEFAULT 'legacy',
    reference_form TEXT NOT NULL DEFAULT '',
    CHECK (
        (subject_kind = 'current' AND revision_id = 0)
        OR (subject_kind = 'revision' AND revision_id > 0)
    ),
    PRIMARY KEY (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form)
);
INSERT INTO post_media_new (post_id, source, sha256, filename, reference_kind, reference_form)
SELECT post_id, source, sha256, filename, reference_kind, reference_form FROM post_media;
DROP TABLE post_media;
ALTER TABLE post_media_new RENAME TO post_media;
CREATE INDEX idx_post_media_lookup
    ON post_media (sha256, filename, source, reference_kind, reference_form);

-- SQLite cannot express the conditional composite foreign key required by the
-- sentinel current subject, so these triggers enforce the exact Revision key.
CREATE TRIGGER post_media_revision_subject_insert
BEFORE INSERT ON post_media
WHEN NEW.subject_kind = 'revision'
 AND NOT EXISTS (
    SELECT 1 FROM post_revisions
    WHERE post_id = NEW.post_id AND revision_id = NEW.revision_id
 )
BEGIN
    SELECT RAISE(ABORT, 'revision media subject must match post revision');
END;
CREATE TRIGGER post_media_revision_subject_update
BEFORE UPDATE OF post_id, subject_kind, revision_id ON post_media
WHEN NEW.subject_kind = 'revision'
 AND NOT EXISTS (
    SELECT 1 FROM post_revisions
    WHERE post_id = NEW.post_id AND revision_id = NEW.revision_id
 )
BEGIN
    SELECT RAISE(ABORT, 'revision media subject must match post revision');
END;
