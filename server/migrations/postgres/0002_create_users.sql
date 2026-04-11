CREATE TABLE users (
    user_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT,
    bio TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    last_authenticated_at TIMESTAMPTZ
);
