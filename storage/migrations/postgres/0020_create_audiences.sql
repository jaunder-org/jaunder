CREATE TABLE audiences (
    audience_id    BIGSERIAL PRIMARY KEY,
    author_user_id BIGINT NOT NULL REFERENCES users(user_id),
    name           TEXT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (author_user_id, name),
    UNIQUE (audience_id, author_user_id)
);

CREATE TABLE audience_members (
    audience_id     BIGINT NOT NULL,
    subscription_id BIGINT NOT NULL,
    author_user_id  BIGINT NOT NULL,
    PRIMARY KEY (audience_id, subscription_id),
    FOREIGN KEY (audience_id, author_user_id)
        REFERENCES audiences (audience_id, author_user_id),
    FOREIGN KEY (subscription_id, author_user_id)
        REFERENCES subscriptions (subscription_id, author_user_id)
);
