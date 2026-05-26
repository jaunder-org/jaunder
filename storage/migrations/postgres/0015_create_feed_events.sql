CREATE TABLE feed_events (
    id              BIGSERIAL PRIMARY KEY,
    feed_url        TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    regenerated_at  TIMESTAMPTZ,
    pinged_at       TIMESTAMPTZ
);

CREATE INDEX idx_feed_events_status_next_attempt ON feed_events(status, next_attempt_at);
CREATE INDEX idx_feed_events_feed_url_status ON feed_events(feed_url, status);
