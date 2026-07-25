//! Tiny declarative helpers for the residual boilerplate that `strum`-derived
//! string enums repeat verbatim — the pieces `strum` references but doesn't
//! generate.
//!
//! Introduced in #607 once the completed StrEnum→strum migration showed the
//! blocks repeated across the enums (ADR-0075's "reconsider a shared helper only
//! with data" condition). These are **not** a return to `StrEnum`: each enum keeps
//! plain `strum` derives and points its `parse_err_fn` / serde `into`/`try_from`
//! at what these macros generate.

/// Declares a unit parse-error type — `#[derive(Debug, Clone, Copy, PartialEq, Eq,
/// thiserror::Error)]` with message `$msg` — plus the private `$parse_fn(&str) ->
/// $err` that a sibling enum's `#[strum(parse_err_fn = …)]` points at.
macro_rules! parse_error {
    ($err:ident, $parse_fn:ident, $msg:literal) => {
        /// Parse error for the sibling string-token enum (see its `parse_err_ty`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
        #[error($msg)]
        pub struct $err;

        fn $parse_fn(_: &str) -> $err {
            $err
        }
    };
}

/// Implements the serde `into`/`try_from = "String"` proxy for a `strum` string
/// enum: `From<$ty> for String` (serialize the token) and `TryFrom<String>`
/// (deserialize through `FromStr`, surfacing the enum's own named parse error —
/// `<$ty as FromStr>::Err`). Requires the enum to derive `strum::AsRefStr` +
/// `strum::EnumString` and carry `#[serde(into = "String", try_from = "String")]`.
macro_rules! impl_string_serde_proxy {
    ($ty:ty) => {
        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.as_ref().to_owned()
            }
        }

        impl TryFrom<String> for $ty {
            type Error = <$ty as ::std::str::FromStr>::Err;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }
    };
}

pub(crate) use impl_string_serde_proxy;
pub(crate) use parse_error;
