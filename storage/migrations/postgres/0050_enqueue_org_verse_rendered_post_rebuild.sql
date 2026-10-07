-- Refresh current projections for Org verse layout and inline formatting.
INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
