CREATE TABLE subscriptions (
    subscription_id BIGSERIAL PRIMARY KEY,
    author_user_id  BIGINT NOT NULL REFERENCES users(user_id),
    channel_id      BIGINT NOT NULL REFERENCES channels(channel_id),
    subscriber_ref  TEXT NOT NULL,
    status_id       BIGINT NOT NULL REFERENCES subscription_statuses(status_id),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (author_user_id, channel_id, subscriber_ref),
    UNIQUE (subscription_id, author_user_id)
);
CREATE INDEX idx_subscriptions_author_status ON subscriptions(author_user_id, status_id);
