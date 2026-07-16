use atom_syndication::{Category, Content, Entry, Feed, Link, Text};

use crate::feed::metadata::{FeedItem, FeedMetadata};

#[must_use]
pub fn render_atom(meta: &FeedMetadata, items: &[FeedItem]) -> String {
    let mut links = vec![
        Link {
            href: meta.canonical_url.clone(),
            rel: "alternate".to_string(),
            ..Default::default()
        },
        Link {
            href: meta.self_url.clone(),
            rel: "self".to_string(),
            ..Default::default()
        },
    ];
    if let Some(hub) = &meta.hub_url {
        links.push(Link {
            href: hub.clone(),
            rel: "hub".to_string(),
            ..Default::default()
        });
    }

    let entries: Vec<Entry> = items
        .iter()
        .map(|i| {
            let mut entry = Entry {
                id: i.permalink.clone(),
                title: Text::plain(i.title.clone().map(String::from).unwrap_or_default()),
                updated: i.updated_at.fixed_offset(),
                published: Some(i.published_at.fixed_offset()),
                links: vec![Link {
                    href: i.permalink.clone(),
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
                entry.summary = Some(Text::plain(s.clone()));
            }
            entry
        })
        .collect();

    let feed = Feed {
        title: Text::plain(meta.title.clone()),
        id: meta.self_url.clone(),
        updated: meta.updated_at.fixed_offset(),
        subtitle: meta.description.clone().map(Text::plain),
        links,
        entries,
        ..Default::default()
    };

    feed.to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::feed::metadata::{FeedItem, FeedMetadata};
    use crate::ids::PostId;
    use crate::render::RenderedHtml;
    use crate::test_support::parse_post_title;

    fn meta(hub: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".into(),
            description: Some("A site".into()),
            canonical_url: "https://example.com/".into(),
            self_url: "https://example.com/feed.atom".into(),
            hub_url: hub.map(str::to_string),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item() -> FeedItem {
        FeedItem {
            id: PostId::from(1),
            title: Some(parse_post_title("Hello")),
            permalink: "https://example.com/~alice/posts/1".into(),
            summary: Some("hi".into()),
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: vec!["rust".parse().unwrap()],
        }
    }

    #[test]
    fn renders_empty_atom() {
        let out = render_atom(&meta(None), &[]);
        assert!(out.contains("<feed"));
        assert!(!out.contains("<entry>"));
    }

    #[test]
    fn emits_self_link() {
        let out = render_atom(&meta(None), &[]);
        assert!(out.contains("rel=\"self\""));
        assert!(out.contains("href=\"https://example.com/feed.atom\""));
    }

    #[test]
    fn includes_hub_link_when_set() {
        let out = render_atom(&meta(Some("https://hub.example.com/")), &[item()]);
        assert!(out.contains("rel=\"hub\""));
        assert!(out.contains("https://hub.example.com/"));
    }

    #[test]
    fn omits_hub_link_when_unset() {
        let out = render_atom(&meta(None), &[item()]);
        assert!(!out.contains("rel=\"hub\""));
    }

    #[test]
    fn includes_tags_as_categories() {
        let out = render_atom(&meta(None), &[item()]);
        assert!(out.contains("term=\"rust\""));
    }
}
