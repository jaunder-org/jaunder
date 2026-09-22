-- Repair legacy per-date slug duplicates before making the active slug namespace
-- authoritative per User. Temporary state keeps allocation independent of UPDATE
-- order; the recursive walk tries the same base, -1, -2, ... sequence as runtime.
CREATE TABLE post_permalink_aliases (
    alias_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id        INTEGER NOT NULL REFERENCES posts(post_id),
    user_id        INTEGER NOT NULL REFERENCES users(user_id),
    permalink_date TEXT NOT NULL,
    slug           TEXT NOT NULL,
    UNIQUE (user_id, permalink_date, slug)
);
CREATE INDEX post_permalink_aliases_post_id ON post_permalink_aliases(post_id);

CREATE TEMP TABLE _slug_repair_clock (observed_at TEXT NOT NULL);
INSERT INTO _slug_repair_clock
VALUES (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

CREATE TEMP TABLE _slug_repair_queue AS
WITH ranked AS (
    SELECT
        post_id,
        user_id,
        slug AS old_slug,
        date(COALESCE(published_at, created_at)) AS old_permalink_date,
        created_at,
        updated_at AS old_updated_at,
        ROW_NUMBER() OVER (
            PARTITION BY user_id, slug
            ORDER BY created_at DESC, post_id DESC
        ) AS newest_rank,
        COUNT(*) OVER (PARTITION BY user_id, slug) AS duplicate_count
    FROM posts
    WHERE deleted_at IS NULL
), repairs AS (
    SELECT * FROM ranked WHERE duplicate_count > 1 AND newest_rank > 1
)
SELECT
    ROW_NUMBER() OVER (
        ORDER BY user_id, created_at, post_id
    ) AS repair_sequence,
    post_id,
    user_id,
    old_slug,
    old_permalink_date,
    old_updated_at
FROM repairs;

CREATE TEMP TABLE _slug_repairs (
    post_id            INTEGER PRIMARY KEY,
    user_id            INTEGER NOT NULL,
    old_slug           TEXT NOT NULL,
    old_permalink_date TEXT NOT NULL,
    new_slug           TEXT NOT NULL,
    new_updated_at     TEXT NOT NULL,
    revision_id        INTEGER
);

WITH RECURSIVE allocation(
    phase,
    repair_sequence,
    attempt,
    assigned,
    candidate,
    candidate_available,
    accepted_post_id,
    accepted_user_id,
    accepted_old_slug,
    accepted_old_permalink_date,
    accepted_new_slug,
    accepted_old_updated_at
) AS (
    SELECT
        0,
        queued.repair_sequence,
        1,
        '|',
        rtrim(substr(queued.old_slug, 1, 78), '-') || '-1',
        NULL,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL
    FROM _slug_repair_queue queued
    WHERE queued.repair_sequence = 1

    UNION ALL

    -- Candidate rows become decision rows. The candidate expression and its
    -- availability test each have one owner rather than being repeated across
    -- every accepted-field projection.
    SELECT
        1,
        allocation.repair_sequence,
        allocation.attempt,
        allocation.assigned,
        allocation.candidate,
        NOT EXISTS (
            SELECT 1 FROM posts existing
            WHERE existing.deleted_at IS NULL
              AND existing.user_id = queued.user_id
              AND existing.slug = allocation.candidate
        ) AND instr(
            allocation.assigned,
            '|' || CAST(queued.user_id AS TEXT) || ':' || allocation.candidate || '|'
        ) = 0,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL
    FROM allocation
    JOIN _slug_repair_queue queued
      ON queued.repair_sequence = allocation.repair_sequence
    WHERE allocation.phase = 0

    UNION ALL

    -- Decision rows either accept and advance to the next queued repair, or
    -- retain the row and try its next suffix. Accepted data rides on the next
    -- candidate row so the final repair remains observable when no queue row follows.
    SELECT
        0,
        CASE WHEN allocation.candidate_available
             THEN allocation.repair_sequence + 1
             ELSE allocation.repair_sequence END,
        CASE WHEN allocation.candidate_available
             THEN 1
             ELSE allocation.attempt + 1 END,
        CASE WHEN allocation.candidate_available
             THEN allocation.assigned || CAST(queued.user_id AS TEXT) || ':' ||
                  allocation.candidate || '|'
             ELSE allocation.assigned END,
        CASE
            WHEN allocation.candidate_available AND next_queued.post_id IS NOT NULL
            THEN rtrim(substr(next_queued.old_slug, 1, 78), '-') || '-1'
            WHEN NOT allocation.candidate_available
            THEN rtrim(
                     substr(
                         queued.old_slug,
                         1,
                         80 - length(CAST(allocation.attempt + 1 AS TEXT)) - 1
                     ),
                     '-'
                 ) || '-' || CAST(allocation.attempt + 1 AS TEXT)
            ELSE NULL
        END,
        NULL,
        CASE WHEN allocation.candidate_available THEN queued.post_id ELSE NULL END,
        CASE WHEN allocation.candidate_available THEN queued.user_id ELSE NULL END,
        CASE WHEN allocation.candidate_available THEN queued.old_slug ELSE NULL END,
        CASE WHEN allocation.candidate_available THEN queued.old_permalink_date ELSE NULL END,
        CASE WHEN allocation.candidate_available THEN allocation.candidate ELSE NULL END,
        CASE WHEN allocation.candidate_available THEN queued.old_updated_at ELSE NULL END
    FROM allocation
    JOIN _slug_repair_queue queued
      ON queued.repair_sequence = allocation.repair_sequence
    LEFT JOIN _slug_repair_queue next_queued
      ON next_queued.repair_sequence = allocation.repair_sequence + 1
    WHERE allocation.phase = 1
)
INSERT INTO _slug_repairs (
    post_id, user_id, old_slug, old_permalink_date, new_slug, new_updated_at
)
SELECT
    accepted_post_id,
    accepted_user_id,
    accepted_old_slug,
    accepted_old_permalink_date,
    accepted_new_slug,
    CASE
        WHEN accepted_old_updated_at >= clock.observed_at
        THEN strftime('%Y-%m-%dT%H:%M:%SZ', accepted_old_updated_at, '+1 second')
        ELSE clock.observed_at
    END
FROM allocation
CROSS JOIN _slug_repair_clock clock
WHERE accepted_post_id IS NOT NULL;

INSERT INTO post_permalink_aliases (post_id, user_id, permalink_date, slug)
SELECT post_id, user_id, old_permalink_date, old_slug
FROM _slug_repairs;

INSERT INTO post_revisions (
    post_id, user_id, title, rendered_title, slug, body, format, rendered_html,
    summary, created_at, updated_at, published_at, deleted_at, captured_at
)
SELECT
    post.post_id, post.user_id, post.title, post.rendered_title, post.slug,
    post.body, post.format, post.rendered_html, post.summary, post.created_at,
    post.updated_at, post.published_at, post.deleted_at, repair.new_updated_at
FROM posts post
JOIN _slug_repairs repair ON repair.post_id = post.post_id;

UPDATE _slug_repairs
SET revision_id = (
    SELECT MAX(revision.revision_id)
    FROM post_revisions revision
    WHERE revision.post_id = _slug_repairs.post_id
);

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
UPDATE posts
SET slug = (SELECT repair.new_slug FROM _slug_repairs repair WHERE repair.post_id = posts.post_id),
    updated_at = (SELECT repair.new_updated_at FROM _slug_repairs repair WHERE repair.post_id = posts.post_id)
WHERE post_id IN (SELECT post_id FROM _slug_repairs);
CREATE UNIQUE INDEX posts_user_slug
    ON posts (user_id, slug)
    WHERE deleted_at IS NULL;

DELETE FROM feed_cache WHERE EXISTS (SELECT 1 FROM _slug_repairs);

DROP TABLE _slug_repairs;
DROP TABLE _slug_repair_queue;
DROP TABLE _slug_repair_clock;
