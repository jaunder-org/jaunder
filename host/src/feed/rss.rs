use std::collections::BTreeMap;

use rss::extension::Extension;
use rss::extension::atom::{AtomExtension, Link};
use rss::extension::dublincore::DublinCoreExtension;
use rss::{ChannelBuilder, GuidBuilder, ItemBuilder};

const CREATIVE_COMMONS_NAMESPACE: &str = "http://backend.userland.com/creativeCommonsRssModule";

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
            let mut item = ItemBuilder::default()
                .title(i.visible_title.clone())
                .link(Some(i.permalink.to_string()))
                .description(Some(i.content_html.to_string()))
                .pub_date(Some(
                    i.published_at
                        .value()
                        .strftime("%a, %d %b %Y %H:%M:%S %z")
                        .to_string(),
                ))
                .guid(Some(
                    GuidBuilder::default()
                        .value(i.permalink.to_string())
                        .permalink(true)
                        .build(),
                ))
                .dublin_core_ext(Some(DublinCoreExtension {
                    rights: vec![format!(
                        "© {} {} · {}",
                        i.creation_year,
                        i.author_name,
                        i.content_license.label()
                    )],
                    ..Default::default()
                }))
                .build();
            if let Some(url) = i.content_license.canonical_url() {
                let mut extensions = BTreeMap::new();
                extensions.insert(
                    CREATIVE_COMMONS_NAMESPACE.to_owned(),
                    BTreeMap::from([(
                        "license".to_owned(),
                        vec![Extension {
                            name: "creativeCommons:license".to_owned(),
                            value: Some(url.to_owned()),
                            ..Default::default()
                        }],
                    )]),
                );
                item.set_extensions(extensions);
            }
            item
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
        .last_build_date(Some(
            meta.representation_modified_at
                .value()
                .strftime("%a, %d %b %Y %H:%M:%S %z")
                .to_string(),
        ))
        .atom_ext(Some(AtomExtension { links: atom_links }))
        .items(rss_items);

    let mut channel = builder.build();
    if items
        .iter()
        .any(|item| item.content_license.canonical_url().is_some())
    {
        channel.set_namespaces(BTreeMap::from([(
            "creativeCommons".to_owned(),
            CREATIVE_COMMONS_NAMESPACE.to_owned(),
        )]));
    }
    SyndicationFeedRepresentation::from_rss(channel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::FeedDescription;
    use crate::feed::test_support::{feed_item, feed_metadata};
    use common::{
        ids::PostId,
        test_support::{parse_url, parse_utc_instant, rendered_html},
    };

    fn meta(hub: Option<&str>, description: Option<&str>) -> FeedMetadata {
        FeedMetadata {
            description: description.map(|value| value.parse::<FeedDescription>().unwrap()),
            hub_url: hub.map(parse_url),
            ..feed_metadata(parse_url("https://example.com/feed.rss"))
        }
    }

    fn item(title: Option<&str>) -> FeedItem {
        FeedItem {
            rendered_title: title.map(common::render::sanitize_post_title),
            visible_title: title.map(ToOwned::to_owned),
            ..feed_item(
                PostId::from(1),
                parse_url("https://example.com/~alice/posts/1"),
                rendered_html("<p>hi</p>"),
                parse_utc_instant("2026-01-01T00:00:00Z"),
            )
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
    fn uses_feed_representation_time_for_last_build_date() {
        let representation_time = parse_utc_instant("2026-02-03T04:05:06Z");
        let mut metadata = meta(None, Some("A site"));
        metadata.representation_modified_at = representation_time;

        let rendered = render_rss(&metadata, &[item(Some("Hello"))]);
        let channel = rss::Channel::read_from(rendered.body().as_bytes()).unwrap();

        let expected = representation_time
            .value()
            .strftime("%a, %d %b %Y %H:%M:%S %z")
            .to_string();
        assert_eq!(channel.last_build_date(), Some(expected.as_str()));
    }

    #[test]
    fn renders_formatted_title_as_visible_text() {
        let title = common::render::sanitize_post_title("<strong>A &amp; B</strong><br>C");
        let item = FeedItem {
            visible_title: Some(crate::render::rendered_title_visible_text(&title)),
            rendered_title: Some(title),
            ..item(Some("fallback"))
        };
        let out = render_rss(&meta(None, Some("A site")), &[item]);
        let channel = rss::Channel::read_from(out.body().as_bytes()).unwrap();
        assert_eq!(channel.items()[0].title(), Some("A & B C"));
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
    fn omits_title_for_present_empty_rendered_title() {
        let item = FeedItem {
            rendered_title: Some(common::render::RenderedPostTitle::empty()),
            visible_title: None,
            ..item(Some("fallback"))
        };
        let out = render_rss(&meta(None, Some("A site")), &[item]);
        let channel = rss::Channel::read_from(out.body().as_bytes()).unwrap();
        assert_eq!(channel.items().len(), 1, "empty title must retain its item");
        assert!(channel.items()[0].title().is_none());
        assert_eq!(channel.items()[0].description(), Some("<p>hi</p>"));
    }

    #[test]
    fn serializes_rights_and_cc_license_elements_for_every_license() {
        use common::content_license::ContentLicense;
        use strum::VariantArray as _;

        for &license in ContentLicense::VARIANTS {
            let item = FeedItem {
                creation_year: 2024,
                author_name: "Alice Example".to_owned(),
                content_license: license,
                ..item(Some("Hello"))
            };
            let body = render_rss(&meta(None, Some("A site")), &[item])
                .body()
                .to_owned();
            assert!(
                body.contains("xmlns:dc=\"http://purl.org/dc/elements/1.1/\""),
                "Dublin Core namespace for {license}: {body}"
            );
            assert!(
                body.contains(&format!(
                    "<dc:rights>© 2024 Alice Example · {}</dc:rights>",
                    license.label()
                )),
                "rights for {license}: {body}"
            );
            if let Some(url) = license.canonical_url() {
                assert!(
                    body.contains(
                        "xmlns:creativeCommons=\"http://backend.userland.com/creativeCommonsRssModule\""
                    ),
                    "Creative Commons namespace for {license}: {body}"
                );
                assert!(
                    body.contains(&format!(
                        "<creativeCommons:license>{url}</creativeCommons:license>"
                    )),
                    "license element for {license}: {body}"
                );
            } else {
                assert!(
                    !body.contains("xmlns:creativeCommons="),
                    "ARR has no Creative Commons namespace: {body}"
                );
                assert!(
                    !body.contains("creativeCommons:license"),
                    "ARR has no license element: {body}"
                );
            }
        }
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
