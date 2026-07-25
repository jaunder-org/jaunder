//! A tiny declarative helper for the residual per-enum boilerplate that `strum`'s
//! `parse_err_ty`/`parse_err_fn` reference but don't generate: the unit error type
//! and its `&str -> Err` constructor.
//!
//! Introduced in #607 once the completed StrEnum→strum migration showed the block
//! repeated verbatim across six enums (the ADR-0075 non-goal's "reconsider a tiny
//! shared helper only with data" condition). This is **not** a return to `StrEnum`:
//! it declares only the error type + parse fn; each enum still uses plain `strum`
//! derives and points its `parse_err_fn` at the generated function.

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
