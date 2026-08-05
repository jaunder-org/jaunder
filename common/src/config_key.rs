//! The closed registries of site-wide and per-user configuration keys (#687).
//!
//! A key cannot exist here without a validator: both are columns of one table, so the
//! two lists that used to drift — the `*_KEY` consts and whatever parsed their values —
//! are now one scannable block. See spec D1.

use std::str::FromStr;

use thiserror::Error;

use crate::absolute_url::AbsoluteUrl;
use crate::backup::{BackupMode, BackupSchedule, DestinationPath, RetentionCount};
use crate::feed::{FeedMinDays, FeedMinItems};
use crate::media::{MaxFileSize, UserQuota};
use crate::registration::RegistrationPolicy;
use crate::render::PostFormat;
use crate::site::SiteTitle;
use crate::smtp_host::SmtpHost;
use crate::smtp_password::SmtpPassword;
use crate::smtp_port::SmtpPort;
use crate::smtp_sender::SmtpSender;
use crate::smtp_tls_mode::SmtpTlsMode;
use crate::smtp_username::SmtpUsername;
use crate::visibility::parse_default_audience;

/// Error returned when a stored or offered value does not parse as its key's type.
///
/// Carries the key and the value type's own reason, but **never the value**: one of the
/// keys is `smtp.password`, and an error that echoed its value would put a secret in the
/// CLI's output and in any log that captured it. The caller offering a value already has
/// it.
#[derive(Debug, Error)]
#[error("{key}: {reason}")]
pub struct InvalidSiteConfigValue {
    /// The dotted key whose value failed.
    key: &'static str,
    /// The value type's own rejection message.
    reason: String,
}

/// Runs a key's value type as a validator: parse, then discard the value.
///
/// Rust has no dependent types, so `get(key)` cannot vary its return type by key. The
/// workable substitute is to run the real parser for its `Result` — which is exactly as
/// strict as the type it produces, and cannot drift from it.
fn check<T>(key: &'static str, raw: &str) -> Result<(), InvalidSiteConfigValue>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    T::from_str(raw).map_err(|e| InvalidSiteConfigValue {
        key,
        reason: e.to_string(),
    })?;
    Ok(())
}

/// The validator for `posts.default_audience` — the one key whose value type is not
/// reached through `FromStr`.
///
/// [`crate::visibility::AudienceTarget`] has a `Named(_)` variant that is per-author and
/// has no site-wide form, so the type as a whole is not the site-wide default's grammar;
/// [`parse_default_audience`] is.
fn check_default_audience(key: &'static str, raw: &str) -> Result<(), InvalidSiteConfigValue> {
    if parse_default_audience(raw).is_some() {
        return Ok(());
    }
    Err(InvalidSiteConfigValue {
        key,
        reason: "must be \"public\", \"subscribers\", or \"private\"".to_owned(),
    })
}

/// Emits [`SiteConfigKey`] and its per-key validator from one table.
///
/// Each row is `Variant => "dotted.key" : <value> { optional }?, bad: "<example>";`
/// where `<value>` is either the key's value type (validated through its `FromStr`) or a
/// parenthesised custom validator fn. The `{ optional }` marker drives **both** the
/// empty-accepting validator and `is_optional`, so the two cannot disagree — the
/// empty-means-unset contract (spec D1b) is stated once per key.
///
/// `bad:` is a value that key must reject. There is no universal junk string — four of
/// the value types reject only `""` — so the example is per-row and the test reads it
/// back rather than guessing.
macro_rules! site_config_keys {
    // -- internal: presence of the `{ optional }` marker as a bool --
    (@optional) => { false };
    (@optional { optional }) => { true };

    // -- internal: a row's validator. The parenthesised form comes first so a custom
    //    validator is not mistaken for a type name. --
    (@validate ($custom:path), $key:expr, $raw:expr) => { $custom($key, $raw) };
    (@validate $ty:ident, $key:expr, $raw:expr) => { check::<$ty>($key, $raw) };

    ($(
        $variant:ident => $lit:literal : $value:tt $({ $marker:tt })? , bad: $bad:literal ;
    )+) => {
        /// A site-wide configuration key — the only way to name one.
        ///
        /// Closed by construction: `from_str` rejects anything not in the table, so an
        /// orphaned row in `site_config` can still be *listed* (that is what `list()` is
        /// for) but can never be read or written through the typed seam.
        #[macros::text_enum(
            sqlx,
            error = UnknownSiteConfigKey,
            message = "unknown site-config key"
        )]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
        pub enum SiteConfigKey {
            $(
                #[strum(serialize = $lit)]
                $variant,
            )+
        }

        impl SiteConfigKey {
            /// Checks `raw` against this key's value type, discarding the parsed value.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidSiteConfigValue`] when `raw` does not parse.
            pub fn validate(self, raw: &str) -> Result<(), InvalidSiteConfigValue> {
                // Empty means unset for the keys marked `{ optional }` — a shipped,
                // load-bearing contract (`set_identity`, `set_feeds_config` and
                // `set_backup_config` all store `""` for an absent value). Stated here
                // once rather than in three validators that could drift.
                if raw.is_empty() && self.is_optional() {
                    return Ok(());
                }
                match self {
                    $( Self::$variant => site_config_keys!(@validate $value, $lit, raw), )+
                }
            }

            /// Whether the empty string is a legal value for this key, meaning "unset".
            #[must_use]
            pub fn is_optional(self) -> bool {
                match self {
                    $( Self::$variant => site_config_keys!(@optional $({ $marker })?), )+
                }
            }

            /// A value this key must reject — the table's `bad:` column.
            ///
            /// Test-only: it exists so `every_key_rejects_its_known_bad_value` can prove
            /// each row's validator is wired to a real parser without inventing a junk
            /// string that happens to fail every one of them (there is none).
            #[cfg(test)]
            fn known_bad_example(self) -> &'static str {
                match self {
                    $( Self::$variant => $bad, )+
                }
            }
        }
    };
}

