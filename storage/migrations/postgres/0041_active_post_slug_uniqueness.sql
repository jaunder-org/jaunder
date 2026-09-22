-- Repair legacy per-date slug duplicates before making the active slug namespace
-- authoritative per User. The procedural allocation is deterministic and tries
-- the same base, -1, -2, ... sequence as runtime without a fixed attempt bound.
CREATE TABLE post_permalink_aliases (
    alias_id       BIGSERIAL PRIMARY KEY,
    post_id        BIGINT NOT NULL REFERENCES posts(post_id) DEFERRABLE INITIALLY IMMEDIATE,
    user_id        BIGINT NOT NULL REFERENCES users(user_id) DEFERRABLE INITIALLY IMMEDIATE,
    permalink_date DATE NOT NULL,
    slug           TEXT NOT NULL,
    UNIQUE (user_id, permalink_date, slug)
);
CREATE INDEX post_permalink_aliases_post_id ON post_permalink_aliases(post_id);

CREATE TEMP TABLE _slug_repair_queue ON COMMIT DROP AS
WITH ranked AS (
    SELECT
        post_id,
        user_id,
        slug AS old_slug,
        (COALESCE(published_at, created_at) AT TIME ZONE 'UTC')::date AS old_permalink_date,
        created_at,
        updated_at AS old_updated_at,
        ROW_NUMBER() OVER (
            PARTITION BY user_id, slug
            ORDER BY created_at DESC, post_id DESC
        ) AS newest_rank,
        COUNT(*) OVER (PARTITION BY user_id, slug) AS duplicate_count
    FROM posts
    WHERE deleted_at IS NULL
)
SELECT
    ROW_NUMBER() OVER (
        ORDER BY user_id, old_slug, created_at, post_id
    ) AS repair_sequence,
    post_id,
    user_id,
    old_slug,
    old_permalink_date,
    old_updated_at
FROM ranked
WHERE duplicate_count > 1 AND newest_rank > 1;

CREATE TEMP TABLE _slug_repairs (
    post_id            BIGINT PRIMARY KEY,
    user_id            BIGINT NOT NULL,
    old_slug           TEXT NOT NULL,
    old_permalink_date DATE NOT NULL,
    new_slug           TEXT NOT NULL,
    new_updated_at     TIMESTAMPTZ NOT NULL,
    revision_id        BIGINT
) ON COMMIT DROP;

DO $$
DECLARE
    queued RECORD;
    attempt BIGINT;
    candidate TEXT;
BEGIN
    FOR queued IN
        SELECT * FROM _slug_repair_queue ORDER BY repair_sequence
    LOOP
        attempt := 1;
        LOOP
            candidate := left(
                queued.old_slug,
                80 - char_length('-' || attempt::text)
            ) || '-' || attempt::text;
            EXIT WHEN NOT EXISTS (
                SELECT 1 FROM posts existing
                WHERE existing.deleted_at IS NULL
                  AND existing.user_id = queued.user_id
                  AND existing.slug = candidate
            ) AND NOT EXISTS (
                SELECT 1 FROM _slug_repairs assigned
                WHERE assigned.user_id = queued.user_id
                  AND assigned.new_slug = candidate
            );
            attempt := attempt + 1;
        END LOOP;

        INSERT INTO _slug_repairs (
            post_id, user_id, old_slug, old_permalink_date, new_slug, new_updated_at
        ) VALUES (
            queued.post_id,
            queued.user_id,
            queued.old_slug,
            queued.old_permalink_date,
            candidate,
            GREATEST(CURRENT_TIMESTAMP, queued.old_updated_at + INTERVAL '1 microsecond')
        );
    END LOOP;
END
$$;

INSERT INTO post_permalink_aliases (post_id, user_id, permalink_date, slug)
SELECT post_id, user_id, old_permalink_date, old_slug
FROM _slug_repairs;

WITH inserted AS (
    INSERT INTO post_revisions (
        post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
        summary, created_at, updated_at, published_at, deleted_at, captured_at
    )
    SELECT
        post.post_id, post.user_id, post.title, post.rendered_title, post.slug,
        post.body, post.format, post.rendered_html, post.summary, post.created_at,
        post.updated_at, post.published_at, post.deleted_at, repair.new_updated_at
    FROM posts post
    JOIN _slug_repairs repair ON repair.post_id = post.post_id
    RETURNING post_id, revision_id
)
UPDATE _slug_repairs repair
SET revision_id = inserted.revision_id
FROM inserted
WHERE inserted.post_id = repair.post_id;

INSERT INTO post_revision_tags (revision_id, tag_slug, tag_display)
SELECT repair.revision_id, tag.tag_slug, post_tag.tag_display
FROM _slug_repairs repair
JOIN post_tags post_tag ON post_tag.post_id = repair.post_id
JOIN tags tag ON tag.tag_id = post_tag.tag_id;

INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id)
SELECT repair.revision_id, kind.name, audience.audience_id
FROM _slug_repairs repair
JOIN post_audiences audience ON audience.post_id = repair.post_id
JOIN target_kinds kind ON kind.kind_id = audience.target_kind_id;

INSERT INTO post_media (
    post_id, subject_kind, revision_id, source, sha256, filename,
    reference_kind, reference_form
)
SELECT
    media.post_id, 'revision', repair.revision_id, media.source, media.sha256,
    media.filename, media.reference_kind, media.reference_form
FROM _slug_repairs repair
JOIN post_media media
  ON media.post_id = repair.post_id
 AND media.subject_kind = 'current'
 AND media.revision_id = 0;

DROP INDEX posts_user_date_slug;
UPDATE posts post
SET slug = repair.new_slug,
    updated_at = repair.new_updated_at
FROM _slug_repairs repair
WHERE repair.post_id = post.post_id;
CREATE UNIQUE INDEX posts_user_slug
    ON posts (user_id, slug)
    WHERE deleted_at IS NULL;

DELETE FROM feed_cache WHERE EXISTS (SELECT 1 FROM _slug_repairs);
