ALTER TABLE posts ADD COLUMN search_text TEXT;
ALTER TABLE posts ADD COLUMN mutation_version BIGINT NOT NULL DEFAULT 1;

CREATE INDEX idx_posts_manage_owner_updated
    ON posts (user_id, updated_at DESC, post_id DESC)
    WHERE deleted_at IS NULL;
