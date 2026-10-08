//! Pure projector-seed decoding shared by CSR boot and host tests.

use common::seed::{PageSeed, PublicPresentation};

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
pub fn decode_projector_seed(
    raw: Option<&str>,
) -> Result<Option<PublicPresentation<PageSeed>>, serde_json::Error> {
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
    fn valid_presentation_retains_the_server_resolved_destination_theme() {
        let json = format!(
            r#"{{"theme":{{"identity":{{"kind":"built_in","value":"reader"}},"revision":"{}","stylesheet_url":"/theme/{}","logo_url":null,"header_url":null}},"page":{{"SiteTimeline":{{"identity":{{"title":"Jaunder","base_url":null}},"registration_policy":"open","order":"newest","page":{{"posts":[],"next_cursor":null,"has_more":false}}}}}}}}"#,
            "a".repeat(64),
            "b".repeat(64),
        );

        let decoded = decode_projector_seed(Some(&json)).expect("valid presentation");
        assert!(matches!(
            decoded,
            Some(PublicPresentation {
                theme,
                page: PageSeed::SiteTimeline {
                    identity,
                    order: common::seed::TimelineOrder::Newest,
                    ..
                },
            }) if theme == common::theme::PublishedThemePresentation {
                identity: common::theme::PublishedThemeIdentity::BuiltIn(common::theme::Theme::Reader),
                revision: Some("a".repeat(64).parse().unwrap()),
                stylesheet_url: format!("/theme/{}", "b".repeat(64)).parse().unwrap(),
                logo_url: None,
                header_url: None,
            }
                && identity.title == common::site::SiteTitle::default()
                && identity.tagline.is_none()
                && identity.base_url.is_none()
        ));
    }
}
