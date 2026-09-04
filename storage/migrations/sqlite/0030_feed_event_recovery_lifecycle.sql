CREATE TABLE feed_events_recovery (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    feed_url                TEXT NOT NULL,
    status                  TEXT NOT NULL DEFAULT 'pending',
    phase                   TEXT NOT NULL DEFAULT 'regeneration',
    regeneration_attempts   INTEGER NOT NULL DEFAULT 0,
    publication_attempts    INTEGER NOT NULL DEFAULT 0,
    regeneration_diagnostic TEXT,
    publication_diagnostic  TEXT,
    next_attempt_at         TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    claimed_at              TIMESTAMP,
    terminal_at             TIMESTAMP,
    created_at              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    regenerated_at          TIMESTAMP,
    pinged_at               TIMESTAMP
);

INSERT INTO feed_events_recovery (
    id, feed_url, status, phase, regeneration_attempts, publication_attempts,
    regeneration_diagnostic, publication_diagnostic, next_attempt_at, claimed_at,
    terminal_at, created_at, regenerated_at, pinged_at
)
SELECT
    id,
    feed_url,
    status,
    CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN 'publication'
        ELSE 'regeneration'
    END,
    CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN 0
        ELSE attempts
    END,
    CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN attempts
        ELSE 0
    END,
    CASE
        WHEN status = 'failed' AND regenerated_at IS NULL THEN last_error
        ELSE NULL
    END,
    CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN last_error
        ELSE NULL
    END,
    next_attempt_at,
    claimed_at,
    terminal_at,
    created_at,
    regenerated_at,
    pinged_at
FROM feed_events;

DROP TABLE feed_events;
ALTER TABLE feed_events_recovery RENAME TO feed_events;
CREATE INDEX idx_feed_events_status_next_attempt ON feed_events(status, next_attempt_at);
CREATE INDEX idx_feed_events_feed_url_status ON feed_events(feed_url, status);
CREATE INDEX idx_feed_events_terminal_retention ON feed_events(status, terminal_at);
CREATE INDEX idx_feed_events_dead_letters ON feed_events(status, phase, terminal_at DESC, id DESC);
