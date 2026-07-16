use atom_syndication::Link as AtomLink;
use rss::extension::atom::AtomExtension;
use rss::{ChannelBuilder, GuidBuilder, ItemBuilder};

use crate::feed::metadata::{FeedItem, FeedMetadata};

/// Render an RSS 2.0 feed document.
///
/// RSS 2.0 has no native element for declaring the feed's own URL or a `WebSub`
/// hub — both `<self>` and `<hub>` links are conventionally emitted using
/// Atom's `<link>` element via the Atom namespace. The W3C Feed Validator
/// expects `<atom:link rel="self">`, and the `WebSub` Recommendation requires
/// `<atom:link rel="hub">` for RSS publishers (there is no RSS-native
/// alternative for either).
#[must_use]
pub fn render_rss(meta: &FeedMetadata, items: &[FeedItem]) -> String {
    let rss_items: Vec<rss::Item> = items
        .iter()
        .map(|i| {
            ItemBuilder::default()
                .title(i.title.clone().map(String::from))
                .link(Some(i.permalink.clone()))
                .description(Some(i.content_html.to_string()))
                .pub_date(Some(i.published_at.to_rfc2822()))
                .guid(Some(
                    GuidBuilder::default()
                        .value(i.permalink.clone())
                        .permalink(true)
                        .build(),
                ))
                .build()
        })
        .collect();

    let mut atom_links = vec![AtomLink {
        href: meta.self_url.clone(),
        rel: "self".into(),
        mime_type: Some("application/rss+xml".into()),
        ..Default::default()
    }];
    if let Some(hub) = &meta.hub_url {
        atom_links.push(AtomLink {
            href: hub.clone(),
            rel: "hub".into(),
            ..Default::default()
        });
    }

    let mut builder = ChannelBuilder::default();
    builder
        .title(meta.title.clone())
        .link(meta.canonical_url.clone())
        .description(meta.description.clone().unwrap_or_default())
        .last_build_date(Some(meta.updated_at.to_rfc2822()))
        .atom_ext(Some(AtomExtension { links: atom_links }))
        .items(rss_items);

    builder.build().to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::render::RenderedHtml;
    use crate::test_support::parse_post_title;

    fn meta(hub: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".into(),
            description: Some("A site".into()),
            canonical_url: "https://example.com/".into(),
            self_url: "https://example.com/feed.rss".into(),
            hub_url: hub.map(str::to_string),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item(title: Option<&str>) -> FeedItem {
        FeedItem {
            id: 1,
            title: title.map(parse_post_title),
            permalink: "https://example.com/~alice/posts/1".into(),
            summary: None,
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: vec![],
        }
    }

    #[test]
    fn renders_empty_feed() {
        let out = render_rss(&meta(None), &[]);
        assert!(out.contains("<rss"));
        assert!(out.contains("<title>Site</title>"));
        assert!(!out.contains("<item>"));
    }

    #[test]
    fn renders_post_with_title() {
        let out = render_rss(&meta(None), &[item(Some("Hello"))]);
        assert!(out.contains("<title>Hello</title>"));
        assert!(out.contains("<link>https://example.com/~alice/posts/1</link>"));
    }

    #[test]
    fn renders_titleless_post() {
        let out = render_rss(&meta(None), &[item(None)]);
        assert!(out.contains("<item>"));
        // Description still emitted
        assert!(out.contains("<description>"));
    }

    #[test]
    fn emits_atom_self_link() {
        let out = render_rss(&meta(None), &[]);
        assert!(out.contains("xmlns:atom=\"http://www.w3.org/2005/Atom\""));
        assert!(out.contains("<atom:link"));
        assert!(out.contains("rel=\"self\""));
        assert!(out.contains("href=\"https://example.com/feed.rss\""));
    }

    #[test]
    fn emits_atom_hub_link_when_configured() {
        let out = render_rss(&meta(Some("https://hub.example.com/")), &[]);
        assert!(out.contains("rel=\"hub\""));
        assert!(out.contains("href=\"https://hub.example.com/\""));
    }

    #[test]
    fn omits_atom_hub_link_when_unset() {
        let out = render_rss(&meta(None), &[]);
        assert!(!out.contains("rel=\"hub\""));
    }
}
