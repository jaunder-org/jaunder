use std::fmt;
use std::str::FromStr;

use jiff::{Timestamp, civil, tz::TimeZone};
use serde::{Deserialize, Serialize, de};
use thiserror::Error;

/// Supported named IANA publication zone, retaining the author's exact spelling.
#[derive(Clone, Debug, PartialEq, Eq, macros::StrNewtype)]
pub struct PublicationTimezone(String);

/// A publication zone must resolve through the bundled named IANA database.
#[derive(Debug, Error)]
#[error("publication timezone must be a supported named IANA timezone")]
pub struct InvalidPublicationTimezone;

impl FromStr for PublicationTimezone {
    type Err = InvalidPublicationTimezone;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let zone = TimeZone::get(value).map_err(|_| InvalidPublicationTimezone)?;
        if value.trim() != value
            || zone
                .iana_name()
                .is_none_or(|name| name.eq_ignore_ascii_case("Etc/Unknown"))
        {
            return Err(InvalidPublicationTimezone);
        }
        Ok(Self(value.to_owned()))
    }
}

impl PublicationTimezone {
    /// Capture-only fallback from the device's current UTC offset, in seconds east.
    /// Whole hours within ±12 map to POSIX-sign-reversed named Etc/GMT zones.
    /// This records a fixed offset, not a regional timezone or its DST history.
    #[must_use]
    pub fn from_device_offset_seconds(seconds_east: i32) -> Option<Self> {
        if !(-43_200..=43_200).contains(&seconds_east) || seconds_east % 3600 != 0 {
            return None;
        }
        let hours_west = -seconds_east / 3600;
        let name = if hours_west == 0 {
            "Etc/GMT".to_owned()
        } else {
            format!("Etc/GMT{hours_west:+}")
        };
        name.parse().ok()
    }

    /// Explicit compatibility fallback for legacy callers and migrated rows.
    #[must_use]
    pub fn utc() -> Self {
        Self("UTC".to_owned())
    }
}

/// Formats a publication instant in its saved editing zone, including the
/// date-specific IANA abbreviation. Audit instants and input conversion are separate.
#[must_use]
pub fn format_publication_time(instant: UtcInstant, timezone: &PublicationTimezone) -> String {
    let zone = TimeZone::get(timezone.as_ref()).unwrap_or_else(|_| {
        unreachable!("PublicationTimezone already validated against the bundled named database")
    });
    instant
        .value()
        .to_zoned(zone)
        .strftime("%Y-%m-%d %H:%M %Z")
        .to_string()
}

#[cfg(test)]
mod publication_timezone_tests {
    #[test]
    fn device_offset_capture_is_exact_bounded_and_posix_reversed() {
        use super::PublicationTimezone;
        for (seconds_east, expected) in [
            (-43200, "Etc/GMT+12"),
            (-18000, "Etc/GMT+5"),
            (0, "Etc/GMT"),
            (3600, "Etc/GMT-1"),
            (43200, "Etc/GMT-12"),
        ] {
            assert_eq!(
                PublicationTimezone::from_device_offset_seconds(seconds_east)
                    .unwrap()
                    .as_ref(),
                expected
            );
        }
        for seconds_east in [-46800, -19800, -1, 1, 20700, 46800, i32::MIN, i32::MAX] {
            assert!(PublicationTimezone::from_device_offset_seconds(seconds_east).is_none());
        }
    }

    #[test]
    fn publication_timezone_accepts_named_aliases_without_normalizing() {
        for name in ["UTC", "America/New_York", "US/Eastern", "Europe/London"] {
            let zone: super::PublicationTimezone = name.parse().unwrap();
            assert_eq!(zone.as_ref(), name);
            let encoded = serde_json::to_string(&zone).unwrap();
            assert_eq!(
                serde_json::from_str::<super::PublicationTimezone>(&encoded).unwrap(),
                zone
            );
        }
        for name in [
            "",
            " ",
            " UTC",
            "UTC ",
            "Etc/Unknown",
            "+05:00",
            "-0400",
            "No/Such_Zone",
        ] {
            assert!(
                name.parse::<super::PublicationTimezone>().is_err(),
                "{name:?}"
            );
        }
    }
}

