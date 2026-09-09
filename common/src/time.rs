use std::fmt;
#[cfg(feature = "sqlx")]
use std::fmt::Write;
use std::str::FromStr;

use jiff::{Timestamp, civil, tz::TimeZone};
use serde::{Deserialize, Serialize, de};
use thiserror::Error;

/// A UTC instant that crosses the web `#[server]` boundary as a domain value
/// instead of a bare RFC 3339 `String` (ADR-0063; see
/// `docs/adr/0072-timestamps-cross-boundary-as-utcinstant.md`).
///
/// It wraps [`jiff::Timestamp`] and is serde-transparent, so it serializes as
/// Jiff's canonical RFC 3339 UTC string. Parsing delegates directly to Jiff's
/// Temporal timestamp parser; offset-bearing input therefore represents the
/// same instant as its canonical `Z` form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UtcInstant(Timestamp);

/// Error returned when a string cannot be parsed as a [`UtcInstant`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid RFC 3339 timestamp")]
pub struct InvalidInstant;

impl UtcInstant {
    /// The current wall-clock instant in UTC.
    #[must_use]
    pub fn now() -> Self {
        Self(Timestamp::now())
    }

    /// The inner [`Timestamp`] by value. Use `Timestamp::from(x)` / `x.into()`
    /// where that reads better than `x.value()`.
    #[must_use]
    pub fn value(self) -> Timestamp {
        self.0
    }
}

impl From<Timestamp> for UtcInstant {
    fn from(timestamp: Timestamp) -> Self {
        Self(timestamp)
    }
}

impl From<UtcInstant> for Timestamp {
    fn from(instant: UtcInstant) -> Self {
        instant.0
    }
}

impl FromStr for UtcInstant {
    type Err = InvalidInstant;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self).map_err(|_| InvalidInstant)
    }
}

impl fmt::Display for UtcInstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A validated permalink calendar date (`YYYY-MM-DD`), wrapping a Jiff civil
/// [`Date`](civil::Date) so an impossible date is unrepresentable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PermalinkDate(civil::Date);

impl<'de> Deserialize<'de> for PermalinkDate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PermalinkDateVisitor;

        impl de::Visitor<'_> for PermalinkDateVisitor {
            type Value = PermalinkDate;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a canonical ISO 8601 calendar date")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                parse_permalink_date(value)
                    .ok_or_else(|| E::custom("invalid canonical ISO 8601 calendar date"))
            }
        }

        deserializer.deserialize_str(PermalinkDateVisitor)
    }
}

fn parse_permalink_date(value: &str) -> Option<PermalinkDate> {
    let bytes = value.as_bytes();
    let (year, month, day) = match bytes {
        _ if bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' => (
            parse_decimal(&bytes[..4])?,
            parse_decimal(&bytes[5..7])?,
            parse_decimal(&bytes[8..10])?,
        ),
        _ if bytes.len() == 13 && bytes[0] == b'-' && bytes[7] == b'-' && bytes[10] == b'-' => {
            let year = parse_decimal(&bytes[1..7])?;
            (
                year.checked_neg().filter(|year| *year != 0)?,
                parse_decimal(&bytes[8..10])?,
                parse_decimal(&bytes[11..13])?,
            )
        }
        _ => return None,
    };
    PermalinkDate::from_ymd(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)
}

fn parse_decimal(bytes: &[u8]) -> Option<i32> {
    let mut number = 0;
    for &byte in bytes {
        let digit = byte.checked_sub(b'0')?;
        if digit > 9 {
            return None;
        }
        number = number * 10 + i32::from(digit);
    }
    Some(number)
}

impl PermalinkDate {
    /// The one fallible construction door: an impossible date (bad month/day,
    /// non-existent day-of-month, or a year outside Jiff's supported range)
    /// yields `None`.
    #[must_use]
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        civil::Date::new(
            year.try_into().ok()?,
            month.try_into().ok()?,
            day.try_into().ok()?,
        )
        .ok()
        .map(Self)
    }

    /// The inner civil date by value, for in-Rust date comparison.
    #[must_use]
    pub fn value(self) -> civil::Date {
        self.0
    }
}

impl From<civil::Date> for PermalinkDate {
    fn from(date: civil::Date) -> Self {
        Self(date)
    }
}

