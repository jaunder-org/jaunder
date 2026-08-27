//! Closed site-wide configuration-key registry (#687).
//!
//! A key cannot exist without a validator: both are columns of one scannable
//! table, so the key list and its value contracts cannot drift apart.

use std::str::FromStr;

use thiserror::Error;

use crate::feed::{FeedMinDays, FeedMinItems};
use common::{
    backup::{BackupMode, BackupSchedule, DestinationPath, RetentionCount},
    media::{MaxFileSize, UserQuota},
    registration::RegistrationPolicy,
    site::SiteTitle,
    smtp_host::SmtpHost,
    smtp_password::SmtpPassword,
    smtp_port::SmtpPort,
    smtp_sender::SmtpSender,
    smtp_tls_mode::SmtpTlsMode,
    smtp_username::SmtpUsername,
    tagged_url::{BaseUrl, HubUrl},
    visibility::DefaultAudience,
};

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

/// Emits [`SiteConfigKey`] and its per-key validator from one table.
///
/// Each row is `Variant => "dotted.key" : ValueType { optional }?, bad: "<example>";`.
/// Every `ValueType` is validated through its `FromStr` implementation. The `{ optional }`
/// marker drives **both** the empty-accepting validator and `is_optional`, so the two
/// cannot disagree — the empty-means-unset contract (spec D1b) is stated once per key.
///
/// `bad:` is a value that key must reject. There is no universal junk string — four of
/// the value types reject only `""` — so the example is per-row and the test reads it
/// back rather than guessing.
macro_rules! site_config_keys {
    // -- internal: presence of the `{ optional }` marker as a bool --
    (@optional) => { false };
    (@optional { optional }) => { true };

    // -- internal: a row's validator --
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
    FeedsWebsubHubUrl      => "feeds.websub_hub_url"      : HubUrl { optional },          bad: "nonsense://x";
    PostsDefaultAudience   => "posts.default_audience"    : DefaultAudience,              bad: "everyone";
    SiteRegistrationPolicy => "site.registration_policy"  : RegistrationPolicy,           bad: "sideways";
    SiteTitle              => "site.title"                : SiteTitle,                    bad: "";
    SiteBaseUrl            => "site.base_url"             : BaseUrl { optional },         bad: "nonsense://x";
    MediaMaxFileSizeBytes  => "media.max_file_size_bytes" : MaxFileSize,                  bad: "0";
    MediaUserQuotaBytes    => "media.user_quota_bytes"    : UserQuota,                    bad: "0";
    SmtpHost               => "smtp.host"                 : SmtpHost,                     bad: "";
    SmtpPort               => "smtp.port"                 : SmtpPort,                     bad: "not-a-port";
    SmtpTlsMode            => "smtp.tls_mode"             : SmtpTlsMode,                  bad: "ssl";
    SmtpSender             => "smtp.sender"               : SmtpSender,                   bad: "not-a-valid-email";
    SmtpUsername           => "smtp.username"             : SmtpUsername,                 bad: "";
    SmtpPassword           => "smtp.password"             : SmtpPassword,                 bad: "";
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

    /// The accepting half of the validator: the table's rows are wired to their declared
    /// value types' parsers that say yes as well as no.
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

    #[test]
    fn posts_default_audience_validates_only_exact_tokens() {
        let key = SiteConfigKey::PostsDefaultAudience;
        for token in ["public", "subscribers", "private"] {
            assert!(key.validate(token).is_ok(), "{token:?} must validate");
        }
        for token in ["unknown", " public", "public ", "\tpublic"] {
            assert!(key.validate(token).is_err(), "{token:?} must reject");
        }
    }
}
