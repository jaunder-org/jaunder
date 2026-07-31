//! The residual parse-error boilerplate for the `strum` string enums that have not
//! yet moved to `#[text_enum]`.
//!
//! Introduced in #607 once the completed StrEnum→strum migration showed the blocks
//! repeated across the enums (ADR-0075's "reconsider a shared helper only with data"
//! condition). `impl_string_serde_proxy!` lived here too until #746, whose
//! `#[text_enum]` attribute generates the serde bridge directly and retired the last
//! of its four users; this module goes with the last `parse_error!` caller.

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

pub(crate) use parse_error;