site_config_keys! {
    BackupDestinationPath  => "backup.destination_path"   : DestinationPath { optional }, bad: "   ";
    BackupSchedule         => "backup.schedule"           : BackupSchedule,               bad: "not a cron";
    BackupRetentionCount   => "backup.retention_count"    : RetentionCount,               bad: "0";
    BackupMode             => "backup.mode"               : BackupMode,                   bad: "sideways";
    FeedsMinItems          => "feeds.min_items"           : FeedMinItems,                 bad: "0";
    FeedsMinDays           => "feeds.min_days"            : FeedMinDays,                  bad: "0";
    FeedsWebsubHubUrl      => "feeds.websub_hub_url"      : AbsoluteUrl { optional },     bad: "nonsense://x";
    PostsDefaultAudience   => "posts.default_audience"    : (check_default_audience),     bad: "everyone";
    SiteRegistrationPolicy => "site.registration_policy"  : RegistrationPolicy,           bad: "sideways";
    SiteTitle              => "site.title"                : SiteTitle,                    bad: "";
    SiteBaseUrl            => "site.base_url"             : AbsoluteUrl { optional },     bad: "nonsense://x";
    MediaMaxFileSizeBytes  => "media.max_file_size_bytes" : MaxFileSize,                  bad: "0";
    MediaUserQuotaBytes    => "media.user_quota_bytes"    : UserQuota,                    bad: "0";
    SmtpHost               => "smtp.host"                 : SmtpHost,                     bad: "";
    SmtpPort               => "smtp.port"                 : SmtpPort,                     bad: "not-a-port";
    SmtpTlsMode            => "smtp.tls_mode"             : SmtpTlsMode,                  bad: "ssl";
    SmtpSender             => "smtp.sender"               : SmtpSender,                   bad: "not-a-valid-email";
    SmtpUsername           => "smtp.username"             : SmtpUsername,                 bad: "";
    SmtpPassword           => "smtp.password"             : SmtpPassword,                 bad: "";
}

/// Error returned when a stored or offered per-user value does not parse as its key's
/// type.
///
/// Separate from [`InvalidSiteConfigValue`] because the two registries are separate
/// closed sets: a `user_config` failure can never name a site key, and the type says so.
#[derive(Debug, Error)]
#[error("{key}: {reason}")]
pub struct InvalidUserConfigValue {
    /// The dotted key whose value failed.
    key: &'static str,
    /// The value type's own rejection message.
    reason: String,
}

/// Runs a per-user key's value type as a validator: parse, then discard the value.
fn check_user<T>(key: &'static str, raw: &str) -> Result<(), InvalidUserConfigValue>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    T::from_str(raw).map_err(|e| InvalidUserConfigValue {
        key,
        reason: e.to_string(),
    })?;
    Ok(())
}

/// Emits [`UserConfigKey`] and its per-key validator from one table.
///
/// The same shape as [`site_config_keys!`], minus the `{ optional }` marker: no per-user
/// key uses the empty-means-unset contract, and a marker no row spells is a branch no
/// test could reach. Each row is `Variant => "dotted.key" : <value>, bad: "<example>";`.
macro_rules! user_config_keys {
    ($(
        $variant:ident => $lit:literal : $value:ident , bad: $bad:literal ;
    )+) => {
        /// A per-user configuration key — the only way to name one.
        ///
        /// Closed by construction, exactly as [`SiteConfigKey`] is: `user_config` has no
        /// CLI door, but the typed seam is what keeps a typo from writing a row nothing
        /// will ever read back.
        #[macros::text_enum(
            sqlx,
            error = UnknownUserConfigKey,
            message = "unknown user-config key"
        )]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
        pub enum UserConfigKey {
            $(
                #[strum(serialize = $lit)]
                $variant,
            )+
        }

        impl UserConfigKey {
            /// Checks `raw` against this key's value type, discarding the parsed value.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidUserConfigValue`] when `raw` does not parse.
            pub fn validate(self, raw: &str) -> Result<(), InvalidUserConfigValue> {
                match self {
                    $( Self::$variant => check_user::<$value>($lit, raw), )+
                }
            }

            /// A value this key must reject — the table's `bad:` column.
            #[cfg(test)]
            fn known_bad_example(self) -> &'static str {
                match self {
                    $( Self::$variant => $bad, )+
                }
            }
        }
    };
}

