//! The site's user-registration access policy — a wire+DB string enum shared by
//! `storage` (persists it as `site.registration_policy`, read typed via
//! `SiteConfigStorage::get_registration_policy`) and `web` (returns it typed from
//! `get_registration_policy`). A `strum` string enum (ADR-0075).

use crate::parse_error::parse_error;

/// The site's user-registration access policy.
///
/// A `strum` string enum: `serialize_all = "snake_case"` gives the wire/DB
/// tokens `"open"` / `"invite_only"` / `"closed"`, with a named
/// `InvalidRegistrationPolicy` parse error via `parse_err_ty`/`parse_err_fn`.
/// serde routes through an owned-`String` proxy (`into`/`try_from`) so a bad
/// wire value surfaces the domain error rather than serde's generic message.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    strum::AsRefStr,
    strum::Display,
    strum::EnumString,
)]
#[serde(into = "String", try_from = "String")]
#[strum(serialize_all = "snake_case")]
#[strum(parse_err_ty = InvalidRegistrationPolicy, parse_err_fn = registration_policy_parse_err)]
pub enum RegistrationPolicy {
    /// Anyone may register without a code.
    Open,
    /// New accounts require a valid, unused invite code.
    InviteOnly,
    /// Registration is disabled; no new accounts can be created.
    Closed,
}

parse_error!(
    InvalidRegistrationPolicy,
    registration_policy_parse_err,
    "registration policy must be \"open\", \"invite_only\", or \"closed\""
);

// serde `into`/`try_from` proxy: serialize the wire token, deserialize an owned
// `String` through `FromStr` so the domain `InvalidRegistrationPolicy` message surfaces.
impl From<RegistrationPolicy> for String {
    fn from(policy: RegistrationPolicy) -> Self {
        policy.as_ref().to_owned()
    }
}

impl TryFrom<String> for RegistrationPolicy {
    type Error = InvalidRegistrationPolicy;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_parse() {
        assert_eq!(
            "open".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Open
        );
        assert_eq!(
            "invite_only".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::InviteOnly
        );
        assert_eq!(
            "closed".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Closed
        );
    }

    #[test]
    fn unknown_token_is_error() {
        // The camelCase default would be "inviteonly"; the rename must reject it.
        assert!("inviteonly".parse::<RegistrationPolicy>().is_err());
        // An unknown token surfaces the named error with its specific message.
        let err = "unknown".parse::<RegistrationPolicy>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "registration policy must be \"open\", \"invite_only\", or \"closed\""
        );
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

    #[test]
    fn invite_only_wire_token_is_snake_case() {
        // Guards the snake_case token: the DB value is `invite_only`, not `inviteonly`.
        assert_eq!(RegistrationPolicy::InviteOnly.as_ref(), "invite_only");
        assert_eq!(
            serde_json::to_string(&RegistrationPolicy::InviteOnly).unwrap(),
            "\"invite_only\""
        );
        let back: RegistrationPolicy = serde_json::from_str("\"invite_only\"").unwrap();
        assert_eq!(back, RegistrationPolicy::InviteOnly);
    }
}
