CREATE TABLE idempotency_keys (
    idempotency_key_id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id),
    key TEXT NOT NULL,
    post_id BIGINT NOT NULL REFERENCES posts(post_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, key)
);
