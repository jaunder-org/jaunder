//! The site's user-registration access policy — a wire+DB string enum shared by
//! `storage` (persists it as `site.registration_policy`, read typed via
//! `SiteConfigStorage::get_registration_policy`) and `web` (returns it typed from
//! `get_registration_policy`). A closed string enum (`#[text_enum]`, ADR-0075 as
//! amended by #746).

/// The site's user-registration access policy.
///
/// `serialize_all = "snake_case"` gives the wire/DB tokens `"open"` /
/// `"invite_only"` / `"closed"`. The attribute injects strum's
/// token/`Display`/`FromStr` derives and generates the named
/// `InvalidRegistrationPolicy` plus the serde bridge, whose deserialize routes
/// `String` → `FromStr` so a bad wire value surfaces the domain error rather than
/// serde's generic message. Not `sqlx`: storage persists it through
/// `site.registration_policy` as a plain config string, not a typed column.
#[macros::text_enum(
    error = InvalidRegistrationPolicy,
    message = "registration policy must be \"open\", \"invite_only\", or \"closed\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
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
