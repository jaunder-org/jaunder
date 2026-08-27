//! Closed per-user configuration-key registry.

use std::str::FromStr;

use thiserror::Error;

use crate::render::PostFormat;
/// Error returned when a stored or offered per-user value does not parse as its key's
/// type.
///
/// Separate from the host-owned site-config error because the two registries are
/// closed sets: a `user_config` failure can never name a site key.
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
/// This follows the host-owned site-key registry's table shape, minus the
/// `{ optional }` marker: no per-user key uses the empty-means-unset contract.
/// Each row is `Variant => "dotted.key" : <value>, bad: "<example>";`.
macro_rules! user_config_keys {
    ($(
        $variant:ident => $lit:literal : $value:ident , bad: $bad:literal ;
    )+) => {
        /// A per-user configuration key — the only way to name one.
        ///
        /// Closed construction keeps a typo from writing a `user_config` row
        /// that no typed reader can reach.
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