/// A UTC instant that crosses the web `#[server]` boundary as a domain value
/// instead of a bare RFC 3339 `String` (ADR-0063; see
/// `docs/adr/0072-timestamps-cross-boundary-as-utcinstant.md`).
///
/// It wraps [`jiff::Timestamp`] within the application's positive-year range.
/// Parsing retains Jiff's Temporal input compatibility, while Display, serde
/// and `SQLite` all emit the same lossless, fixed-nine UTC representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcInstant(Timestamp);

/// Error returned when a string cannot be parsed as a [`UtcInstant`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid timestamp or instant outside application UTC bounds")]
pub struct InvalidInstant;

impl UtcInstant {
    /// The earliest sortable application instant, not Jiff's BCE minimum.
    pub const MIN: Self = Self(Timestamp::constant(-62_135_596_800, 0));
    /// Jiff's real maximum; the rest of calendar year 9999 is not representable.
    pub const MAX: Self = Self(Timestamp::MAX);

    /// Check after UTC normalization, so offset crossings cannot evade bounds.
    ///
    /// # Errors
    /// Returns [`InvalidInstant`] when `timestamp` is outside [`Self::MIN`] through
    /// [`Self::MAX`], inclusive.
    pub fn from_timestamp(timestamp: Timestamp) -> Result<Self, InvalidInstant> {
        if timestamp < Self::MIN.0 || timestamp > Self::MAX.0 {
            return Err(InvalidInstant);
        }
        Ok(Self(timestamp))
    }

    /// Checked timestamp arithmetic never silently clamps a result.
    ///
    /// # Errors
    /// Returns [`InvalidInstant`] if Jiff arithmetic fails or the result is
    /// outside the application bounds.
    pub fn checked_add<A: Into<jiff::TimestampArithmetic>>(
        self,
        amount: A,
    ) -> Result<Self, InvalidInstant> {
        Self::from_timestamp(self.0.checked_add(amount).map_err(|_| InvalidInstant)?)
    }

    /// Checks both Jiff overflow and the application's narrower lower bound.
    ///
    /// # Errors
    /// Returns [`InvalidInstant`] if Jiff arithmetic fails or the result is
    /// outside the application bounds.
    pub fn checked_sub<A: Into<jiff::TimestampArithmetic>>(
        self,
        amount: A,
    ) -> Result<Self, InvalidInstant> {
        Self::from_timestamp(self.0.checked_sub(amount).map_err(|_| InvalidInstant)?)
    }

    /// The current wall-clock instant in UTC.
    ///
    /// # Panics
    /// Panics if the system clock is outside the application bounds.
    #[must_use]
    pub fn now() -> Self {
        Self::from_clock(Timestamp::now())
    }

    fn from_clock(timestamp: Timestamp) -> Self {
        match Self::from_timestamp(timestamp) {
            Ok(instant) => instant,
            Err(error) => panic!(
                "system clock invariant: current UTC instant is outside application bounds: {error:?}"
            ),
        }
    }

    /// The inner [`Timestamp`] by value. Use `Timestamp::from(x)` / `x.into()`
    /// where that reads better than `x.value()`.
    #[must_use]
    pub fn value(self) -> Timestamp {
        self.0
    }
}

impl TryFrom<Timestamp> for UtcInstant {
    type Error = InvalidInstant;

    fn try_from(timestamp: Timestamp) -> Result<Self, Self::Error> {
        Self::from_timestamp(timestamp)
    }
}

impl Serialize for UtcInstant {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UtcInstant {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_timestamp(Timestamp::deserialize(deserializer)?).map_err(de::Error::custom)
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
        Self::from_timestamp(s.parse().map_err(|_| InvalidInstant)?)
    }
}

impl fmt::Display for UtcInstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            &jiff::fmt::temporal::DateTimePrinter::new()
                .precision(Some(9))
                .timestamp_to_string(&self.0),
        )
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
        .and_then(|timestamp| UtcInstant::from_timestamp(timestamp).ok())
}

