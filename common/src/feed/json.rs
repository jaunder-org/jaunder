use serde_json::{Value, json};

use crate::feed::metadata::{FeedItem, FeedMetadata};

#[must_use]
pub fn render_json(meta: &FeedMetadata, items: &[FeedItem]) -> String {
    let json_items: Vec<Value> = items
        .iter()
        .map(|i| {
            let mut o = json!({
                "id": &*i.permalink,
                "url": &*i.permalink,
                "content_html": &*i.content_html,
                "date_published": i.published_at.to_rfc3339(),
                "date_modified": i.updated_at.to_rfc3339(),
            });
            if let Some(t) = &i.title {
                o["title"] = Value::String(t.to_string());
            }
            if let Some(s) = &i.summary {
                // ADR-0063 §5: read the summary out to a plain `String` at the
                // serde_json boundary (mirrors the `title` handling above).
                o["summary"] = Value::String(s.to_string());
            }
            if !i.tags.is_empty() {
                o["tags"] = json!(i.tags);
            }
            o
        })
        .collect();

    let mut root = json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": meta.title,
        "home_page_url": &*meta.canonical_url,
        "feed_url": &*meta.self_url,
        "items": json_items,
    });
    if let Some(d) = &meta.description {
        root["description"] = Value::String(d.to_string());
    }
    if let Some(hub) = &meta.hub_url {
        root["hubs"] = json!([{ "type": "WebSub", "url": hub.as_ref() }]);
    }
    root.to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::ids::PostId;
    use crate::post_summary::PostSummary;
    use crate::post_title::PostTitle;
    use crate::render::RenderedHtml;
    use crate::test_support::{parse_post_summary, parse_post_title, parse_url};

    fn meta(hub: Option<&str>, description: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".parse::<crate::feed::FeedTitle>().unwrap(),
            description: description
                .map(|value| value.parse::<crate::feed::FeedDescription>().unwrap()),
            canonical_url: parse_url("https://example.com/"),
            self_url: parse_url("https://example.com/feed.json"),
            hub_url: hub.map(parse_url),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item(title: Option<PostTitle>, tags: Vec<&str>) -> FeedItem {
        item_with_summary(title, tags, None)
    }

    fn item_with_summary(
        title: Option<PostTitle>,
        tags: Vec<&str>,
        summary: Option<PostSummary>,
    ) -> FeedItem {
        FeedItem {
            id: PostId::from(1),
            title,
            permalink: parse_url("https://example.com/~alice/posts/1"),
            summary,
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: tags.into_iter().map(|t| t.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn renders_empty_jsonfeed() {
        let out = render_json(&meta(None, Some("A site")), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], "https://jsonfeed.org/version/1.1");
        assert!(v["items"].as_array().unwrap().is_empty());
    }
    #[test]
    fn serializes_feed_title_and_description_presence() {
        let without: Value = serde_json::from_str(&render_json(&meta(None, None), &[])).unwrap();
        assert_eq!(without["title"], "Site");
        assert!(without.get("description").is_none());

        let with: Value =
            serde_json::from_str(&render_json(&meta(None, Some("A site")), &[])).unwrap();
        assert_eq!(with["description"], "A site");
    }

    #[test]
    fn emits_feed_url_as_self() {
        let out = render_json(&meta(None, Some("A site")), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["feed_url"], "https://example.com/feed.json");
    }

    #[test]
    fn includes_hub_when_set() {
        let out = render_json(&meta(Some("https://hub.example.com/"), Some("A site")), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hubs"][0]["type"], "WebSub");
        assert_eq!(v["hubs"][0]["url"], "https://hub.example.com/");
    }

    #[test]
    fn omits_title_for_titleless_post() {
        let out = render_json(&meta(None, Some("A site")), &[item(None, vec![])]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["items"][0].get("title").is_none());
    }

    #[test]
    fn includes_summary_when_present() {
        let out = render_json(
            &meta(None, Some("A site")),
            &[item_with_summary(
                Some(parse_post_title("t")),
                vec![],
                Some(parse_post_summary("a summary")),
            )],
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["summary"], "a summary");
    }

    #[test]
    fn includes_tags_only_when_present() {
        let out = render_json(
            &meta(None, Some("A site")),
            &[item(Some(parse_post_title("t")), vec!["rust"])],
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["tags"][0], "rust");
        let out2 = render_json(
            &meta(None, Some("A site")),
            &[item(Some(parse_post_title("t")), vec![])],
        );
        let v2: Value = serde_json::from_str(&out2).unwrap();
        assert!(v2["items"][0].get("tags").is_none());
    }
}
