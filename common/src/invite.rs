//! Invite-code value types.
//!
//! An invite code is a high-entropy base64url capability token that authorizes
//! registration (ADR-0063). Two newtypes split it by trust and direction:
//! [`ProfferedInviteCode`] here in `common` is the raw value a **client submits**
//! (wasm-constructible, serde-capable, inbound-only); the validated domain type
//! `host::invite::InviteCode` — server-only, serde-free — is what the rest of the
//! server speaks. Both delegate their shape check to [`crate::token::validate_shape`]
//! (the token-general invariant), so the two types cannot drift.

use std::str::FromStr;

use macros::{NumNewtype, StrNewtype};

use crate::token::{validate_shape, InvalidTokenShape};

/// A raw invite code as **submitted by a client** during registration.
///
/// Secret-bearing and serde-inbound per ADR-0063 (`#[str_newtype(secret, serde)]`):
/// redacting `Debug`, `AsRef<str>`, `TryFrom<String>`, and the validating serde
/// bridge — but no `Display`/`Deref`/owned-`String`/`PartialEq`. It exists only to be
/// validated (client-side per ADR-0065, and again on the wire at deserialize), travel
/// client→server, and be converted into `host::invite::InviteCode`. An xtask gate pins
/// it to `#[server]` parameter positions so a raw code is never sent back to a client.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct ProfferedInviteCode(String);

impl FromStr for ProfferedInviteCode {
    type Err = InvalidTokenShape;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(ProfferedInviteCode(s.to_owned()))
    }
}

/// Hours until an invite code expires — bounded `1..=336` (14 days), default 168 (7 days).
///
/// The bound that `create_invite` (web and CLI) used to enforce in-body now lives in the type.
/// The `i64` inner feeds `chrono::Duration::hours` directly, and the `max = 336` keeps it far
/// from overflow; the `NumNewtype` trailer rejects a non-integer, a negative, `0`, `> 336`, and
/// (at serde/`FromStr`) a `u64::MAX`-shaped value that doesn't fit `i64`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 1,
    max = 336,
    default = 168,
    error = "invite expiry must be between 1 and 336 hours"
)]
pub struct InviteTtlHours(i64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proffered_from_str_valid_and_invalid() {
        assert!("roundTrip-code_42".parse::<ProfferedInviteCode>().is_ok());
        assert!("bad code".parse::<ProfferedInviteCode>().is_err());
        assert!("".parse::<ProfferedInviteCode>().is_err());
    }

    #[test]
    fn proffered_serde_roundtrips_and_validates_on_the_wire() {
        let p: ProfferedInviteCode = "code123".parse().unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"code123\"");
        let back: ProfferedInviteCode = serde_json::from_str("\"code123\"").unwrap();
        assert_eq!(back.as_ref(), "code123");
        // Deserialize routes through validate_shape, so invalid input is rejected on the wire.
        assert!(serde_json::from_str::<ProfferedInviteCode>("\"a b\"").is_err());
    }

    #[test]
    fn proffered_debug_is_redacted() {
        let raw = "secretcode012ABC";
        let p: ProfferedInviteCode = raw.parse().unwrap();
        let out = format!("{p:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "ProfferedInviteCode([redacted])");
    }

    #[test]
    fn invite_ttl_hours_surface() {
        // value()/From<Self>, trim, in-range parse.
        assert_eq!(
            "168".parse::<InviteTtlHours>().map(i64::from).ok(),
            Some(168)
        );
        assert_eq!(
            "  1  "
                .parse::<InviteTtlHours>()
                .map(InviteTtlHours::value)
                .ok(),
            Some(1)
        );
        assert_eq!(
            "336"
                .parse::<InviteTtlHours>()
                .map(InviteTtlHours::value)
                .ok(),
            Some(336)
        );
        // FromStr rejects out-of-range / non-integer / u64::MAX (doesn't fit i64)...
        for bad in ["0", "337", "-1", "abc", "1.5", "18446744073709551615"] {
            assert!(
                bad.parse::<InviteTtlHours>().is_err(),
                "{bad} should reject"
            );
        }
        // ...with the domain message.
        assert!("0"
            .parse::<InviteTtlHours>()
            .err()
            .is_some_and(|e| e.to_string().starts_with("invite expiry")));
        // Default is 168 and Display round-trips.
        let d = InviteTtlHours::default();
        assert_eq!(d.value(), 168);
        assert_eq!(d.to_string().parse::<InviteTtlHours>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of out-of-range.
        assert_eq!(serde_json::to_string(&d).ok(), Some("168".to_owned()));
        assert_eq!(
            serde_json::from_str::<InviteTtlHours>("24")
                .map(i64::from)
                .ok(),
            Some(24)
        );
        assert!(serde_json::from_str::<InviteTtlHours>("0").is_err());
        assert!(serde_json::from_str::<InviteTtlHours>("337").is_err());
        // The generated TryFrom<i64>.
        assert_eq!(InviteTtlHours::try_from(48_i64).map(i64::from), Ok(48));
        assert!(InviteTtlHours::try_from(0_i64).is_err());
        // The shared fixture.
        assert_eq!(
            crate::test_support::parse_invite_ttl_hours("48").value(),
            48
        );
    }
}
