-- Rebuild both sides of the composite foreign key so existing audience membership
-- keeps pointing at the same subscription IDs throughout the atomic migration.
CREATE TABLE subscriptions_0026 (
    subscription_id INTEGER PRIMARY KEY AUTOINCREMENT,
    author_user_id  INTEGER NOT NULL REFERENCES users(user_id),
    channel_id      INTEGER NOT NULL REFERENCES channels(channel_id),
    subscriber_ref  TEXT NOT NULL
        CONSTRAINT subscriptions_subscriber_ref_nonempty CHECK (subscriber_ref <> ''),
    status_id       INTEGER NOT NULL REFERENCES subscription_statuses(status_id),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (author_user_id, channel_id, subscriber_ref),
    UNIQUE (subscription_id, author_user_id)
);

INSERT INTO subscriptions_0026 (
    subscription_id,
    author_user_id,
    channel_id,
    subscriber_ref,
    status_id,
    created_at
)
SELECT
    subscription_id,
    author_user_id,
    channel_id,
    subscriber_ref,
    status_id,
    created_at
FROM subscriptions;

-- Preserve AUTOINCREMENT's historical high-water mark even when the table has
-- no live rows, so rebuilding cannot make a deleted subscription ID reusable.
DELETE FROM sqlite_sequence WHERE name = 'subscriptions_0026';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'subscriptions_0026', seq
FROM sqlite_sequence
WHERE name = 'subscriptions';

CREATE TABLE audience_members_0026 (
    audience_id     INTEGER NOT NULL,
    subscription_id INTEGER NOT NULL,
    author_user_id  INTEGER NOT NULL,
    PRIMARY KEY (audience_id, subscription_id),
    FOREIGN KEY (audience_id, author_user_id)
        REFERENCES audiences (audience_id, author_user_id),
    FOREIGN KEY (subscription_id, author_user_id)
        REFERENCES subscriptions_0026 (subscription_id, author_user_id)
);

INSERT INTO audience_members_0026 (audience_id, subscription_id, author_user_id)
SELECT audience_id, subscription_id, author_user_id
FROM audience_members;

DROP TABLE audience_members;
DROP TABLE subscriptions;
ALTER TABLE subscriptions_0026 RENAME TO subscriptions;
ALTER TABLE audience_members_0026 RENAME TO audience_members;

CREATE INDEX idx_subscriptions_author_status ON subscriptions(author_user_id, status_id);
