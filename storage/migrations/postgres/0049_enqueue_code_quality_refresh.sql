-- Fresh durable request even when the earlier renderer rebuilds and bounded
-- refresh have completed. The host performs both operations before traffic;
-- the offline rebuild is separate from the bounded active-Post pass.
INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
UPDATE post_projection_refresh_progress
SET version = 2, cursor_post_id = 0, completed = FALSE
WHERE id = 1 AND version = 1;
