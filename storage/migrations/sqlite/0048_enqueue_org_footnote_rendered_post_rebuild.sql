-- Reserve Post IDs in a short autocommit write before rendering an Org Post.
-- The counter starts above every retained Post (including Deleted Posts).
CREATE TABLE post_id_allocator (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_post_id INTEGER NOT NULL
);
INSERT INTO post_id_allocator (id, last_post_id)
SELECT 1, COALESCE(MAX(post_id), 0) FROM posts;

INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
