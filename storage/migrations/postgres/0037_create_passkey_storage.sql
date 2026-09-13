-- Durable discoverable-credential identities, plus short-lived server-only ceremonies.
CREATE TABLE passkey_user_handles (
    user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    user_handle TEXT NOT NULL UNIQUE CHECK (length(user_handle) = 32 AND user_handle ~ '^[0-9a-f]+$')
);
INSERT INTO passkey_user_handles (user_id, user_handle)
SELECT user_id, replace(gen_random_uuid()::text, '-', '') FROM users;

CREATE TABLE passkey_credentials (
    credential_id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    label TEXT NOT NULL CHECK (length(btrim(label)) > 0),
    credential TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ
);
CREATE INDEX passkey_credentials_user_created ON passkey_credentials(user_id, created_at, credential_id);

CREATE TABLE passkey_registration_ceremonies (
    handle_hash TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose = 'registration'),
    user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    session_token_hash TEXT NOT NULL REFERENCES sessions(token_hash) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
    label TEXT NOT NULL CHECK (length(btrim(label)) > 0),
    origin TEXT NOT NULL,
    rp_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ
);
CREATE INDEX passkey_registration_ceremonies_cleanup ON passkey_registration_ceremonies(claimed_at, expires_at);

CREATE TABLE passkey_authentication_ceremonies (
    handle_hash TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose = 'authentication'),
    origin TEXT NOT NULL,
    rp_id TEXT NOT NULL,
    state TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ
);
CREATE INDEX passkey_authentication_ceremonies_cleanup ON passkey_authentication_ceremonies(claimed_at, expires_at);
