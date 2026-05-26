CREATE TABLE feed_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_url        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    next_attempt_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    claimed_at      TIMESTAMP,
    created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    regenerated_at  TIMESTAMP,
    pinged_at       TIMESTAMP
);

CREATE INDEX idx_feed_events_status_next_attempt ON feed_events(status, next_attempt_at);
CREATE INDEX idx_feed_events_feed_url_status ON feed_events(feed_url, status);
