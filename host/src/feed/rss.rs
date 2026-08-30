use atom_syndication::Link;
use rss::extension::atom::AtomExtension;
use rss::{ChannelBuilder, GuidBuilder, ItemBuilder};

use crate::feed::{FeedItem, FeedMetadata, SyndicationFeedRepresentation};

/// Render an RSS 2.0 feed document.
///
/// RSS 2.0 has no native element for declaring the feed's own URL or a `WebSub`
/// hub — both `<self>` and `<hub>` links are conventionally emitted using
/// Atom's `<link>` element via the Atom namespace. The W3C Feed Validator
/// expects `<atom:link rel="self">`, and the `WebSub` Recommendation requires
/// `<atom:link rel="hub">` for RSS publishers (there is no RSS-native
/// alternative for either).
#[must_use]
pub fn render_rss(meta: &FeedMetadata, items: &[FeedItem]) -> SyndicationFeedRepresentation {
    let rss_items: Vec<rss::Item> = items
        .iter()
        .map(|i| {
            ItemBuilder::default()
                .title(i.title.clone().map(String::from))
                .link(Some(i.permalink.to_string()))
                .description(Some(i.content_html.to_string()))
                .pub_date(Some(i.published_at.to_rfc2822()))
                .guid(Some(
                    GuidBuilder::default()
                        .value(i.permalink.to_string())
                        .permalink(true)
                        .build(),
                ))
                .build()
        })
        .collect();

    let mut atom_links = vec![Link {
        href: meta.self_url.to_string(),
        rel: "self".into(),
        mime_type: Some("application/rss+xml".into()),
        ..Default::default()
    }];
    if let Some(hub) = &meta.hub_url {
        atom_links.push(Link {
            href: hub.to_string(),
            rel: "hub".into(),
            ..Default::default()
        });
    }

    let mut builder = ChannelBuilder::default();
    builder
        .title(meta.title.to_string())
        .link(meta.canonical_url.to_string())
        .description(
            meta.description
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )
        .last_build_date(Some(meta.updated_at.to_rfc2822()))
        .atom_ext(Some(AtomExtension { links: atom_links }))
        .items(rss_items);

    SyndicationFeedRepresentation::from_rss(builder.build().to_string())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use common::{
        ids::PostId,
        test_support::{parse_post_title, parse_url},
    };

    fn meta(hub: Option<&str>, description: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".parse::<crate::feed::FeedTitle>().unwrap(),
            description: description
                .map(|value| value.parse::<crate::feed::FeedDescription>().unwrap()),
            canonical_url: parse_url("https://example.com/"),
            self_url: parse_url("https://example.com/feed.rss"),
            hub_url: hub.map(parse_url),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item(title: Option<&str>) -> FeedItem {
        FeedItem {
            id: PostId::from(1),
            title: title.map(parse_post_title),
            permalink: parse_url("https://example.com/~alice/posts/1"),
            summary: None,
            content_html: common::test_support::rendered_html("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: vec![],
        }
    }

    #[test]
    fn renders_empty_feed() {
        let out = render_rss(&meta(None, Some("A site")), &[]);
        assert!(out.body().contains("<rss"));
        assert!(out.body().contains("<title>Site</title>"));
        assert!(!out.body().contains("<item>"));
        assert_eq!(out.format(), common::feed::FeedFormat::Rss);
        assert_eq!(
            out.content_type(),
            common::feed::FeedFormat::Rss.content_type()
        );
    }
    #[test]
    fn serializes_feed_title_and_description_presence() {
        let without = render_rss(&meta(None, None), &[]);
        let channel = rss::Channel::read_from(without.body().as_bytes()).unwrap();
        assert_eq!(channel.title(), "Site");
        assert_eq!(channel.description(), "");

        let with = render_rss(&meta(None, Some("A site")), &[]);
        let channel = rss::Channel::read_from(with.body().as_bytes()).unwrap();
        assert_eq!(channel.description(), "A site");
    }

    #[test]
    fn renders_post_with_title() {
        let out = render_rss(&meta(None, Some("A site")), &[item(Some("Hello"))]);
        assert!(out.body().contains("<title>Hello</title>"));
        assert!(
            out.body()
                .contains("<link>https://example.com/~alice/posts/1</link>")
        );
    }

    #[test]
    fn renders_titleless_post() {
        let out = render_rss(&meta(None, Some("A site")), &[item(None)]);
        let channel = rss::Channel::read_from(out.body().as_bytes()).unwrap();
        assert_eq!(channel.items().len(), 1);
        assert!(channel.items()[0].title().is_none());
        assert!(channel.items()[0].description().is_some());
    }

    #[test]
    fn emits_atom_self_link() {
        let out = render_rss(&meta(None, Some("A site")), &[]);
        assert!(
            out.body()
                .contains("xmlns:atom=\"http://www.w3.org/2005/Atom\"")
        );
        assert!(out.body().contains("<atom:link"));
        assert!(out.body().contains("rel=\"self\""));
        assert!(out.body().contains("href=\"https://example.com/feed.rss\""));
    }

    #[test]
    fn emits_atom_hub_link_when_configured() {
        let out = render_rss(&meta(Some("https://hub.example.com/"), Some("A site")), &[]);
        assert!(out.body().contains("rel=\"hub\""));
        assert!(out.body().contains("href=\"https://hub.example.com/\""));
    }

    #[test]
    fn omits_atom_hub_link_when_unset() {
        let out = render_rss(&meta(None, Some("A site")), &[]);
        assert!(!out.body().contains("rel=\"hub\""));
    }
}
