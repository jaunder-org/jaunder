-- Pending Rust work is a queue, not a history of completed migrations.
CREATE TABLE pending_code_migrations (
    queue_id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation TEXT NOT NULL,
    enqueued_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
