use atom_syndication::{Category, Content, Entry, Feed, Link, Text};

use crate::feed::{FeedItem, FeedMetadata, SyndicationFeedRepresentation};

#[must_use]
pub fn render_atom(meta: &FeedMetadata, items: &[FeedItem]) -> SyndicationFeedRepresentation {
    let mut links = vec![
        Link {
            href: meta.canonical_url.to_string(),
            rel: "alternate".to_string(),
            ..Default::default()
        },
        Link {
            href: meta.self_url.to_string(),
            rel: "self".to_string(),
            ..Default::default()
        },
    ];
    if let Some(hub) = &meta.hub_url {
        links.push(Link {
            href: hub.to_string(),
            rel: "hub".to_string(),
            ..Default::default()
        });
    }

    let entries: Vec<Entry> = items
        .iter()
        .map(|i| {
            let mut entry = Entry {
                id: i.permalink.to_string(),
                title: Text::plain(i.title.clone().map(String::from).unwrap_or_default()),
                updated: i.updated_at.fixed_offset(),
                published: Some(i.published_at.fixed_offset()),
                links: vec![Link {
                    href: i.permalink.to_string(),
                    rel: "alternate".to_string(),
                    ..Default::default()
                }],
                content: Some(Content {
                    content_type: Some("html".to_string()),
                    value: Some(i.content_html.to_string()),
                    ..Default::default()
                }),
                categories: i
                    .tags
                    .iter()
                    .map(|t| Category {
                        // atom_syndication::Category.term is an external owned String — materialize the label.
                        term: t.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            if let Some(s) = &i.summary {
                // ADR-0063 §5: read the summary out to a plain `String` at the
                // atom_syndication boundary (mirrors the `title` handling above).
                entry.summary = Some(Text::plain(s.to_string()));
            }
            entry
        })
        .collect();

    let feed = Feed {
        title: Text::plain(meta.title.to_string()),
        id: meta.self_url.to_string(),
        updated: meta.updated_at.fixed_offset(),
        subtitle: meta
            .description
            .as_ref()
            .map(|description| Text::plain(description.to_string())),
        links,
        entries,
        ..Default::default()
    };

    SyndicationFeedRepresentation::from_atom(feed.to_string())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::feed::metadata::{FeedItem, FeedMetadata};
    use common::{
        ids::PostId,
        render::RenderedHtml,
        test_support::{parse_post_summary, parse_post_title, parse_url},
    };

    fn meta(hub: Option<&str>, description: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".parse::<crate::feed::FeedTitle>().unwrap(),
            description: description
                .map(|value| value.parse::<crate::feed::FeedDescription>().unwrap()),
            canonical_url: parse_url("https://example.com/"),
            self_url: parse_url("https://example.com/feed.atom"),
            hub_url: hub.map(parse_url),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item() -> FeedItem {
        FeedItem {
            id: PostId::from(1),
            title: Some(parse_post_title("Hello")),
            permalink: parse_url("https://example.com/~alice/posts/1"),
            summary: Some(parse_post_summary("hi")),
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: vec!["rust".parse().unwrap()],
        }
    }

    #[test]
    fn renders_empty_atom() {
        let out = render_atom(&meta(None, Some("A site")), &[]);
        assert!(out.body().contains("<feed"));
        assert!(!out.body().contains("<entry>"));
        assert_eq!(out.format(), crate::feed::FeedFormat::Atom);
        assert_eq!(
            out.content_type(),
            crate::feed::FeedFormat::Atom.content_type()
        );
    }
    #[test]
    fn serializes_feed_title_and_description_presence() {
        let without = render_atom(&meta(None, None), &[]);
        assert!(without.body().contains("<title>Site</title>"));
        assert!(!without.body().contains("<subtitle"));

        let with = render_atom(&meta(None, Some("A site")), &[]);
        assert!(with.body().contains("<subtitle>A site</subtitle>"));
    }

    #[test]
    fn renders_explicit_title_for_titled_post() {
        let out = render_atom(&meta(None, Some("A site")), &[item()]);
        let body = out.body();

        assert!(body.contains("<title>Hello</title>"), "out: {body}");
        assert!(!body.contains("<title></title>"), "out: {body}");
    }

    #[test]
    fn renders_empty_title_for_titleless_post() {
        let mut item = item();
        item.title = None;

        let out = render_atom(&meta(None, Some("A site")), &[item]);
        let body = out.body();

        assert_eq!(body.matches("<title></title>").count(), 1, "out: {body}");
        assert!(!body.contains("<title>hi</title>"), "out: {body}");
    }

    #[test]
    fn emits_self_link() {
        let out = render_atom(&meta(None, Some("A site")), &[]);
        assert!(out.body().contains("rel=\"self\""));
        assert!(
            out.body()
                .contains("href=\"https://example.com/feed.atom\"")
        );
    }

    #[test]
    fn includes_hub_link_when_set() {
        let out = render_atom(
            &meta(Some("https://hub.example.com/"), Some("A site")),
            &[item()],
        );
        assert!(out.body().contains("rel=\"hub\""));
        assert!(out.body().contains("https://hub.example.com/"));
    }

    #[test]
    fn omits_hub_link_when_unset() {
        let out = render_atom(&meta(None, Some("A site")), &[item()]);
        assert!(!out.body().contains("rel=\"hub\""));
    }

    #[test]
    fn includes_tags_as_categories() {
        let out = render_atom(&meta(None, Some("A site")), &[item()]);
        assert!(out.body().contains("term=\"rust\""));
    }

    #[test]
    fn per_item_urls_are_absolute() {
        // #560 / AC5: the entry `atom:id` and alternate `<link>` render the composed
        // *absolute* permalink — never a relative `/…` atom:id (RFC-4287 requires an
        // absolute IRI).
        let out = render_atom(&meta(None, Some("A site")), &[item()]);
        let body = out.body();
        assert!(
            body.contains("https://example.com/~alice/posts/1"),
            "entry permalink should be absolute: {body}"
        );
        assert!(
            !body.contains("<id>/"),
            "no entry/feed id should be root-relative: {body}"
        );
    }
}
