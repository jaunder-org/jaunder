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

use macros::StrNewtype;

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
}