impl From<PermalinkDate> for civil::Date {
    fn from(date: PermalinkDate) -> Self {
        date.0
    }
}

impl fmt::Display for PermalinkDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Formats a UTC instant for a browser `<input type="datetime-local">` in the
/// ambient local timezone, at the control's minute precision.
#[must_use]
pub fn local_datetime_from_utc(instant: UtcInstant) -> String {
    local_datetime_from_utc_in(instant, &TimeZone::system())
}

fn local_datetime_from_utc_in(instant: UtcInstant, timezone: &TimeZone) -> String {
    timezone
        .to_datetime(instant.value())
        .strftime("%Y-%m-%dT%H:%M")
        .to_string()
}

/// Converts a browser `<input type="datetime-local">` value — a local
/// wall-clock `YYYY-MM-DDTHH:MM` with optional seconds — into a [`UtcInstant`]
/// in the ambient local timezone. Empty, malformed, and impossible values yield
/// `None`. Compatible disambiguation retains browser-normalized gap handling
/// and selects the earlier instant in a fold.
#[must_use]
pub fn utc_instant_from_local(local: &str) -> Option<UtcInstant> {
    utc_instant_from_local_in(local, &TimeZone::system())
}

/// Converts a local wall-clock value only when compatible resolution projects
/// back to the same local value. Thus gaps are rejected while real fall-back
/// ambiguities retain Jiff's earlier-fold selection.
#[must_use]
pub fn strict_utc_instant_from_local(local: &str) -> Option<UtcInstant> {
    strict_utc_instant_from_local_in(local, &TimeZone::system())
}

fn parse_html_local(local: &str) -> Option<civil::DateTime> {
    let local = local.trim();
    let bytes = local.as_bytes();
    let valid_shape = matches!(bytes.len(), 16 | 19)
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && (bytes.len() == 16 || bytes.get(16) == Some(&b':'))
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit());
    valid_shape.then(|| local.parse().ok()).flatten()
}

fn utc_instant_from_local_in(local: &str, timezone: &TimeZone) -> Option<UtcInstant> {
    let datetime = parse_html_local(local)?;
    timezone
        .to_ambiguous_timestamp(datetime)
        .compatible()
        .ok()
        .map(UtcInstant::from)
}

fn strict_utc_instant_from_local_in(local: &str, timezone: &TimeZone) -> Option<UtcInstant> {
    let datetime = parse_html_local(local)?;
    let timestamp = timezone
        .to_ambiguous_timestamp(datetime)
        .compatible()
        .ok()?;
    (timezone.to_datetime(timestamp) == datetime).then_some(UtcInstant::from(timestamp))
}

#[cfg(feature = "sqlx")]
impl sqlx::Type<sqlx::Postgres> for UtcInstant {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <jiff_sqlx::Timestamp as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <jiff_sqlx::Timestamp as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

#[cfg(feature = "sqlx")]
impl<'q> sqlx::Encode<'q, sqlx::Postgres> for UtcInstant {
    fn encode_by_ref(
        &self,
        buffer: &mut <sqlx::Postgres as sqlx::Database>::ArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let timestamp = jiff_sqlx::Timestamp::from(self.0);
        <jiff_sqlx::Timestamp as sqlx::Encode<'q, sqlx::Postgres>>::encode_by_ref(
            &timestamp, buffer,
        )
    }
}

#[cfg(feature = "sqlx")]
impl<'r> sqlx::Decode<'r, sqlx::Postgres> for UtcInstant {
    fn decode(
        value: <sqlx::Postgres as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        <jiff_sqlx::Timestamp as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)
            .map(jiff_sqlx::Timestamp::to_jiff)
            .map(Self)
    }
}

#[cfg(feature = "sqlx")]
impl sqlx::Type<sqlx::Sqlite> for UtcInstant {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <jiff_sqlx::Timestamp as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        matches!(
            sqlx::TypeInfo::name(ty),
            "DATETIME" | "TEXT" | "INTEGER" | "REAL"
        )
    }
}

