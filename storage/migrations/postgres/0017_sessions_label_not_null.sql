UPDATE sessions SET label = 'Unknown device' WHERE label IS NULL;
ALTER TABLE sessions ALTER COLUMN label SET NOT NULL;
