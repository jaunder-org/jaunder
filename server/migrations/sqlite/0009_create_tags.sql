CREATE TABLE IF NOT EXISTS tags (
    tag_id INTEGER PRIMARY KEY AUTOINCREMENT,
    tag_slug TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS post_tags (
    post_id INTEGER NOT NULL REFERENCES posts(post_id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(tag_id) ON DELETE CASCADE,
    tag_display TEXT NOT NULL,
    UNIQUE (post_id, tag_id)
);

CREATE INDEX IF NOT EXISTS post_tags_tag_id ON post_tags(tag_id);
CREATE INDEX IF NOT EXISTS post_tags_post_id ON post_tags(post_id);
