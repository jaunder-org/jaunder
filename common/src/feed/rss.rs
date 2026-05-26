use crate::feed::metadata::{FeedItem, FeedMetadata};
use rss::{ChannelBuilder, GuidBuilder, ItemBuilder};

/// Render an RSS 2.0 feed document.
///
/// # Panics
///
/// Panics if the underlying RSS writer fails to produce valid UTF-8 output,
/// which should not happen for well-formed `FeedMetadata` and `FeedItem` inputs.
#[must_use]
pub fn render_rss(meta: &FeedMetadata, items: &[FeedItem]) -> String {
    let rss_items: Vec<rss::Item> = items
        .iter()
        .map(|i| {
            ItemBuilder::default()
                .title(i.title.clone())
                .link(Some(i.permalink.clone()))
                .description(Some(i.content_html.clone()))
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

    let mut builder = ChannelBuilder::default();
    builder
        .title(meta.title.clone())
        .link(meta.canonical_url.clone())
        .description(meta.description.clone().unwrap_or_default())
        .last_build_date(Some(meta.updated_at.to_rfc2822()))
        .items(rss_items);

    let mut channel = builder.build();
    // atom:link rel=self
    let mut ns = std::collections::BTreeMap::new();
    ns.insert(
        "atom".to_string(),
        "http://www.w3.org/2005/Atom".to_string(),
    );
    channel.set_namespaces(ns);

    let mut buf = Vec::new();
    channel
        .pretty_write_to(&mut buf, b' ', 2)
        .expect("write rss");
    String::from_utf8(buf).expect("rss is utf-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn meta() -> FeedMetadata {
        FeedMetadata {
            title: "Site".into(),
            description: Some("A site".into()),
            canonical_url: "https://example.com/".into(),
            self_url: "https://example.com/feed.rss".into(),
            hub_url: None,
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item(title: Option<&str>) -> FeedItem {
        FeedItem {
            id: 1,
            title: title.map(str::to_string),
            permalink: "https://example.com/~alice/posts/1".into(),
            summary: None,
            content_html: "<p>hi</p>".into(),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: vec![],
        }
    }

    #[test]
    fn renders_empty_feed() {
        let out = render_rss(&meta(), &[]);
        assert!(out.contains("<rss"));
        assert!(out.contains("<title>Site</title>"));
        assert!(!out.contains("<item>"));
    }

    #[test]
    fn renders_post_with_title() {
        let out = render_rss(&meta(), &[item(Some("Hello"))]);
        assert!(out.contains("<title>Hello</title>"));
        assert!(out.contains("<link>https://example.com/~alice/posts/1</link>"));
    }

    #[test]
    fn renders_titleless_post() {
        let out = render_rss(&meta(), &[item(None)]);
        assert!(out.contains("<item>"));
        // Description still emitted
        assert!(out.contains("<description>"));
    }
}
