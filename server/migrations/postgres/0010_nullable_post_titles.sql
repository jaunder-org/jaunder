UPDATE posts SET title = NULL WHERE title = '';
UPDATE post_revisions SET title = NULL WHERE title = '';

ALTER TABLE posts ALTER COLUMN title DROP NOT NULL;
ALTER TABLE post_revisions ALTER COLUMN title DROP NOT NULL;
