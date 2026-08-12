use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
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

/// A validated permalink calendar date (`YYYY-MM-DD`), wrapping a [`chrono::NaiveDate`] so
/// an impossible date is unrepresentable — the permalink-route sibling of [`UtcInstant`]
/// (ADR-0072/0063/0065). Like `UtcInstant` it is **serde-transparent** (a newtype struct →
/// (de)serializes exactly as its inner `NaiveDate`, i.e. the ISO string `"2026-01-02"`), so
/// a corrupt wire value is a decode error, not a silent bad date. It crosses the web
/// `#[server]` boundary (`get_post`) in place of a loose `(i32, u32, u32)` triple, and is
/// assembled from the permalink URL's three `/YYYY/MM/DD/` segments via
/// [`from_ymd`](PermalinkDate::from_ymd). `chrono` is compiled into the CSR/wasm bundle via
/// `common`, so the type is expressible on both server and wasm client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermalinkDate(NaiveDate);

impl PermalinkDate {
    /// The one fallible construction door: an impossible date (bad month/day, non-existent
    /// day-of-month) yields `None`. Used to assemble the type from the URL's three parsed
    /// segments (the client parse and the server `SoftPath` route).
    #[must_use]
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, day).map(Self)
    }

    /// The inner [`NaiveDate`] — the ADR-0063 `value()` accessor (by value; `Copy`), for
    /// in-Rust date comparison.
    #[must_use]
    pub fn value(self) -> NaiveDate {
        self.0
    }
}

impl From<NaiveDate> for PermalinkDate {
    fn from(date: NaiveDate) -> Self {
        Self(date)
    }
}

impl fmt::Display for PermalinkDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `NaiveDate`'s Display is ISO `YYYY-MM-DD` (4-digit zero-padded year) — the exact
        // string the permalink storage query binds.
        write!(f, "{}", self.0)
    }
}

/// Converts a `<input type="datetime-local">` value — a naive local wall-clock such
/// as `"2026-07-01T13:30"` (seconds optional) — into a [`UtcInstant`], interpreting
/// it in the ambient local timezone. `None` for an empty/whitespace or unparseable
/// input, or a local time that doesn't exist (a DST spring-forward gap). An
/// *ambiguous* time (a fall-back fold) resolves to its earliest instant rather than
/// `None`, so a real (if doubled) wall-clock isn't lost.
///
/// Lives here beside [`UtcInstant`] (rather than in `web`) because `chrono` is
/// already wasm-available through `common`; on the browser `chrono::Local` reads the
/// user's timezone via `wasmbind`, making this a host-testable pure conversion
/// rather than `js_sys::Date` glue (ADR-0055). The boundary type stays `UtcInstant`,
/// so no `chrono` type reaches a `#[server]` signature (ADR-0072).
#[must_use]
pub fn utc_instant_from_local(local: &str) -> Option<UtcInstant> {
    utc_instant_from_local_in(local, &Local)
}

/// The timezone-parametric core of [`utc_instant_from_local`], so the local→UTC
/// conversion is host-testable against a fixed offset instead of the ambient zone.
fn utc_instant_from_local_in<Tz: TimeZone>(local: &str, tz: &Tz) -> Option<UtcInstant> {
    let trimmed = local.trim();
    if trimmed.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|dt| UtcInstant::from(dt.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn permalink_date_from_ymd_accepts_a_real_date() {
        assert!(PermalinkDate::from_ymd(2026, 1, 2).is_some());
    }

    #[test]
    fn permalink_date_from_ymd_rejects_impossible_dates() {
        for (y, m, d) in [
            (2026, 13, 1),
            (2026, 0, 1),
            (2026, 1, 0),
            (2026, 2, 30),
            (2026, 4, 31),
        ] {
            assert!(
                PermalinkDate::from_ymd(y, m, d).is_none(),
                "{y}-{m}-{d} must reject"
            );
        }
    }

    #[test]
    fn permalink_date_serde_is_transparent_iso_and_rejects_invalid() {
        let pd = PermalinkDate::from_ymd(2026, 1, 2).unwrap();
        assert_eq!(serde_json::to_string(&pd).unwrap(), "\"2026-01-02\"");
        assert_eq!(
            serde_json::from_str::<PermalinkDate>("\"2026-01-02\"").unwrap(),
            pd
        );
        assert!(serde_json::from_str::<PermalinkDate>("\"2026-13-40\"").is_err());
    }

    #[test]
    fn permalink_date_display_and_value_round_trip() {
        let pd = PermalinkDate::from_ymd(2026, 1, 2).unwrap();
        assert_eq!(pd.to_string(), "2026-01-02");
        let nd = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        assert_eq!(pd.value(), nd);
        assert_eq!(PermalinkDate::from(nd), pd);
    }

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

    #[test]
    fn local_datetime_converts_to_utc_by_offset() {
        // 13:30 wall-clock at +05:00 is 08:30 UTC.
        let tz = chrono::FixedOffset::east_opt(5 * 3600).unwrap();
        let got = utc_instant_from_local_in("2026-07-01T13:30", &tz).unwrap();
        assert_eq!(got, "2026-07-01T08:30:00Z".parse().unwrap());
    }

    #[test]
    fn local_datetime_accepts_optional_seconds() {
        let tz = chrono::FixedOffset::east_opt(0).unwrap();
        let got = utc_instant_from_local_in("2026-07-01T13:30:45", &tz).unwrap();
        assert_eq!(got, "2026-07-01T13:30:45Z".parse().unwrap());
    }

    #[test]
    fn local_datetime_west_offset() {
        // 23:00 wall-clock at -05:00 is 04:00 UTC the next day.
        let tz = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        let got = utc_instant_from_local_in("2026-07-01T23:00", &tz).unwrap();
        assert_eq!(got, "2026-07-02T04:00:00Z".parse().unwrap());
    }

    #[test]
    fn local_datetime_rejects_empty_and_unparseable() {
        let tz = chrono::FixedOffset::east_opt(0).unwrap();
        assert_eq!(utc_instant_from_local_in("", &tz), None);
        assert_eq!(utc_instant_from_local_in("   ", &tz), None);
        assert_eq!(utc_instant_from_local_in("not-a-date", &tz), None);
        assert_eq!(utc_instant_from_local_in("2026-13-99T99:99", &tz), None);
    }

    #[test]
    fn public_wrapper_uses_the_ambient_zone() {
        // The exact instant depends on the host timezone, so assert only shape: a
        // well-formed local time (never a DST gap at 13:30) parses; empty/garbage don't.
        assert!(utc_instant_from_local("2026-07-01T13:30").is_some());
        assert_eq!(utc_instant_from_local(""), None);
        assert_eq!(utc_instant_from_local("garbage"), None);
    }
}
