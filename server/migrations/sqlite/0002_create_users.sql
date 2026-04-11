CREATE TABLE users (
    user_id               INTEGER PRIMARY KEY,
    username              TEXT NOT NULL UNIQUE,
    password_hash         TEXT NOT NULL,
    display_name          TEXT,
    bio                   TEXT,
    created_at            TEXT NOT NULL,
    last_authenticated_at TEXT
);
