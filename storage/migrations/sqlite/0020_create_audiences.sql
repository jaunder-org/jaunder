CREATE TABLE audiences (
    audience_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    author_user_id INTEGER NOT NULL REFERENCES users(user_id),
    name           TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (author_user_id, name),
    UNIQUE (audience_id, author_user_id)
);

CREATE TABLE audience_members (
    audience_id     INTEGER NOT NULL,
    subscription_id INTEGER NOT NULL,
    author_user_id  INTEGER NOT NULL,
    PRIMARY KEY (audience_id, subscription_id),
    FOREIGN KEY (audience_id, author_user_id)
        REFERENCES audiences (audience_id, author_user_id),
    FOREIGN KEY (subscription_id, author_user_id)
        REFERENCES subscriptions (subscription_id, author_user_id)
);
