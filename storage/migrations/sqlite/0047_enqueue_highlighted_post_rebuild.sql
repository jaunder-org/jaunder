-- Rebuild current Post projections with the host's code-block syntax highlighter.
INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
