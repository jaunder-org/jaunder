-- Durable discoverable-credential identities, plus short-lived server-only ceremonies.
CREATE TABLE passkey_user_handles (
    user_id INTEGER PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
    user_handle TEXT NOT NULL UNIQUE CHECK (length(user_handle) = 32 AND user_handle NOT GLOB '*[^0-9a-f]*')
);
INSERT INTO passkey_user_handles (user_id, user_handle)
SELECT user_id, lower(hex(randomblob(16))) FROM users;

CREATE TABLE passkey_credentials (
    credential_id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    label TEXT NOT NULL CHECK (length(trim(label)) > 0),
    credential TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_used_at TEXT
);
CREATE INDEX passkey_credentials_user_created ON passkey_credentials(user_id, created_at, credential_id);

CREATE TABLE passkey_registration_ceremonies (
    handle_hash TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose = 'registration'),
    user_id INTEGER NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    session_token_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE,
    label TEXT NOT NULL CHECK (length(trim(label)) > 0),
    origin TEXT NOT NULL,
    rp_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    claimed_at TEXT
);
CREATE INDEX passkey_registration_ceremonies_cleanup ON passkey_registration_ceremonies(claimed_at, expires_at);

CREATE TABLE passkey_authentication_ceremonies (
    handle_hash TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose = 'authentication'),
    origin TEXT NOT NULL,
    rp_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    claimed_at TEXT
);
CREATE INDEX passkey_authentication_ceremonies_cleanup ON passkey_authentication_ceremonies(claimed_at, expires_at);
