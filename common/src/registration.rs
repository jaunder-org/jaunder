//! The site's user-registration access policy — a wire+DB string enum shared by
//! `storage` (persists it as `site.registration_policy`) and `web` (returns it
//! typed from `get_registration_policy`). Rides the `StrEnum` trailer (#562).

use macros::StrEnum;

/// The site's user-registration access policy.
///
/// Wire/DB tokens: `"open"` / `"invite_only"` / `"closed"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
#[str_enum(serde)]
pub enum RegistrationPolicy {
    /// Anyone may register without a code.
    Open,
    /// New accounts require a valid, unused invite code.
    InviteOnly,
    /// Registration is disabled; no new accounts can be created.
    Closed,
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
        assert!("unknown".parse::<RegistrationPolicy>().is_err());
        // The camelCase default would be "inviteonly"; the rename must reject it.
        assert!("inviteonly".parse::<RegistrationPolicy>().is_err());
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
        // Guards the StrEnum snake_case default: the DB value is `invite_only`, not `inviteonly`.
        assert_eq!(RegistrationPolicy::InviteOnly.as_str(), "invite_only");
        assert_eq!(
            serde_json::to_string(&RegistrationPolicy::InviteOnly).unwrap(),
            "\"invite_only\""
        );
        let back: RegistrationPolicy = serde_json::from_str("\"invite_only\"").unwrap();
        assert_eq!(back, RegistrationPolicy::InviteOnly);
    }
}
