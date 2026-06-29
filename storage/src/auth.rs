use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use thiserror::Error;

use crate::SiteConfigStorage;

// ---------------------------------------------------------------------------
// RegistrationPolicy
// ---------------------------------------------------------------------------

/// The site's user-registration access policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Anyone may register without a code.
    Open,
    /// New accounts require a valid, unused invite code.
    InviteOnly,
    /// Registration is disabled; no new accounts can be created.
    Closed,
}

impl fmt::Display for RegistrationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationPolicy::Open => write!(f, "open"),
            RegistrationPolicy::InviteOnly => write!(f, "invite_only"),
            RegistrationPolicy::Closed => write!(f, "closed"),
        }
    }
}

/// Error returned when a string does not name a valid [`RegistrationPolicy`].
#[derive(Debug, Error)]
#[error("invalid registration policy: {0:?}")]
pub struct InvalidRegistrationPolicy(String);

impl FromStr for RegistrationPolicy {
    type Err = InvalidRegistrationPolicy;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(RegistrationPolicy::Open),
            "invite_only" => Ok(RegistrationPolicy::InviteOnly),
            "closed" => Ok(RegistrationPolicy::Closed),
            other => Err(InvalidRegistrationPolicy(other.to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// load_registration_policy
// ---------------------------------------------------------------------------

/// Reads `site.registration_policy` from the config store and parses it.
///
/// Returns [`RegistrationPolicy::Closed`] when the key is absent or its
/// value cannot be parsed — a safe default that prevents unintended open
/// registration on a freshly initialised instance.
pub async fn load_registration_policy(store: &dyn SiteConfigStorage) -> RegistrationPolicy {
    store
        .get("site.registration_policy")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RegistrationPolicy::Closed)
}

// ---------------------------------------------------------------------------
// Token generation / hashing
// ---------------------------------------------------------------------------

/// Generates an opaque session token: 32 cryptographically random bytes encoded
/// as base64url without padding (43 characters).
#[must_use]
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hashes a raw token using SHA-256 and returns the base64url-encoded digest.
///
/// This is used to store hashes of opaque tokens (sessions, invites) so that
/// the raw token is never persisted.
///
/// # Errors
///
/// Returns an error if the `raw_token` is not valid base64url.
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
    use crate::test_support::{backends, Backend};
    use rstest::*;
    use rstest_reuse::*;

    // --- FromStr / Display ---

    #[test]
    fn open_parses() {
        assert_eq!(
            "open".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Open
        );
    }

    #[test]
    fn invite_only_parses() {
        assert_eq!(
            "invite_only".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::InviteOnly
        );
    }

    #[test]
    fn closed_parses() {
        assert_eq!(
            "closed".parse::<RegistrationPolicy>().unwrap(),
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
                policy.to_string().parse::<RegistrationPolicy>().unwrap(),
                policy
            );
        }
    }

    // --- load_registration_policy ---

    #[apply(backends)]
    #[tokio::test]
    async fn absent_key_returns_closed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Closed
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn key_set_to_open_returns_open(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store.set("site.registration_policy", "open").await.unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Open
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn key_set_to_invite_only_returns_invite_only(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set("site.registration_policy", "invite_only")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::InviteOnly
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn invalid_value_in_db_returns_closed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set("site.registration_policy", "garbage")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Closed
        );
    }

    // --- generate_token / hash_token ---

    #[test]
    fn generate_token_returns_non_empty_string() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert!(!t1.is_empty());
        assert!(!t2.is_empty());
        assert_ne!(t1, t2);
    }

    #[test]
    fn hash_token_roundtrips() {
        let raw = generate_token();
        let hash = hash_token(&raw).unwrap();
        assert!(!hash.is_empty());
        assert_ne!(raw, hash);
    }

    #[test]
    fn hash_token_rejects_invalid_base64() {
        assert!(hash_token("not base64!").is_err());
    }
}
