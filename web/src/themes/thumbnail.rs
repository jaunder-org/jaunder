//! Deterministic public Style Contract fixture for repository thumbnails.
//!
//! The fixture deliberately uses the public projector rather than copied Theme
//! markup, so a thumbnail changes only when the versioned Style Contract does.

use anyhow::Context;
use common::{
    ids::{PostId, ThemeId},
    render::sanitize,
    root_relative_url::RootRelativeUrl,
    seed::{Page, PageSeed, PublicPresentation, RenderedPost, TagSummary, TimelineOrder},
    site::{SiteIdentity, SiteTitle},
    tag::TagLabel,
    theme::{PublishedThemeIdentity, PublishedThemePresentation, ThemeRevisionDigest},
};

const FIXTURE_VERSION: u8 = 1;

/// Builds the complete, standalone document used by `jaunder theme thumbnail`.
///
/// The document is storage-independent: its fixed post exercises the public
/// navigation, masthead, metadata, tags, and continuation hooks. The explicit
/// ready marker lets the external browser wait for both document fonts and the
/// fixture's first paint without guessing from network idleness.
///
/// # Errors
///
/// Returns an error if a source-controlled fixture value violates its domain type.
pub fn thumbnail_document(
    logo_url: Option<RootRelativeUrl>,
    header_url: Option<RootRelativeUrl>,
) -> anyhow::Result<String> {
    let presentation = PublicPresentation {
        theme: PublishedThemePresentation {
            identity: PublishedThemeIdentity::Custom(ThemeId::from(0)),
            revision: Some(fixture_revision()?),
            stylesheet_url: "/theme.css"
                .parse()
                .context("thumbnail fixture stylesheet URL is valid")?,
            logo_url,
            header_url,
        },
        page: PageSeed::SiteTimeline {
            identity: SiteIdentity {
                title: SiteTitle::default(),
                tagline: None,
                base_url: None,
            },
            order: TimelineOrder::Newest,
            page: Page {
                posts: vec![fixture_post()?],
                next_cursor: None,
                has_more: true,
            },
        },
    };
    Ok(format!(
        "<!doctype html><html lang=\"en\" data-jaunder-thumbnail-fixture=\"{FIXTURE_VERSION}\" data-jaunder-thumbnail-ready=\"0\"><head><link rel=\"icon\" href=\"data:,\">{}{}</head><body>{}<script>document.fonts.ready.then(()=>document.documentElement.dataset.jaunderThumbnailReady='1');</script></body></html>",
        crate::app::render_head(&presentation.page, None).into_string(),
        crate::app::render_theme_stylesheet(&presentation.theme).into_string(),
        crate::app::render_shell(&presentation).into_string(),
    ))
}

fn fixture_revision() -> anyhow::Result<ThemeRevisionDigest> {
    "0000000000000000000000000000000000000000000000000000000000000000"
        .parse()
        .context("thumbnail fixture digest is valid")
}

fn fixture_post() -> anyhow::Result<RenderedPost> {
    Ok(RenderedPost {
        post_id: PostId::from(1),
        username: "jaunder"
            .parse()
            .context("thumbnail fixture username is valid")?,
        display_name: Some(
            "Jaunder"
                .parse()
                .context("thumbnail fixture display name is valid")?,
        ),
        rendered_title: Some(
            "A calm place to read"
                .parse()
                .context("thumbnail fixture title is valid")?,
        ),
        summary: Some(
            "A deterministic public Style Contract preview."
                .parse()
                .context("thumbnail fixture summary is valid")?,
        ),
        slug: "style-contract"
            .parse()
            .context("thumbnail fixture slug is valid")?,
        rendered_html: sanitize("<p>Thoughtful publishing, clear reading, and durable themes.</p>"),
        created_at: "2026-01-01T00:00:00Z"
            .parse()
            .context("thumbnail fixture timestamp is valid")?,
        published_at: Some(
            "2026-01-01T00:00:00Z"
                .parse()
                .context("thumbnail fixture timestamp is valid")?,
        ),
        permalink: Some(
            "/~jaunder/style-contract"
                .parse()
                .context("thumbnail fixture permalink is valid")?,
        ),
        is_author: false,
        tags: vec![TagSummary::from(
            "Themes"
                .parse::<TagLabel>()
                .context("thumbnail fixture tag is valid")?,
        )],
    })
}

#[cfg(test)]
mod tests {
    use common::root_relative_url::RootRelativeUrl;

    use super::thumbnail_document;

    fn root_relative_url(path: &str) -> RootRelativeUrl {
        path.parse().expect("test URL is root-relative")
    }

    #[test]
    fn fixture_is_versioned_and_contains_representative_contract_hooks() {
        let document = thumbnail_document(None, None).expect("trusted fixture");
        assert!(document.contains("data-jaunder-thumbnail-fixture=\"1\""));
        assert!(document.contains("href=\"/style/jaunder.css\""));
        assert!(document.contains("href=\"/style/jaunder-themes.css\""));
        assert!(document.contains("href=\"/theme.css\""));
        for part in [
            "masthead",
            "post",
            "published-time",
            "tag-list",
            "continuation",
        ] {
            assert!(
                document.contains(&format!("data-jaunder-part=\"{part}\"")),
                "{document}"
            );
        }
        assert!(document.contains("data-jaunder-thumbnail-ready"));
        assert!(!document.contains("data-jaunder-part=\"logo\""));
        assert!(!document.contains("data-jaunder-part=\"header-image\""));
    }

    #[test]
    fn fixture_projects_supplied_theme_default_images() {
        let document = thumbnail_document(
            Some(root_relative_url("/theme-assets/assets%2Flogo.png")),
            Some(root_relative_url("/theme-assets/assets%2Fheader.png")),
        )
        .expect("trusted fixture");

        assert!(
            document.contains("data-jaunder-part=\"logo\" src=\"/theme-assets/assets%2Flogo.png\"")
        );
        assert!(document.contains(
            "data-jaunder-part=\"header-image\" src=\"/theme-assets/assets%2Fheader.png\""
        ));
    }
}
