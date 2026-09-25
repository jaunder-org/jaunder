-- PostgreSQL reserves pre-render Post IDs from the existing posts.post_id sequence.
INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
