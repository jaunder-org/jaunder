use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A UTC instant that crosses the web `#[server]` boundary as a domain value
/// instead of a bare RFC 3339 `String` (ADR-0063; see
/// `docs/adr/0072-timestamps-cross-boundary-as-utcinstant.md`).
///
/// It wraps [`chrono::DateTime<Utc>`] and is **serde-transparent**: `#[derive]`d
/// `Serialize`/`Deserialize` route straight through chrono's own serde (the
/// `serde` feature is enabled on `common`'s `chrono` dependency), so it
/// (de)serializes exactly as a `DateTime<Utc>` — an RFC 3339 string, wire-compatible
/// with the `String` fields it replaces. chrono's `Deserialize` normalizes any offset
/// to UTC and rejects malformed input, so decode-time wire validation comes for free.
///
/// The only hand-written piece is [`FromStr`] — the chokepoint the client
/// `Field<UtcInstant>`/`ValidatedInput<UtcInstant>` path (ADR-0065) needs, where a
/// browser control yields a *string*. It parses RFC 3339 and canonicalizes to UTC so
/// an offset-bearing input compares equal to its `Z`-form equivalent. `chrono` is
/// already compiled into the CSR/wasm bundle via `common`, so this type is expressible
/// in a `#[server]` signature on both the server and the wasm client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtcInstant(DateTime<Utc>);

/// Error returned when a string cannot be parsed as a [`UtcInstant`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid RFC 3339 timestamp")]
pub struct InvalidInstant;

impl UtcInstant {
    /// The inner `DateTime<Utc>` — the ADR-0063 `value()` accessor convention (by
    /// value; `UtcInstant` is `Copy`). Use `DateTime::from(x)` / `x.into()` where that
    /// reads better than `x.value()`.
    #[must_use]
    pub fn value(self) -> DateTime<Utc> {
        self.0
    }
}

impl From<DateTime<Utc>> for UtcInstant {
    fn from(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }
}

impl From<UtcInstant> for DateTime<Utc> {
    fn from(instant: UtcInstant) -> Self {
        instant.0
    }
}

impl FromStr for UtcInstant {
    type Err = InvalidInstant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse RFC 3339 (any offset) and canonicalize to UTC, so a value entered as
        // `…+05:00` and its `…Z` equivalent are the same instant.
        DateTime::parse_from_rfc3339(s)
            .map(|dt| Self(dt.with_timezone(&Utc)))
            .map_err(|_| InvalidInstant)
    }
}

impl fmt::Display for UtcInstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Canonical `Z`-suffixed RFC 3339 UTC (matches the datetime control's
        // `Date.toISOString()`); `to_rfc3339()` alone would emit `+00:00`.
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_an_rfc3339_z_string() {
        assert!("2026-07-19T10:30:00Z".parse::<UtcInstant>().is_ok());
    }

    #[test]
    fn parses_and_canonicalizes_an_offset_to_utc() {
        let a = "2026-07-19T15:30:00+05:00".parse::<UtcInstant>().unwrap();
        let b = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
        assert_eq!(a, b); // same instant, regardless of the wire offset
    }

    #[test]
    fn from_str_rejects_malformed_input() {
        assert_eq!("not-a-time".parse::<UtcInstant>(), Err(InvalidInstant));
        assert_eq!("2026-13-99".parse::<UtcInstant>(), Err(InvalidInstant));
    }

    #[test]
    fn serde_round_trips_preserving_the_instant() {
        let x = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
        let json = serde_json::to_string(&x).unwrap();
        assert_eq!(serde_json::from_str::<UtcInstant>(&json).unwrap(), x);
    }

    #[test]
    fn serializes_as_a_bare_rfc3339_string_not_an_object() {
        let x = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
        let json = serde_json::to_string(&x).unwrap();
        assert!(json.starts_with('"'), "not a JSON string: {json}");
        assert!(
            json.contains("2026-07-19T10:30:00"),
            "unexpected form: {json}"
        );
        assert!(!json.contains('{'), "serialized as an object: {json}");
    }

    #[test]
    fn serde_deserialize_of_an_invalid_string_errors() {
        assert!(serde_json::from_str::<UtcInstant>("\"not-a-time\"").is_err());
    }

    #[test]
    fn value_returns_the_wrapped_utc_instant() {
        let x = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
        assert_eq!(
            x.value(),
            Utc.with_ymd_and_hms(2026, 7, 19, 10, 30, 0).unwrap()
        );
    }

    #[test]
    fn display_emits_an_rfc3339_utc_string_that_round_trips() {
        let x = "2026-07-19T10:30:00Z".parse::<UtcInstant>().unwrap();
        assert_eq!(x.to_string(), "2026-07-19T10:30:00Z");
        assert_eq!(x.to_string().parse::<UtcInstant>().unwrap(), x);
    }

    #[test]
    fn from_datetime_wraps() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 19, 10, 30, 0).unwrap();
        assert_eq!(UtcInstant::from(dt).value(), dt);
    }

    #[test]
    fn into_inner_datetime_via_from() {
        let dt = Utc.with_ymd_and_hms(2026, 7, 19, 10, 30, 0).unwrap();
        assert_eq!(DateTime::<Utc>::from(UtcInstant::from(dt)), dt);
    }
}
