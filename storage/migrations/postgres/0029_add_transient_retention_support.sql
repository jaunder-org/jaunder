ALTER TABLE feed_events ADD COLUMN terminal_at TIMESTAMPTZ;

UPDATE feed_events
SET terminal_at = CASE
    WHEN status = 'done' THEN COALESCE(pinged_at, NOW())
    WHEN status = 'failed' THEN NOW()
END
WHERE status IN ('done', 'failed');

CREATE INDEX idx_idempotency_keys_created_at ON idempotency_keys(created_at);
CREATE INDEX idx_invites_expires_at ON invites(expires_at);
CREATE INDEX idx_invites_used_at ON invites(used_at);
CREATE INDEX idx_email_verifications_expires_at ON email_verifications(expires_at);
CREATE INDEX idx_email_verifications_used_at ON email_verifications(used_at);
CREATE INDEX idx_password_resets_expires_at ON password_resets(expires_at);
CREATE INDEX idx_password_resets_used_at ON password_resets(used_at);
CREATE INDEX idx_feed_events_terminal_retention ON feed_events(status, terminal_at);
