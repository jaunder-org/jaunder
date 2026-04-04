// RegistrationPolicy, load_registration_policy live in `common` (shared by
// both `web` and `server`).  AuthUser and require_auth are defined in `web`
// (they use Leptos/Axum types) and re-exported here for server-crate callers.
pub use common::auth::{load_registration_policy, InvalidRegistrationPolicy, RegistrationPolicy};
pub use web::auth::{require_auth, AuthUser};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;

/// Generates an opaque session token: 32 cryptographically random bytes encoded
/// as base64url without padding (43 characters).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hashes a raw token using SHA-256 and returns the base64url-encoded digest.
///
/// This is used to store hashes of opaque tokens (sessions, invites) so that
/// the raw token is never persisted.
pub fn hash_token(raw_token: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let bytes = URL_SAFE_NO_PAD
        .decode(raw_token)
        .map_err(|e| e.to_string())?;
    let hash = Sha256::digest(&bytes);
    Ok(URL_SAFE_NO_PAD.encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{SiteConfigStorage, SqliteSiteConfigStorage};

    // --- FromStr / Display ---

    #[test]
    fn open_parses() {
        assert_eq!(
            "open"
                .parse::<RegistrationPolicy>()
                .expect("\"open\" is a valid RegistrationPolicy"),
            RegistrationPolicy::Open
        );
    }

    #[test]
    fn invite_only_parses() {
        assert_eq!(
            "invite_only"
                .parse::<RegistrationPolicy>()
                .expect("\"invite_only\" is a valid RegistrationPolicy"),
            RegistrationPolicy::InviteOnly
        );
    }

    #[test]
    fn closed_parses() {
        assert_eq!(
            "closed"
                .parse::<RegistrationPolicy>()
                .expect("\"closed\" is a valid RegistrationPolicy"),
            RegistrationPolicy::Closed
        );
    }

    #[test]
    fn unknown_string_returns_error() {
        assert!("unknown".parse::<RegistrationPolicy>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for policy in [
            RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed,
        ] {
            assert_eq!(
                policy
                    .to_string()
                    .parse::<RegistrationPolicy>()
                    .expect("Display output should round-trip through FromStr"),
                policy
            );
        }
    }

    // --- load_registration_policy ---

    async fn in_memory_store() -> SqliteSiteConfigStorage {
        let pool = sqlx::SqlitePool::connect(":memory:")
            .await
            .expect("in-memory SQLite pool should open");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations should run on in-memory pool");
        SqliteSiteConfigStorage::new(pool)
    }

    #[tokio::test]
    async fn absent_key_returns_closed() {
        let store = in_memory_store().await;
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }

    #[tokio::test]
    async fn key_set_to_open_returns_open() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "open")
            .await
            .expect("set should succeed");
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Open
        );
    }

    #[tokio::test]
    async fn key_set_to_invite_only_returns_invite_only() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "invite_only")
            .await
            .expect("set should succeed");
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::InviteOnly
        );
    }

    #[tokio::test]
    async fn invalid_value_in_db_returns_closed() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "garbage")
            .await
            .expect("set should succeed");
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }
}
