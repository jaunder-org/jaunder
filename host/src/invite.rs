//! The validated invite-code domain type.
//!
//! [`InviteCode`] is the server-side counterpart of `common::invite::ProfferedInviteCode`:
//! a high-entropy capability token, secret-bearing and **serde-free**. Because this crate
//! is never built for wasm (ADR-0058), `InviteCode` cannot be named by client code — so
//! "a raw code never reaches a client" is a compile fact, not a convention. It is
//! constructed from a validated [`ProfferedInviteCode`] (the inbound wire value) or, for a
//! trusted stored code, via [`FromStr`]; both delegate the shape check to
//! [`common::token::validate_shape`].

use std::str::FromStr;

use common::invite::ProfferedInviteCode;
use common::token::{validate_shape, InvalidTokenShape};
use macros::StrNewtype;

/// A validated invite code held server-side.
///
/// Secret-bearing per ADR-0063 (`#[str_newtype(secret)]`): redacting `Debug`,
/// `AsRef<str>`, `TryFrom<String>` — no `Display`, no serde. Deliberate egress (the CLI
/// invitation URL, a future email) is a single explicit `code.as_ref()`.
///
/// `sqlx` re-opts the storage bridge: an `InviteCode` *is* persisted (the `invites`
/// table), so it needs `Encode`/`Decode` even though `secret` drops the bridge by
/// default (#438).
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct InviteCode(String);

impl FromStr for InviteCode {
    type Err = InvalidTokenShape;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(InviteCode(s.to_owned()))
    }
}

impl TryFrom<ProfferedInviteCode> for InviteCode {
    type Error = InvalidTokenShape;

    /// Converts a client-submitted code into the domain type. `ProfferedInviteCode` was
    /// already validated at construction, so this cannot actually fail — but it re-runs the
    /// shared validator rather than relying on that (no infallible cross-type constructor).
    fn try_from(p: ProfferedInviteCode) -> Result<Self, Self::Error> {
        p.as_ref().parse()
    }
}

/// Mints a fresh invite code: the 32-byte base64url secret from
/// [`crate::token::generate`], wrapped in the domain type without re-validation.
///
/// A freshly minted code is canonical base64url by construction, so this is the
/// trusted-mint counterpart to the validating [`FromStr`] inbound door: it lets
/// `create_invite` store a typed `InviteCode` end-to-end with no fallible
/// re-parse (#438). Mirrors `common::token::RawToken::from_generated`.
#[must_use]
pub fn generate() -> InviteCode {
    InviteCode(crate::token::generate().as_ref().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_valid_and_invalid() {
        assert!("roundTrip-code_42".parse::<InviteCode>().is_ok());
        assert!("bad code".parse::<InviteCode>().is_err());
        assert!("".parse::<InviteCode>().is_err());
    }

    #[test]
    fn try_from_proffered_round_trips() {
        let p: ProfferedInviteCode = "code123".parse().unwrap();
        let code = InviteCode::try_from(p).unwrap();
        assert_eq!(code.as_ref(), "code123");
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "secretcode012ABC";
        let code: InviteCode = raw.parse().unwrap();
        let out = format!("{code:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "InviteCode([redacted])");
    }

    #[test]
    fn as_ref_round_trips_the_code() {
        let raw = "abcABC012_-";
        let code: InviteCode = raw.parse().unwrap();
        assert_eq!(code.as_ref(), raw);
    }

    #[test]
    fn generate_mints_distinct_canonical_codes() {
        let a = generate();
        let b = generate();
        // Distinct high-entropy values.
        assert_ne!(a.as_ref(), b.as_ref());
        // A minted code is canonical, so it round-trips through the validating door.
        assert!(a.as_ref().parse::<InviteCode>().is_ok());
    }
}
