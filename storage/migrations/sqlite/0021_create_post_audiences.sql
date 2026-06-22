CREATE TABLE post_audiences (
    post_id        INTEGER NOT NULL REFERENCES posts(post_id),
    target_kind_id INTEGER NOT NULL REFERENCES target_kinds(kind_id),
    audience_id    INTEGER REFERENCES audiences(audience_id),
    PRIMARY KEY (post_id, target_kind_id, audience_id)
);
CREATE INDEX idx_post_audiences_kind_post ON post_audiences(target_kind_id, post_id);
