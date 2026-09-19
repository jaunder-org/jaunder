-- Rendered titles are nullable because titleless Posts have no derivative. A present
-- empty fragment remains distinct from NULL for authored titles with no visible output.
ALTER TABLE posts ADD COLUMN rendered_title TEXT;
ALTER TABLE post_revisions ADD COLUMN rendered_title TEXT;

-- This raw internal marker survives an interrupted bounded application backfill.
INSERT INTO site_config (key, value)
VALUES ('migration.0038.rendered_title_backfill_pending', '1');
