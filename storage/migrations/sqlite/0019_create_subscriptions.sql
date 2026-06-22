CREATE TABLE subscriptions (
    subscription_id INTEGER PRIMARY KEY AUTOINCREMENT,
    author_user_id  INTEGER NOT NULL REFERENCES users(user_id),
    channel_id      INTEGER NOT NULL REFERENCES channels(channel_id),
    subscriber_ref  TEXT NOT NULL,
    status_id       INTEGER NOT NULL REFERENCES subscription_statuses(status_id),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (author_user_id, channel_id, subscriber_ref),
    UNIQUE (subscription_id, author_user_id)
);
CREATE INDEX idx_subscriptions_author_status ON subscriptions(author_user_id, status_id);