#[cfg(feature = "sqlx")]
impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for UtcInstant {
    fn encode_by_ref(
        &self,
        buffer: &mut <sqlx::Sqlite as sqlx::Database>::ArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        let nanos = self.0.subsec_nanosecond().unsigned_abs();
        let fractional_width = if nanos == 0 {
            0
        } else if nanos.is_multiple_of(1_000_000) {
            3
        } else if nanos.is_multiple_of(1_000) {
            6
        } else {
            9
        };
        let mut timestamp = String::with_capacity(38);
        write!(&mut timestamp, "{:.*}", fractional_width, self.0)?;
        let _ = timestamp.pop();
        timestamp.push_str("+00:00");
        <String as sqlx::Encode<'q, sqlx::Sqlite>>::encode(timestamp, buffer)
    }
}

#[cfg(feature = "sqlx")]
impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for UtcInstant {
    fn decode(
        value: <sqlx::Sqlite as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        if sqlx::TypeInfo::name(sqlx::ValueRef::type_info(&value).as_ref()) != "TEXT" {
            return <jiff_sqlx::Timestamp as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)
                .map(jiff_sqlx::Timestamp::to_jiff)
                .map(Self);
        }

        let timestamp = <&str as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)?;
        if let Ok(timestamp) = timestamp.parse() {
            return Ok(Self(timestamp));
        }

        timestamp
            .parse::<civil::DateTime>()
            .and_then(|datetime| datetime.to_zoned(TimeZone::UTC))
            .map(|datetime| Self(datetime.timestamp()))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permalink_date_rejects_impossible_dates_and_preserves_iso_form() {
        assert_eq!(PermalinkDate::from_ymd(2026, 2, 30), None);
        let date = PermalinkDate::from_ymd(2026, 1, 2).unwrap();
        assert_eq!(date.to_string(), "2026-01-02");
        assert_eq!(serde_json::to_string(&date).unwrap(), "\"2026-01-02\"");
        assert_eq!(
            serde_json::from_str::<PermalinkDate>("\"2026-01-02\"").unwrap(),
            date
        );
    }

    #[test]
    fn permalink_date_accepts_jiff_range_endpoints() {
        assert_eq!(
            PermalinkDate::from_ymd(-9999, 1, 1).unwrap().to_string(),
            "-009999-01-01"
        );
        assert_eq!(
            PermalinkDate::from_ymd(9999, 12, 31).unwrap().to_string(),
            "9999-12-31"
        );
        assert_eq!(PermalinkDate::from_ymd(-10_000, 1, 1), None);
        assert_eq!(PermalinkDate::from_ymd(10_000, 1, 1), None);
    }

    #[test]
    fn permalink_date_serde_only_accepts_canonical_civil_dates() {
        for canonical in ["-009999-01-01", "9999-12-31"] {
            let encoded = format!("\"{canonical}\"");
            let decoded = serde_json::from_str::<PermalinkDate>(&encoded).unwrap();
            assert_eq!(decoded.to_string(), canonical);
            assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
        }

        for noncanonical in [
            "2026-01-02T03:04",
            "2026-01-02+00:00",
            "2026-01-02T03:04Z[UTC]",
            "2026-1-02",
            "-9999-01-01",
            "202a-01-02",
        ] {
            let encoded = format!("\"{noncanonical}\"");
            assert!(
                serde_json::from_str::<PermalinkDate>(&encoded).is_err(),
                "{noncanonical}"
            );
        }

        let err = serde_json::from_str::<PermalinkDate>("1").unwrap_err();
        assert!(
            err.to_string()
                .contains("a canonical ISO 8601 calendar date"),
            "{err}"
        );
    }

    #[test]
    fn utc_instant_parses_jiff_temporal_input_and_canonicalizes_offsets() {
        let jiff_only = "2024-07-01T16:24Z".parse::<UtcInstant>().unwrap();
        let offset = "2024-07-01T18:24+02:00".parse::<UtcInstant>().unwrap();
        assert_eq!(jiff_only, offset);
        assert_eq!(jiff_only.to_string(), "2024-07-01T16:24:00Z");
    }

    #[test]
    fn utc_instant_accepts_jiff_timestamp_endpoints() {
        for (text, endpoint) in [
            ("-009999-01-02T01:59:59Z", Timestamp::MIN),
            ("9999-12-30T22:00:00.999999999Z", Timestamp::MAX),
        ] {
            let instant = text.parse::<UtcInstant>().unwrap();
            assert_eq!(instant.value(), endpoint);
            assert_eq!(instant.to_string(), text);
        }
    }

    #[test]
    fn utc_instant_normalizes_a_leap_second() {
        let instant = "2024-07-01T16:24:60Z".parse::<UtcInstant>().unwrap();
        assert_eq!(instant.to_string(), "2024-07-01T16:24:59Z");
    }

    #[test]
    fn utc_instant_serde_display_order_and_jiff_conversions_are_stable() {
        let earlier = "2024-07-01T16:24Z".parse::<UtcInstant>().unwrap();
        let later = "2024-07-01T16:24:01Z".parse::<UtcInstant>().unwrap();
        let encoded = serde_json::to_string(&earlier).unwrap();
        assert_eq!(encoded, "\"2024-07-01T16:24:00Z\"");
        assert_eq!(
            serde_json::from_str::<UtcInstant>(&encoded).unwrap(),
            earlier
        );
        assert!(earlier < later);
        assert_eq!(UtcInstant::from(Timestamp::from(earlier)), earlier);
        assert_eq!(Timestamp::from(earlier), earlier.value());

        let date = PermalinkDate::from_ymd(2024, 7, 1).unwrap();
        assert_eq!(civil::Date::from(date), date.value());
    }

    #[test]
    fn now_is_an_instant_between_adjacent_clock_reads() {
        let before = Timestamp::now();
        let instant = UtcInstant::now().value();
        let after = Timestamp::now();
        assert!(before <= instant);
        assert!(instant <= after);
    }

    #[test]
    fn html_local_grammar_and_non_strict_gap_normalization_are_retained() {
        let timezone = TimeZone::get("America/New_York").unwrap();
        assert_eq!(
            utc_instant_from_local_in("2024-03-10T02:30", &timezone)
                .unwrap()
                .to_string(),
            "2024-03-10T07:30:00Z"
        );
        assert_eq!(
            utc_instant_from_local_in("2024-07-01T16:24:45", &timezone)
                .unwrap()
                .to_string(),
            "2024-07-01T20:24:45Z"
        );
        for invalid in [
            "",
            "2024-07-01 16:24",
            "2024-07-01T16:24:45.1",
            "2024-7-1T16:24",
        ] {
            assert_eq!(
                utc_instant_from_local_in(invalid, &timezone),
                None,
                "{invalid}"
            );
        }
    }

    #[test]
    fn strict_local_resolution_selects_earlier_fold_and_rejects_gap() {
        let timezone = TimeZone::get("America/New_York").unwrap();
        assert_eq!(
            strict_utc_instant_from_local_in("2024-11-03T01:30", &timezone)
                .unwrap()
                .to_string(),
            "2024-11-03T05:30:00Z"
        );
        assert_eq!(
            strict_utc_instant_from_local_in("2024-03-10T02:30", &timezone),
            None
        );
    }

    #[test]
    fn local_datetime_format_is_minute_precision_in_the_requested_zone() {
        let timezone = TimeZone::get("America/New_York").unwrap();
        let instant = "2024-07-01T20:24:45Z".parse().unwrap();
        assert_eq!(
            local_datetime_from_utc_in(instant, &timezone),
            "2024-07-01T16:24"
        );
    }

    #[cfg(feature = "sqlx")]
    #[tokio::test]
    async fn sqlite_bridge_preserves_timestamp_text_and_accepts_numeric_storage() {
        use sqlx::Connection;

        let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        let instant = "2024-07-01T16:24:45.123456Z".parse::<UtcInstant>().unwrap();

        let encoded: String = sqlx::query_scalar("SELECT ?")
            .bind(instant)
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(encoded, "2024-07-01T16:24:45.123456+00:00");

        let decoded: UtcInstant = sqlx::query_scalar("SELECT ?")
            .bind(&encoded)
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(decoded, instant);

        let julian_day_zero: UtcInstant = sqlx::query_scalar("SELECT 0")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(julian_day_zero.to_string(), "-004713-11-24T12:00:00Z");

        let _ = <UtcInstant as sqlx::Type<sqlx::Sqlite>>::type_info();
    }
}
