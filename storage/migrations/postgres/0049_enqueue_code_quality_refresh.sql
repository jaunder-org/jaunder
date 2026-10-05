-- Requeue the existing offline renderer after earlier requests have drained.
-- Leave the independent bounded refresh checkpoint untouched.
INSERT INTO pending_code_migrations (operation) VALUES ('rebuild_rendered_posts');
