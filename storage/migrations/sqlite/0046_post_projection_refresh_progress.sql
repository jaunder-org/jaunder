-- Only durable progress lives in the schema; rendering runs on the host after
-- migration and before accepting traffic. The row serializes resumable batches.
CREATE TABLE post_projection_refresh_progress (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL CHECK (version > 0),
    cursor_post_id INTEGER NOT NULL DEFAULT 0 CHECK (cursor_post_id >= 0),
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1))
);
INSERT INTO post_projection_refresh_progress (id, version, cursor_post_id, completed)
VALUES (1, 1, 0, 0);
