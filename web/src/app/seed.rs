//! Pure projector-seed decoding shared by CSR boot and host tests.

use common::seed::PageSeed;

/// Decode the optional projector seed blob.
///
/// A missing blob is ordinary SPA-shell control flow. A present blob must decode
/// as the real [`PageSeed`] contract; syntax and shape failures are returned to
/// the CSR caller for one swallowed-browser report.
///
/// # Errors
///
/// Returns the projector seed's [`serde_json::Error`] when a present blob is
/// malformed or does not match [`PageSeed`].
pub fn decode_projector_seed(raw: Option<&str>) -> Result<Option<PageSeed>, serde_json::Error> {
    raw.map(serde_json::from_str).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_seed_is_silent() {
        assert!(matches!(decode_projector_seed(None), Ok(None)));
    }

    #[test]
    fn malformed_and_wrong_shape_seeds_fail() {
        assert!(decode_projector_seed(Some("{")).is_err());
        assert!(decode_projector_seed(Some("{}")).is_err());
    }

    #[test]
    fn valid_page_seed_decodes() {
        let json = r#"{"SiteTimeline":{"posts":[],"has_more":false,"next_cursor":null}}"#;

        assert!(matches!(
            decode_projector_seed(Some(json)),
            Ok(Some(PageSeed::SiteTimeline(_)))
        ));
    }
}
