CREATE TABLE post_audiences (
    post_id        BIGINT NOT NULL REFERENCES posts(post_id),
    target_kind_id BIGINT NOT NULL REFERENCES target_kinds(kind_id),
    audience_id    BIGINT REFERENCES audiences(audience_id)
);
-- named rows: one per (post, audience); non-named rows: one per (post, kind).
CREATE UNIQUE INDEX post_audiences_named
    ON post_audiences (post_id, audience_id) WHERE audience_id IS NOT NULL;
CREATE UNIQUE INDEX post_audiences_builtin
    ON post_audiences (post_id, target_kind_id) WHERE audience_id IS NULL;
CREATE INDEX idx_post_audiences_kind_post ON post_audiences (target_kind_id, post_id);
