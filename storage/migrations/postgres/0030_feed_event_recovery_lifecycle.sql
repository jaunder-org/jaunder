ALTER TABLE feed_events
    ADD COLUMN phase TEXT NOT NULL DEFAULT 'regeneration',
    ADD COLUMN regeneration_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN publication_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN regeneration_diagnostic TEXT,
    ADD COLUMN publication_diagnostic TEXT;

UPDATE feed_events
SET
    phase = CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN 'publication'
        ELSE 'regeneration'
    END,
    regeneration_attempts = CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN 0
        ELSE attempts
    END,
    publication_attempts = CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN attempts
        ELSE 0
    END,
    regeneration_diagnostic = CASE
        WHEN status = 'failed' AND regenerated_at IS NULL THEN last_error
        ELSE NULL
    END,
    publication_diagnostic = CASE
        WHEN status = 'failed' AND regenerated_at IS NOT NULL THEN last_error
        ELSE NULL
    END;

ALTER TABLE feed_events
    DROP COLUMN attempts,
    DROP COLUMN last_error;

CREATE INDEX idx_feed_events_dead_letters
    ON feed_events(status, phase, terminal_at DESC, id DESC);
