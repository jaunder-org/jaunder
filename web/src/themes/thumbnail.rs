//! Deterministic public Style Contract fixture for repository thumbnails.
//!
//! The fixture deliberately uses the public projector rather than copied Theme
//! markup, so a thumbnail changes only when the versioned Style Contract does.

use anyhow::Context;
use common::{
    ids::PostId,
    render::{PostFormat, sanitize},
    root_relative_url::RootRelativeUrl,
    seed::{Page, PageSeed, PublicPresentation, RenderedPost, TagSummary, TimelineOrder},
    site::{SiteIdentity, SiteTitle},
    tag::TagLabel,
    theme::PublishedThemePresentation,
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
    application_stylesheet_url: &RootRelativeUrl,
    theme: PublishedThemePresentation,
) -> anyhow::Result<String> {
    let presentation = PublicPresentation {
        theme,
        page: PageSeed::SiteTimeline {
            identity: SiteIdentity {
                title: SiteTitle::default(),
                tagline: None,
                base_url: None,
            },
            registration_policy: common::registration::RegistrationPolicy::Open,
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
        crate::app::render_head(&presentation.page, None, application_stylesheet_url).into_string(),
        crate::app::render_theme_stylesheet(&presentation.theme).into_string(),
        crate::app::render_shell(&presentation).into_string(),
    ))
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
        content_license: None,
        rendered_title: Some(host::render::render_title(
            &"A calm place to read"
                .parse()
                .context("thumbnail fixture title is valid")?,
            &PostFormat::Html,
        )),
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
    use common::{
        ids::ThemeId,
        theme::{
            PublishedThemeIdentity, PublishedThemePresentation, ThemeAssetDigest,
            ThemeContentDigest, ThemeRevisionDigest, ThemeStylesheetDigest,
        },
    };

    use super::thumbnail_document;

    fn presentation() -> PublishedThemePresentation {
        PublishedThemePresentation {
            identity: PublishedThemeIdentity::Custom(ThemeId::from(7)),
            revision: Some(ThemeRevisionDigest::from_digest([0x11; 32])),
            stylesheet_url: ThemeStylesheetDigest::from_digest([0x22; 32]).content_url(),
            logo_url: None,
            header_url: None,
        }
    }

    #[test]
    fn fixture_is_versioned_and_contains_representative_contract_hooks() {
        let application = ThemeContentDigest::from_digest([0x55; 32]).content_url();
        let theme = presentation();
        let stylesheet = theme.stylesheet_url.clone();
        let document = thumbnail_document(&application, theme).expect("trusted fixture");
        assert!(document.contains("data-jaunder-thumbnail-fixture=\"1\""));
        assert!(document.contains(&format!("href=\"{application}\"")));
        assert!(document.contains(&format!("href=\"{stylesheet}\"")));
        for retired in ["/style/", "/theme.css", "/theme-assets/"] {
            assert!(!document.contains(retired));
        }
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
        let application = ThemeContentDigest::from_digest([0x55; 32]).content_url();
        let logo = ThemeAssetDigest::from_digest([0x33; 32]).content_url();
        let header = ThemeAssetDigest::from_digest([0x44; 32]).content_url();
        let mut theme = presentation();
        theme.logo_url = Some(logo.clone());
        theme.header_url = Some(header.clone());
        let document = thumbnail_document(&application, theme).expect("trusted fixture");
        assert!(document.contains(&format!("data-jaunder-part=\"logo\" src=\"{logo}\"")));
        assert!(document.contains(&format!(
            "data-jaunder-part=\"header-image\" src=\"{header}\""
        )));
    }
}
