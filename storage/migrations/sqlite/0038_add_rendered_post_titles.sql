-- Rendered titles are nullable because titleless Posts have no derivative. A present
-- empty fragment remains distinct from NULL for authored titles with no visible output.
ALTER TABLE posts ADD COLUMN rendered_title TEXT;
ALTER TABLE post_revisions ADD COLUMN rendered_title TEXT;
