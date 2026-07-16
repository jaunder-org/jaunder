use serde_json::{json, Value};

use crate::feed::metadata::{FeedItem, FeedMetadata};

#[must_use]
pub fn render_json(meta: &FeedMetadata, items: &[FeedItem]) -> String {
    let json_items: Vec<Value> = items
        .iter()
        .map(|i| {
            let mut o = json!({
                "id": i.permalink,
                "url": i.permalink,
                "content_html": &*i.content_html,
                "date_published": i.published_at.to_rfc3339(),
                "date_modified": i.updated_at.to_rfc3339(),
            });
            if let Some(t) = &i.title {
                o["title"] = Value::String(t.to_string());
            }
            if let Some(s) = &i.summary {
                o["summary"] = Value::String(s.clone());
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
        "home_page_url": meta.canonical_url,
        "feed_url": meta.self_url,
        "items": json_items,
    });
    if let Some(d) = &meta.description {
        root["description"] = Value::String(d.clone());
    }
    if let Some(hub) = &meta.hub_url {
        root["hubs"] = json!([{ "type": "WebSub", "url": hub }]);
    }
    root.to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::ids::PostId;
    use crate::render::RenderedHtml;
    use crate::test_support::parse_post_title;

    fn meta(hub: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            title: "Site".into(),
            description: Some("A site".into()),
            canonical_url: "https://example.com/".into(),
            self_url: "https://example.com/feed.json".into(),
            hub_url: hub.map(str::to_string),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    fn item(title: Option<&str>, tags: Vec<&str>) -> FeedItem {
        item_with_summary(title, tags, None)
    }

    fn item_with_summary(title: Option<&str>, tags: Vec<&str>, summary: Option<&str>) -> FeedItem {
        FeedItem {
            id: PostId::from(1),
            title: title.map(parse_post_title),
            permalink: "https://example.com/~alice/posts/1".into(),
            summary: summary.map(str::to_string),
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),
            published_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            tags: tags.into_iter().map(|t| t.parse().unwrap()).collect(),
        }
    }

    #[test]
    fn renders_empty_jsonfeed() {
        let out = render_json(&meta(None), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], "https://jsonfeed.org/version/1.1");
        assert!(v["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn emits_feed_url_as_self() {
        let out = render_json(&meta(None), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["feed_url"], "https://example.com/feed.json");
    }

    #[test]
    fn includes_hub_when_set() {
        let out = render_json(&meta(Some("https://hub.example.com/")), &[]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hubs"][0]["type"], "WebSub");
        assert_eq!(v["hubs"][0]["url"], "https://hub.example.com/");
    }

    #[test]
    fn omits_title_for_titleless_post() {
        let out = render_json(&meta(None), &[item(None, vec![])]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["items"][0].get("title").is_none());
    }

    #[test]
    fn includes_summary_when_present() {
        let out = render_json(
            &meta(None),
            &[item_with_summary(Some("t"), vec![], Some("a summary"))],
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["summary"], "a summary");
    }

    #[test]
    fn includes_tags_only_when_present() {
        let out = render_json(&meta(None), &[item(Some("t"), vec!["rust"])]);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["items"][0]["tags"][0], "rust");
        let out2 = render_json(&meta(None), &[item(Some("t"), vec![])]);
        let v2: Value = serde_json::from_str(&out2).unwrap();
        assert!(v2["items"][0].get("tags").is_none());
    }
}
