//! TEMPORARY spike (#687 plan, Task 2). Proves two things before the 19-entry
//! registry is built on them:
//!
//! 1. `#[macros::text_enum(...)]` survives `macro_rules!` expansion as the item's
//!    first active attribute (ADR-0091 requires that position).
//! 2. A `$lit:literal` metavariable substituted into `#[strum(serialize = $lit)]`
//!    survives `syn`'s parse.
//!
//! Deleted in the same task. Do not build on this file.

macro_rules! spike_keys {
    ($($variant:ident => $lit:literal),+ $(,)?) => {
        #[macros::text_enum(sqlx, error = InvalidSpikeKey, message = "unknown spike key")]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, strum::VariantArray)]
        pub enum SpikeKey {
            $( #[strum(serialize = $lit)] $variant, )+
        }
    };
}

spike_keys! {
    SiteTitle => "site.title",
    FeedsMinDays => "feeds.min_days",
}

#[cfg(test)]
mod tests {
    use super::SpikeKey;
    use std::str::FromStr;

    #[test]
    fn spike_key_round_trips_its_dotted_form() {
        assert_eq!(SpikeKey::from_str("site.title").unwrap(), SpikeKey::SiteTitle);
        assert_eq!(SpikeKey::SiteTitle.as_ref(), "site.title");
        assert!(SpikeKey::from_str("nope").is_err());
    }
}
