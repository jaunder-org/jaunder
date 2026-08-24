//! Closed registry of Jaunder-owned browser `localStorage` keys (#827).
//!
//! The raw browser primitive stays in `client::storage` and intentionally accepts
//! `&str` (ADR-0069). Product code names its owned keys here, above that primitive,
//! so runtime callers do not pass transposable literals around.

/// Emits [`LocalStorageKey`] from the one product-owned key table.
macro_rules! local_storage_keys {
    ($(
        $variant:ident => $lit:literal;
    )+) => {
        /// A Jaunder-owned browser `localStorage` key.
        ///
        /// Closed by construction: parsing rejects any key not declared in the table,
        /// while [`as_str`](Self::as_str) gives product accessors the raw key expected by
        /// `client::storage`.
        #[macros::text_enum(
            error = UnknownLocalStorageKey,
            message = "unknown localStorage key"
        )]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
        pub enum LocalStorageKey {
            $(
                #[strum(serialize = $lit)]
                $variant,
            )+
        }

        impl LocalStorageKey {
            /// Returns the browser storage key string for this registry entry.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $lit, )+
                }
            }
        }
    };
}

local_storage_keys! {
    AuthMarker => "jaunder_auth";
    Theme => "jaunder_theme";
    HomeRedirectPreference => "jaunder_home_redirect";
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use strum::VariantArray as _;

    use super::*;

    #[test]
    fn every_key_round_trips_its_storage_string() {
        for key in LocalStorageKey::VARIANTS {
            let storage_key = key.as_str();
            assert_eq!(key.as_ref(), storage_key);
            assert_eq!(
                LocalStorageKey::from_str(storage_key).ok().as_ref(),
                Some(key)
            );
            assert!(!storage_key.is_empty());
        }
        assert_eq!(LocalStorageKey::VARIANTS.len(), 3);
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
