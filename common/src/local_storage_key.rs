//! Closed registry of Jaunder-owned browser `localStorage` keys (#827).
//!
//! The raw browser primitive stays in `client::storage` and intentionally accepts
//! `&str` (ADR-0069). Product code names its owned keys here, above that primitive,
//! so runtime callers do not pass transposable literals around.

/// A Jaunder-owned browser `localStorage` key.
///
/// Closed by construction: parsing rejects any key not declared here, while
/// `text_enum`'s generated `AsRef<str>` gives product accessors the raw key
/// expected by `client::storage`.
#[macros::text_enum(
    error = UnknownLocalStorageKey,
    message = "unknown localStorage key"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
pub enum LocalStorageKey {
    #[strum(serialize = "jaunder_auth")]
    AuthMarker,
    #[strum(serialize = "jaunder_home_redirect")]
    HomeRedirectPreference,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use strum::VariantArray as _;

    use super::*;

    #[test]
    fn every_key_round_trips_its_storage_string() {
        for key in LocalStorageKey::VARIANTS {
            let storage_key = key.as_ref();
            assert_eq!(
                LocalStorageKey::from_str(storage_key).ok().as_ref(),
                Some(key)
            );
            assert!(!storage_key.is_empty());
        }
        assert_eq!(LocalStorageKey::VARIANTS.len(), 2);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        for bad in ["", "jaunder", "jaunder_auth ", "jaunder_nope"] {
            assert!(LocalStorageKey::from_str(bad).is_err(), "{bad} must reject");
        }
        let err = LocalStorageKey::from_str("jaunder_nope").unwrap_err();
        assert_eq!(err.to_string(), "unknown localStorage key");
    }
}
