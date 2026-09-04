//! The site's user-registration access policy — a wire+DB string enum shared by
//! `storage` (persists it as `site.registration_policy`, read typed via
//! `SiteConfigStorage::get_registration_policy`) and `web` (returns it typed from
//! `get_registration_policy`). A closed string enum (`#[text_enum]`, ADR-0075 as
//! amended by #746).

/// The site's user-registration and invitation-authority policy.
///
/// `serialize_all = "snake_case"` gives the wire/DB tokens `"closed"`,
/// `"operator_invites"`, `"member_invites"`, and `"open"`. The attribute injects
/// strum's token/`Display`/`FromStr` derives and generates the named
/// `InvalidRegistrationPolicy` plus the serde bridge, whose deserialize routes
/// `String` → `FromStr` so a bad wire value surfaces the domain error rather than
/// serde's generic message. Not `sqlx`: storage persists it through
/// `site.registration_policy` as a plain config string, not a typed column.
#[macros::text_enum(
    error = InvalidRegistrationPolicy,
    message = "registration policy must be \"closed\", \"operator_invites\", \"member_invites\", or \"open\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum RegistrationPolicy {
    /// Registration and invitations are disabled.
    Closed,
    /// Registration requires an invitation issued by an operator.
    OperatorInvites,
    /// Registration requires an invitation issued by any authenticated member.
    MemberInvites,
    /// Anyone may register directly; invitations are disabled.
    Open,
}

impl RegistrationPolicy {
    /// Whether registration under this policy requires an invitation.
    #[must_use]
    pub fn requires_invitation(self) -> bool {
        matches!(self, Self::OperatorInvites | Self::MemberInvites)
    }

    /// Whether a user with this authority may issue an invitation.
    #[must_use]
    pub fn may_issue_invitation(self, is_operator: bool) -> bool {
        matches!(self, Self::MemberInvites)
            || (matches!(self, Self::OperatorInvites) && is_operator)
    }

    /// Whether a user with this authority may list invitation metadata.
    #[must_use]
    pub fn may_list_invitations(self, is_operator: bool) -> bool {
        matches!(self, Self::OperatorInvites | Self::MemberInvites) && is_operator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICIES: [RegistrationPolicy; 4] = [
        RegistrationPolicy::Closed,
        RegistrationPolicy::OperatorInvites,
        RegistrationPolicy::MemberInvites,
        RegistrationPolicy::Open,
    ];

    #[test]
    fn exact_tokens_parse_display_and_serde_round_trip() {
        for (policy, token) in [
            (RegistrationPolicy::Closed, "closed"),
            (RegistrationPolicy::OperatorInvites, "operator_invites"),
            (RegistrationPolicy::MemberInvites, "member_invites"),
            (RegistrationPolicy::Open, "open"),
        ] {
            assert_eq!(token.parse::<RegistrationPolicy>().unwrap(), policy);
            assert_eq!(policy.to_string(), token);
            assert_eq!(policy.as_ref(), token);
            assert_eq!(
                serde_json::to_string(&policy).unwrap(),
                format!("\"{token}\"")
            );
            assert_eq!(
                serde_json::from_str::<RegistrationPolicy>(&format!("\"{token}\"")).unwrap(),
                policy
            );
        }
    }

    #[test]
    fn removed_and_unknown_tokens_are_rejected() {
        for token in ["invite_only", "inviteonly", "unknown"] {
            assert!(
                token.parse::<RegistrationPolicy>().is_err(),
                "{token} must reject"
            );
            assert!(
                serde_json::from_str::<RegistrationPolicy>(&format!("\"{token}\"")).is_err(),
                "{token} must reject through serde"
            );
        }
        let err = "unknown".parse::<RegistrationPolicy>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "registration policy must be \"closed\", \"operator_invites\", \"member_invites\", or \"open\""
        );
    }

    #[test]
    fn invitation_requirement_matrix() {
        for (policy, requires_invitation) in [
            (RegistrationPolicy::Closed, false),
            (RegistrationPolicy::OperatorInvites, true),
            (RegistrationPolicy::MemberInvites, true),
            (RegistrationPolicy::Open, false),
        ] {
            assert_eq!(policy.requires_invitation(), requires_invitation);
        }
    }

    #[test]
    fn invitation_issuance_matrix() {
        for (policy, is_operator, permitted) in [
            (RegistrationPolicy::Closed, false, false),
            (RegistrationPolicy::Closed, true, false),
            (RegistrationPolicy::OperatorInvites, false, false),
            (RegistrationPolicy::OperatorInvites, true, true),
            (RegistrationPolicy::MemberInvites, false, true),
            (RegistrationPolicy::MemberInvites, true, true),
            (RegistrationPolicy::Open, false, false),
            (RegistrationPolicy::Open, true, false),
        ] {
            assert_eq!(policy.may_issue_invitation(is_operator), permitted);
        }
    }

    #[test]
    fn invitation_listing_matrix() {
        for (policy, is_operator, permitted) in [
            (RegistrationPolicy::Closed, false, false),
            (RegistrationPolicy::Closed, true, false),
            (RegistrationPolicy::OperatorInvites, false, false),
            (RegistrationPolicy::OperatorInvites, true, true),
            (RegistrationPolicy::MemberInvites, false, false),
            (RegistrationPolicy::MemberInvites, true, true),
            (RegistrationPolicy::Open, false, false),
            (RegistrationPolicy::Open, true, false),
        ] {
            assert_eq!(policy.may_list_invitations(is_operator), permitted);
        }
    }

    #[test]
    fn every_policy_is_covered_by_the_matrices() {
        assert_eq!(POLICIES.len(), 4);
    }
}