fn strict_utc_instant_from_local_in(local: &str, timezone: &TimeZone) -> Option<UtcInstant> {
    let datetime = parse_html_local(local)?;
    let timestamp = timezone
        .to_ambiguous_timestamp(datetime)
        .compatible()
        .ok()?;
    if timezone.to_datetime(timestamp) != datetime {
        return None;
    }
    UtcInstant::from_timestamp(timestamp).ok()
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
        let timestamp = <jiff_sqlx::Timestamp as sqlx::Decode<'r, sqlx::Postgres>>::decode(value)?;
        Self::from_timestamp(timestamp.to_jiff()).map_err(Into::into)
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
        let timestamp = self.to_string();
        <String as sqlx::Encode<'q, sqlx::Sqlite>>::encode(timestamp, buffer)
    }
}

#[cfg(feature = "sqlx")]
impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for UtcInstant {
    fn decode(
        value: <sqlx::Sqlite as sqlx::Database>::ValueRef<'r>,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        if sqlx::TypeInfo::name(sqlx::ValueRef::type_info(&value).as_ref()) != "TEXT" {
            let timestamp =
                <jiff_sqlx::Timestamp as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)?;
            return Self::from_timestamp(timestamp.to_jiff()).map_err(Into::into);
        }

        let timestamp = <&str as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)?;
        if let Ok(timestamp) = timestamp.parse::<Timestamp>() {
            return Self::from_timestamp(timestamp).map_err(Into::into);
        }

        let timestamp = timestamp
            .parse::<civil::DateTime>()?
            .to_zoned(TimeZone::UTC)?
            .timestamp();
        Self::from_timestamp(timestamp).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_instant_closes_construction_and_serde_over_normalized_application_bounds() {
        for text in [
            "0000-12-31T23:59:59.999999999Z",
            "0001-01-01T00:00:00+00:01",
            "-000001-12-31T23:59:59Z",
            "2025-02-29T00:00:00Z",
        ] {
            assert!(text.parse::<UtcInstant>().is_err(), "parse admitted {text}");
            assert!(
                serde_json::from_str::<UtcInstant>(&format!("\"{text}\"")).is_err(),
                "serde admitted {text}"
            );
        }
        assert!(UtcInstant::try_from(Timestamp::MIN).is_err());
        for text in [
            "0001-01-01T00:00:00.000000000Z",
            "1969-12-31T23:59:59.000000001Z",
            "9999-12-30T22:00:00.999999999Z",
        ] {
            let instant: UtcInstant = text.parse().unwrap();
            assert_eq!(instant.to_string(), text);
            assert_eq!(
                serde_json::to_string(&instant).unwrap(),
                format!("\"{text}\"")
            );
            assert_eq!(UtcInstant::try_from(instant.value()).unwrap(), instant);
        }
    }

    #[test]
    fn utc_instant_checked_arithmetic_rejects_library_and_application_overflow() {
        use std::time::Duration;
        assert!(
            UtcInstant::MIN
                .checked_sub(Duration::from_nanos(1))
                .is_err()
        );
        assert!(
            UtcInstant::MAX
                .checked_add(Duration::from_nanos(1))
                .is_err()
        );
        assert!(UtcInstant::MIN.checked_add(Duration::MAX).is_err());
        assert!(UtcInstant::MAX.checked_sub(Duration::MAX).is_err());
        assert_eq!(
            UtcInstant::MIN.checked_add(Duration::ZERO).unwrap(),
            UtcInstant::MIN
        );
        assert_eq!(
            UtcInstant::MAX.checked_sub(Duration::ZERO).unwrap(),
            UtcInstant::MAX
        );
        assert_eq!(
            UtcInstant::MIN
                .checked_add(Duration::from_nanos(999))
                .unwrap()
                .to_string(),
            "0001-01-01T00:00:00.000000999Z"
        );
        assert!(
            "9999-12-30T22:00:00.999999999-00:01"
                .parse::<UtcInstant>()
                .is_err()
        );
    }

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
        assert_eq!(jiff_only.to_string(), "2024-07-01T16:24:00.000000000Z");
    }

    #[test]
    fn utc_instant_accepts_application_timestamp_endpoints() {
        for (text, endpoint) in [
            ("0001-01-01T00:00:00.000000000Z", UtcInstant::MIN.value()),
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
        assert_eq!(instant.to_string(), "2024-07-01T16:24:59.000000000Z");
    }

    #[test]
    fn utc_instant_serde_display_order_and_jiff_conversions_are_stable() {
        let earlier = "2024-07-01T16:24Z".parse::<UtcInstant>().unwrap();
        let later = "2024-07-01T16:24:01Z".parse::<UtcInstant>().unwrap();
        let encoded = serde_json::to_string(&earlier).unwrap();
        assert_eq!(encoded, "\"2024-07-01T16:24:00.000000000Z\"");
        assert_eq!(
            serde_json::from_str::<UtcInstant>(&encoded).unwrap(),
            earlier
        );
        assert!(earlier < later);
        assert_eq!(
            UtcInstant::try_from(Timestamp::from(earlier)).unwrap(),
            earlier
        );
        assert_eq!(Timestamp::from(earlier), earlier.value());

        let date = PermalinkDate::from_ymd(2024, 7, 1).unwrap();
        assert_eq!(civil::Date::from(date), date.value());
    }

    #[test]
    #[should_panic(
        expected = "system clock invariant: current UTC instant is outside application bounds"
    )]
    fn out_of_domain_clock_panics_instead_of_clamping() {
        UtcInstant::from_clock(Timestamp::MIN);
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
            "2024-03-10T07:30:00.000000000Z"
        );
        assert_eq!(
            utc_instant_from_local_in("2024-07-01T16:24:45", &timezone)
                .unwrap()
                .to_string(),
            "2024-07-01T20:24:45.000000000Z"
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
            "2024-11-03T05:30:00.000000000Z"
        );
        assert_eq!(
            strict_utc_instant_from_local_in("2024-03-10T02:30", &timezone),
            None
        );
    }

    #[test]
    fn civil_conversion_public_seams_enforce_normalized_application_bounds() {
        const CHILD: &str = "JAUNDER_TEST_CIVIL_BOUNDS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Send each zone together with its declared expectations. The child
            // exercises system-zone entrypoints without a second fixture selector.
            for (zone, cases) in [
                (
                    "UTC",
                    vec![
                        ("0000-12-31T23:59:59", None),
                        (
                            "0001-01-01T00:00:00",
                            Some("0001-01-01T00:00:00.000000000Z"),
                        ),
                        (
                            "9999-12-30T22:00:00",
                            Some("9999-12-30T22:00:00.000000000Z"),
                        ),
                        ("9999-12-30T22:00:01", None),
                    ],
                ),
                (
                    "Etc/GMT-1",
                    vec![
                        ("0000-12-31T23:59:59", None),
                        ("0001-01-01T00:59:59", None),
                        (
                            "0001-01-01T01:00:00",
                            Some("0001-01-01T00:00:00.000000000Z"),
                        ),
                        (
                            "0001-01-01T01:00:01",
                            Some("0001-01-01T00:00:01.000000000Z"),
                        ),
                        (
                            "9999-12-30T23:00:00",
                            Some("9999-12-30T22:00:00.000000000Z"),
                        ),
                        ("9999-12-30T23:00:01", None),
                    ],
                ),
                (
                    "Etc/GMT+1",
                    vec![
                        ("0000-12-31T22:59:59", None),
                        (
                            "0000-12-31T23:00:00",
                            Some("0001-01-01T00:00:00.000000000Z"),
                        ),
                        (
                            "0001-01-01T00:00:00",
                            Some("0001-01-01T01:00:00.000000000Z"),
                        ),
                        (
                            "9999-12-30T21:00:00",
                            Some("9999-12-30T22:00:00.000000000Z"),
                        ),
                        ("9999-12-30T21:00:01", None),
                    ],
                ),
            ] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "time::tests::civil_conversion_public_seams_enforce_normalized_application_bounds", "--nocapture"])
                    .env(CHILD, serde_json::to_string(&cases).unwrap())
                    .env("TZ", zone).output().unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    output.status.success()
                        && stdout.contains("civil-boundary public assertions complete"),
                    "{zone}: {stdout}{stderr}"
                );
            }
            return;
        }
        let cases: Vec<(String, Option<String>)> =
            serde_json::from_str(&std::env::var(CHILD).unwrap()).unwrap();
        for (local, expected) in cases {
            let expected = expected.map(|text| text.parse::<UtcInstant>().unwrap());
            assert_eq!(
                utc_instant_from_local(&local),
                expected,
                "compatible {local}"
            );
            assert_eq!(
                strict_utc_instant_from_local(&local),
                expected,
                "strict {local}"
            );
        }
        println!("civil-boundary public assertions complete");
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
}
