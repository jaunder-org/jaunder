PRAGMA foreign_keys = OFF;

DROP INDEX IF EXISTS posts_user_date_slug;
DROP INDEX IF EXISTS post_tags_tag_id;
DROP INDEX IF EXISTS post_tags_post_id;

ALTER TABLE post_tags RENAME TO post_tags_old;

ALTER TABLE posts RENAME TO posts_old;

CREATE TABLE posts (
    post_id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(user_id),
    title TEXT,
    slug TEXT NOT NULL,
    body TEXT NOT NULL,
    format TEXT NOT NULL,
    rendered_html TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    published_at TEXT,
    deleted_at TEXT
);

INSERT INTO posts (
    post_id,
    user_id,
    title,
    slug,
    body,
    format,
    rendered_html,
    created_at,
    updated_at,
    published_at,
    deleted_at
)
SELECT
    post_id,
    user_id,
    NULLIF(title, ''),
    slug,
    body,
    format,
    rendered_html,
    created_at,
    updated_at,
    published_at,
    deleted_at
FROM posts_old;

DROP TABLE posts_old;

CREATE UNIQUE INDEX posts_user_date_slug
    ON posts (user_id, date(COALESCE(published_at, created_at)), slug)
    WHERE deleted_at IS NULL;

CREATE TABLE post_tags (
    post_id INTEGER NOT NULL REFERENCES posts(post_id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
    tag_display TEXT NOT NULL,
    UNIQUE (post_id, tag_id)
);

INSERT INTO post_tags (post_id, tag_id, tag_display)
SELECT post_id, tag_id, tag_display
FROM post_tags_old;

DROP TABLE post_tags_old;

CREATE INDEX post_tags_tag_id ON post_tags(tag_id);
CREATE INDEX post_tags_post_id ON post_tags(post_id);

ALTER TABLE post_revisions RENAME TO post_revisions_old;

CREATE TABLE post_revisions (
    revision_id INTEGER PRIMARY KEY AUTOINCREMENT,
    post_id INTEGER NOT NULL REFERENCES posts(post_id),
    user_id INTEGER NOT NULL REFERENCES users(user_id),
    title TEXT,
    slug TEXT NOT NULL,
    body TEXT NOT NULL,
    format TEXT NOT NULL,
    rendered_html TEXT NOT NULL,
    edited_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

INSERT INTO post_revisions (
    revision_id,
    post_id,
    user_id,
    title,
    slug,
    body,
    format,
    rendered_html,
    edited_at
)
SELECT
    revision_id,
    post_id,
    user_id,
    NULLIF(title, ''),
    slug,
    body,
    format,
    rendered_html,
    edited_at
FROM post_revisions_old;

DROP TABLE post_revisions_old;

PRAGMA foreign_keys = ON;