user_config_keys! {
    DefaultPostFormat => "posts.default_format" : PostFormat, bad: "hieroglyphs";
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use strum::VariantArray as _;

    #[test]
    fn every_key_round_trips_its_dotted_form() {
        for key in SiteConfigKey::VARIANTS {
            // Bound rather than inlined into the assert's message: a message argument is
            // only evaluated when the assert fails, which the coverage gate reads as an
            // uncovered line.
            let dotted = key.as_ref();
            assert_eq!(SiteConfigKey::from_str(dotted).ok().as_ref(), Some(key));
            assert!(dotted.contains('.'), "{dotted} must be namespace.name");
        }
        assert_eq!(SiteConfigKey::VARIANTS.len(), 19);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        for bad in ["", "site", "site.nope", "nope.title", " site.title"] {
            assert!(SiteConfigKey::from_str(bad).is_err(), "{bad} must reject");
        }
        let err = SiteConfigKey::from_str("site.nope").unwrap_err();
        assert_eq!(err.to_string(), "unknown site-config key");
    }

    /// The rejection names the key and the value type's own reason — and never the
    /// value, which for `smtp.password` would be a secret.
    #[test]
    fn an_invalid_value_reports_the_key_and_the_reason_but_not_the_value() {
        let err = SiteConfigKey::SmtpPassword
            .validate("")
            .expect_err("empty password must reject");
        assert_eq!(
            err.to_string(),
            "smtp.password: SMTP password must not be empty"
        );

        let err = SiteConfigKey::SiteBaseUrl
            .validate("nonsense://x")
            .expect_err("a non-http scheme must reject");
        assert!(
            err.to_string().starts_with("site.base_url: "),
            "the message names the key: {err}"
        );
        assert!(
            !err.to_string().contains("nonsense://x"),
            "the message must not echo the offered value: {err}"
        );
    }

    /// A4. There is no universal junk string: `SiteTitle`, `SmtpUsername`, `SmtpPassword`
    /// and `DestinationPath` reject only the empty string, so each key carries its own
    /// known-bad example in the table and the test reads it back.
    #[test]
    fn every_key_rejects_its_known_bad_value() {
        for key in SiteConfigKey::VARIANTS {
            let dotted = key.as_ref();
            let bad = key.known_bad_example();
            assert!(key.validate(bad).is_err(), "{dotted} must reject {bad:?}");
        }
    }

    /// A5: the empty-means-unset contract, pinned per key.
    #[test]
    fn optional_keys_accept_empty_and_others_reject_it() {
        for key in SiteConfigKey::VARIANTS {
            let dotted = key.as_ref();
            let optional = key.is_optional();
            let got = key.validate("");
            assert_eq!(
                got.is_ok(),
                optional,
                "{dotted} optional={optional} but validate(\"\")={got:?}"
            );
        }
    }

    /// The accepting half of the validator: the table's rows are wired to parsers that
    /// say yes as well as no, including the one custom validator.
    #[test]
    fn valid_values_are_accepted() {
        for (key, good) in [
            (SiteConfigKey::SiteTitle, "My Site"),
            (SiteConfigKey::SiteBaseUrl, "https://example.com/"),
            (SiteConfigKey::SmtpPort, "587"),
            (SiteConfigKey::SmtpTlsMode, "starttls"),
            (SiteConfigKey::BackupRetentionCount, "7"),
            (SiteConfigKey::PostsDefaultAudience, "subscribers"),
        ] {
            let dotted = key.as_ref();
            let got = key.validate(good);
            assert!(got.is_ok(), "{dotted} must accept {good:?}: {got:?}");
        }
    }

    /// D8: the per-user registry is closed and validating in the same two ways.
    #[test]
    fn user_config_key_validates_its_value() {
        assert!(
            UserConfigKey::DefaultPostFormat
                .validate("markdown")
                .is_ok()
        );
        assert!(
            UserConfigKey::DefaultPostFormat
                .validate("hieroglyphs")
                .is_err()
        );
        for key in UserConfigKey::VARIANTS {
            let dotted = key.as_ref();
            let bad = key.known_bad_example();
            assert!(key.validate(bad).is_err(), "{dotted} must reject {bad:?}");
            assert_eq!(UserConfigKey::from_str(dotted).ok().as_ref(), Some(key));
            assert!(dotted.contains('.'), "{dotted} must be namespace.name");
        }
        let err = UserConfigKey::from_str("posts.nope").unwrap_err();
        assert_eq!(err.to_string(), "unknown user-config key");
    }
}
