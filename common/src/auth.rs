use std::{fmt, str::FromStr};

use thiserror::Error;

use crate::storage::SiteConfigStorage;

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
