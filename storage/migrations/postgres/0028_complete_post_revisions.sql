-- A revision is a complete immutable prior Post state. Existing partial revision rows
-- are deliberately not migrated: production has none, and compatibility would make
-- an incomplete snapshot appear reconstructible.
DROP TABLE post_revisions;
CREATE TABLE post_revisions (
    revision_id  BIGSERIAL PRIMARY KEY,
    post_id      BIGINT NOT NULL REFERENCES posts(post_id) DEFERRABLE INITIALLY IMMEDIATE,
    user_id      BIGINT NOT NULL REFERENCES users(user_id) DEFERRABLE INITIALLY IMMEDIATE,
    title        TEXT,
    slug         TEXT NOT NULL,
    body         TEXT NOT NULL,
    format       TEXT NOT NULL,
    rendered_html TEXT NOT NULL,
    summary      TEXT,
    created_at   TIMESTAMPTZ NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL,
    published_at TIMESTAMPTZ,
    deleted_at   TIMESTAMPTZ,
    captured_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (post_id, revision_id)
);

-- Revision children copy their normalized values instead of pointing at mutable
-- lookups, so later tag/audience maintenance cannot rewrite or block history.
CREATE TABLE post_revision_tags (
    revision_id BIGINT NOT NULL REFERENCES post_revisions(revision_id) DEFERRABLE INITIALLY IMMEDIATE,
    tag_slug    TEXT NOT NULL,
    tag_display TEXT NOT NULL,
    PRIMARY KEY (revision_id, tag_slug)
);
CREATE TABLE post_revision_audiences (
    revision_id BIGINT NOT NULL REFERENCES post_revisions(revision_id) DEFERRABLE INITIALLY IMMEDIATE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('public', 'subscribers', 'named')),
    audience_id BIGINT,
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
-- sentinel keeps the portable primary key meaningful for current rows while
-- preserving the existing current-reference relation.
ALTER TABLE post_media
    ADD COLUMN subject_kind TEXT NOT NULL DEFAULT 'current'
        CHECK (subject_kind IN ('current', 'revision')),
    ADD COLUMN revision_id BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT post_media_subject_shape CHECK (
        (subject_kind = 'current' AND revision_id = 0)
        OR (subject_kind = 'revision' AND revision_id > 0)
    );
ALTER TABLE post_media
    DROP CONSTRAINT post_media_pkey,
    ADD PRIMARY KEY (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form);

-- PostgreSQL likewise needs a conditional relation: current rows carry the
-- sentinel rather than a revision ID, while revision rows must match both IDs.
CREATE FUNCTION enforce_post_media_revision_subject() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.subject_kind = 'revision' AND NOT EXISTS (
        SELECT 1 FROM post_revisions
        WHERE post_id = NEW.post_id AND revision_id = NEW.revision_id
    ) THEN
        RAISE EXCEPTION 'revision media subject must match post revision';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER post_media_revision_subject
BEFORE INSERT OR UPDATE OF post_id, subject_kind, revision_id ON post_media
FOR EACH ROW EXECUTE FUNCTION enforce_post_media_revision_subject();
