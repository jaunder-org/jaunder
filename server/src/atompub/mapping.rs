//! Post↔`AtomPub` entry mapping boundary.
//!
//! This module is the single coupling point between Jaunder's `Post`/`PostRecord`
//! and the `AtomPub` wire format. It converts between the storage representation
//! and the `Entry` type for both incoming (create/update) and outgoing
//! (collection member) operations.

use chrono::Utc;
use common::org::{Presence, PublicationState};
use common::post_body::{InvalidPostBody, PostBody};
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::tag::TagLabel;
use common::tagged_url::{BaseUrl, EditUriUrl, Permalink, compose};
use common::time::UtcInstant;
use host::atompub::{
    Category, Content, Entry, Link, Text, draft_marker, is_draft, set_draft, set_j_slug,
};
use storage::{PostFormat, PostRecord};

/// The post-shaped data carried by an incoming `AtomPub` `Entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostFields {
    /// Explicit title carried by the entry (`None` when absent/blank).
    pub title: Option<PostTitle>,
    /// Raw source body (in the selected format).
    pub body: PostBody,
    /// Format/markup language of the body.
    pub format: PostFormat,
    /// Optional summary/excerpt (validated `PostSummary`; an over-cap wire value
    /// is dropped on ingest, mirroring the lenient category handling below).
    pub summary: Option<PostSummary>,
    /// Categories/tags extracted from the entry, preserving whether any category
    /// elements were supplied so an explicit empty collection remains distinct
    /// from omission when Org headers are normalized.
    pub categories: Presence<Vec<TagLabel>>,
    /// The entry's explicit lifecycle source. A draft marker or `<published>`
    /// element wins over Org metadata; their absence remains absence.
    pub lifecycle: Presence<PublicationState>,
    /// Legacy Atom lifecycle fallback used after Org normalization when neither
    /// wire nor header metadata supplied a lifecycle.
    pub is_draft: bool,
}

/// The wire `atom:content` `type` for a post format (ADR-0023). `Html` uses the
/// `html` token (markup), NOT `text/html` (which would mean escaped text).
fn format_to_wire(format: PostFormat) -> &'static str {
    match format {
        PostFormat::Org => "text/org",
        PostFormat::Markdown => "text/markdown",
        PostFormat::Html => "html",
    }
}

/// Lenient inverse: never fails, falls back to `default` for `text`/absent/unknown
/// so reading is robust to any client. Tolerates a media-type parameter.
fn wire_to_format(content_type: Option<&str>, default: PostFormat) -> PostFormat {
    let Some(ct) = content_type else {
        return default;
    };
    let base = ct
        .split(';')
        .next()
        .unwrap_or(ct)
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "text/org" => PostFormat::Org,
        "text/markdown" => PostFormat::Markdown,
        "html" | "xhtml" | "text/html" => PostFormat::Html,
        _ => default,
    }
}

/// Classifies an Atom publication instant against the single clock captured for
/// the handler request.
fn classify_published(published: UtcInstant, request_clock: UtcInstant) -> PublicationState {
    if published.value() > request_clock.value() {
        PublicationState::Scheduled(published)
    } else {
        PublicationState::Published(published)
    }
}

/// Maps an incoming `AtomPub` `Entry` to Jaunder post fields.
///
/// Per ADR-0023, the entry's content `type` carries the storage format as a media
/// type, parsed leniently by [`wire_to_format`]: `text/org`→Org,
/// `text/markdown`→Markdown, `html`/`xhtml`/`text/html`→Html, and bare `text`
/// (or absent/unknown) falls back to the user's `default_format`.
///
/// The body is the **one** field lenient ingest cannot apply to: an unusable
/// summary or category is dropped and the entry still names a post, but an entry
/// with no usable content names nothing. So this is the mapping's sole failure
/// mode, and the handlers turn it into a `400`.
///
/// # Errors
///
/// Returns [`InvalidPostBody`] when the entry's content has no non-blank line —
/// including an entry that carries no content element at all (#811).
pub fn entry_to_post_fields(
    entry: &Entry,
    default_format: PostFormat,
    request_clock: UtcInstant,
) -> Result<PostFields, InvalidPostBody> {
    let (ctype, value) = entry
        .content()
        .and_then(|c| c.value().map(|v| (c.content_type(), v)))
        .unwrap_or((None, ""));

    let format = wire_to_format(ctype, default_format);

    let body: PostBody = value.parse()?;
    // A blank `<title>` means the client supplied no title: `PostTitle`'s `FromStr`
    // rejects it and `ok()` turns that into absence, so the presence policy is the
    // type's rule rather than a hand-rolled emptiness check beside it (#830).
    let title = entry.title().as_str().parse::<PostTitle>().ok();
    // The entry's `<summary>` becomes a validated `PostSummary`. Like the invalid
    // `<category>` term below, an over-cap/blank summary is silently dropped rather
    // than failing the whole entry (lenient ingest, R5) — `entry_to_post_fields`
    // stays infallible.
    let summary = entry
        .summary()
        .and_then(|t| t.as_str().parse::<PostSummary>().ok());
    // atom `<category term>` values are arbitrary RFC-4287 protocol strings (the
    // atom `Entry` model holds them as `String`, not our domain tag) — this is the
    // boundary where a conforming term becomes a `TagLabel`. `entry_to_post_fields`
    // is infallible, so an invalid term is silently skipped here: dropping a
    // malformed term keeps one bad category from failing the whole entry (R5).
    let categories = entry
        .categories()
        .iter()
        .filter_map(|c| c.term().parse::<TagLabel>().ok())
        .collect();
    let is_draft = is_draft(entry);
    // A declared `app:draft` is an explicit Atom lifecycle source, including
    // `no`; only a genuinely absent marker leaves room for Org metadata.
    let published = entry
        .published()
        .map(|published| UtcInstant::from(published.with_timezone(&Utc)));
    let lifecycle = match draft_marker(entry) {
        Some(true) => Presence::Present(PublicationState::Draft),
        Some(false) => Presence::Present(classify_published(
            published.unwrap_or(request_clock),
            request_clock,
        )),
        None => published
            .map(|published| classify_published(published, request_clock))
            .map_or(Presence::Absent, Presence::Present),
    };
    // Any incoming `j:slug` is deliberately ignored (ADR-0023): the slug is a
    // read-only server property, derived here from the title/body, never the wire.

    Ok(PostFields {
        title,
        body,
        format,
        summary,
        categories: if entry.categories().is_empty() {
            Presence::Absent
        } else {
            Presence::Present(categories)
        },
        lifecycle,
        is_draft,
    })
}

/// Builds the `AtomPub` member `Entry` for a post.
///
/// `base_url` is the site's **required** absolute origin (#560): every emitted URI (the
/// member edit URI, the stable id, the public alternate link) is composed absolute via
/// [`compose`]. Per ADR-0023 the content is emitted in *native source* form, with the
/// post's format carried in the `atom:content` `type` as a media type via
/// [`format_to_wire`]: Org→`text/org`, Markdown→`text/markdown`, Html→`html`. The
/// stable id and the `rel="edit"` link both point at the member edit URI; a public
/// `rel="alternate"` link is added only for published posts.
#[must_use]
pub fn post_to_entry(post: &PostRecord, base_url: &BaseUrl) -> Entry {
    let username = &*post.author_username;
    let edit_path = format!("/atompub/{username}/posts/{}", post.post_id);
    // `compose` joins base + the edit path (or emits the relative path when unset).
    let edit_uri: EditUriUrl = compose(base_url, &edit_path);

    // Content: the post's format becomes the wire media `type` (native source form).
    let content_type = format_to_wire(post.format);

    // Links: always an `edit` link; a public `alternate` only when published.
    let mut links = vec![Link {
        rel: "edit".into(),
        href: edit_uri.to_string(),
        ..Default::default()
    }];
    if post.published_at.is_some() {
        let alt_path = post.permalink();
        links.push(Link {
            rel: "alternate".into(),
            // Consumed inline into a `String`, so the role takes the turbofish form.
            href: compose::<Permalink>(base_url, &alt_path).to_string(),
            ..Default::default()
        });
    }

    let mut entry = Entry {
        id: edit_uri.to_string(),
        title: Text::plain(
            post.title
                .as_ref()
                .map_or_else(String::new, ToString::to_string),
        ),
        content: Some(Content {
            content_type: Some(content_type.to_string()),
            value: Some(String::from(post.body.clone())),
            ..Default::default()
        }),
        // ADR-0063 §5: read the summary out to a plain `String` at the atom Entry
        // boundary (mirrors the `title` handling elsewhere in this mapper).
        summary: post.summary.as_deref().map(|s| Text::plain(s.to_owned())),
        categories: post
            .tags
            .iter()
            .map(|t| Category {
                // atom_syndication::Category.term is an external owned String — materialize the label.
                term: t.tag_display.to_string(),
                ..Default::default()
            })
            .collect(),
        links,
        published: post.published_at.map(|d| d.value().fixed_offset()),
        updated: post.updated_at.value().fixed_offset(),
        ..Default::default()
    };

    set_draft(&mut entry, post.published_at.is_none());
    // Read-only server slug (ADR-0023): emitted on every entry, draft or live.
    // `set_j_slug` takes `&str` — it is the generic AtomPub XML-extension writer
    // (a serialization boundary, like the JSON serde bridge), not a slug-value
    // carrier; the typed `Slug` is derefed to its text here.
    set_j_slug(&mut entry, post.slug.as_ref());
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use common::ids::{PostId, TagId, UserId};
    use common::slug::Slug;
    use common::tag::{Tag, TagLabel};
    use common::test_support::{
        parse_post_body, parse_post_summary, parse_post_title, parse_slug, parse_url,
    };

    // -----------------------------------------------------------------------
    // format_wire seam tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_wire_round_trips_every_format() {
        for f in [PostFormat::Org, PostFormat::Markdown, PostFormat::Html] {
            let wire = format_to_wire(f);
            assert_eq!(
                wire_to_format(Some(wire), PostFormat::Markdown),
                f,
                "round-trip {wire}"
            );
        }
    }

    #[test]
    fn wire_to_format_is_lenient() {
        let d = PostFormat::Html; // distinctive default
        assert_eq!(wire_to_format(Some("text/org"), d), PostFormat::Org);
        assert_eq!(
            wire_to_format(Some("text/markdown"), d),
            PostFormat::Markdown
        );
        assert_eq!(
            wire_to_format(Some("text/markdown; variant=GFM"), d),
            PostFormat::Markdown
        );
        assert_eq!(
            wire_to_format(Some("html"), PostFormat::Org),
            PostFormat::Html
        );
        assert_eq!(
            wire_to_format(Some("xhtml"), PostFormat::Org),
            PostFormat::Html
        );
        assert_eq!(
            wire_to_format(Some("text/html"), PostFormat::Org),
            PostFormat::Html
        );
        assert_eq!(wire_to_format(Some("text"), d), d); // bare text → default
        assert_eq!(wire_to_format(None, d), d); // absent → default
        assert_eq!(wire_to_format(Some("application/x-weird"), d), d); // unknown → default
    }

    // -----------------------------------------------------------------------
    // entry_to_post_fields tests
    // -----------------------------------------------------------------------

    #[test]
    fn entry_to_post_fields_html_content_overrides_default_format() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="html">&lt;p&gt;HTML content&lt;/p&gt;</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.format, PostFormat::Html);
        assert_eq!(fields.body, "<p>HTML content</p>");
        assert!(!fields.is_draft);
    }

    #[test]
    fn entry_to_post_fields_xhtml_content_is_html() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">xhtml</div></content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.format, PostFormat::Html);
    }

    #[test]
    fn entry_to_post_fields_text_content_uses_default_format() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="text"># Markdown</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.format, PostFormat::Markdown);
        assert_eq!(fields.body, "# Markdown");
    }

    #[test]
    fn entry_to_post_fields_text_org_is_org() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="text/org">* Org body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        // Default is Markdown, but the explicit media type selects Org.
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.format, PostFormat::Org);
        assert_eq!(fields.body, "* Org body");
    }

    #[test]
    fn entry_to_post_fields_text_markdown_is_markdown() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="text/markdown"># Markdown body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        // Default is Org, but the explicit media type selects Markdown.
        let fields = entry_to_post_fields(&entry, PostFormat::Org, UtcInstant::from(Utc::now()))
            .expect("valid body");

        assert_eq!(fields.format, PostFormat::Markdown);
        assert_eq!(fields.body, "# Markdown body");
    }

    #[test]
    fn entry_to_post_fields_no_content_type_uses_default() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>some text</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Org, UtcInstant::from(Utc::now()))
            .expect("valid body");

        assert_eq!(fields.format, PostFormat::Org);
        assert_eq!(fields.body, "some text");
    }

    #[test]
    fn entry_to_post_fields_no_content_element_is_rejected() {
        // A body is a non-blank value, so an entry with no content element has
        // nothing to describe a post with (#811).
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");

        assert!(
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .is_err()
        );
    }

    #[test]
    fn entry_to_post_fields_summary_extraction() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
  <summary>This is a summary</summary>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(
            fields.summary,
            Some(parse_post_summary("This is a summary"))
        );
    }

    #[test]
    fn entry_to_post_fields_no_summary() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.summary, None);
    }

    #[test]
    fn entry_to_post_fields_categories_extraction() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
  <category term="rust"/>
  <category term="programming"/>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(
            fields.categories,
            Presence::Present(vec![
                "rust".parse().unwrap(),
                "programming".parse().unwrap()
            ])
        );
    }

    #[test]
    fn entry_to_post_fields_no_categories() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.categories, Presence::Absent);
    }

    #[test]
    fn entry_to_post_fields_skips_invalid_category_terms() {
        // One valid and one invalid `<category term>`: the invalid term is
        // silently dropped (R5) rather than failing the whole entry, so exactly
        // the valid label survives.
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
  <category term="rust"/>
  <category term="not a tag"/>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(
            fields.categories,
            Presence::Present(vec!["rust".parse::<TagLabel>().unwrap()])
        );
    }

    #[test]
    fn entry_to_post_fields_draft_marker_detection() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
  <app:control>
    <app:draft>yes</app:draft>
  </app:control>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert!(fields.is_draft);
        assert_eq!(fields.lifecycle, Presence::Present(PublicationState::Draft));
    }

    #[test]
    fn entry_to_post_fields_explicit_non_draft_preserves_published_instant() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><published>2026-05-30T09:15:00Z</published><content>body</content><app:control><app:draft>no</app:draft></app:control></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let published: UtcInstant = "2026-05-30T09:15:00Z".parse().expect("valid timestamp");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Published(published))
        );
    }

    #[test]
    fn entry_to_post_fields_explicit_non_draft_without_published_uses_request_clock() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><content>body</content><app:control><app:draft>no</app:draft></app:control></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Published(clock))
        );
    }

    #[test]
    fn entry_to_post_fields_no_draft_marker() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content>body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert!(!fields.is_draft);
        assert_eq!(fields.lifecycle, Presence::Absent);
    }

    #[test]
    fn entry_to_post_fields_published_without_draft_marker_uses_its_instant() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><published>2026-05-30T09:15:00Z</published><content>body</content></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let published: UtcInstant = "2026-05-30T09:15:00Z".parse().expect("valid timestamp");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Published(published))
        );
    }

    #[test]
    fn entry_to_post_fields_future_published_without_draft_marker_is_scheduled() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><published>2026-06-02T09:15:00Z</published><content>body</content></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let published: UtcInstant = "2026-06-02T09:15:00Z".parse().expect("valid timestamp");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Scheduled(published))
        );
    }

    #[test]
    fn entry_to_post_fields_published_at_request_clock_is_published() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><published>2026-06-01T12:00:00Z</published><content>body</content></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Published(clock))
        );
    }

    #[test]
    fn entry_to_post_fields_explicit_non_draft_with_future_published_is_scheduled() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app"><title>Test</title><id>id</id><updated>2026-05-31T00:00:00Z</updated><published>2026-06-02T09:15:00Z</published><content>body</content><app:control><app:draft>no</app:draft></app:control></entry>"#;
        let clock: UtcInstant = "2026-06-01T12:00:00Z".parse().expect("valid clock");
        let published: UtcInstant = "2026-06-02T09:15:00Z".parse().expect("valid timestamp");
        let entry = xml.parse::<Entry>().expect("parse entry");

        let fields = entry_to_post_fields(&entry, PostFormat::Markdown, clock).expect("valid body");

        assert_eq!(
            fields.lifecycle,
            Presence::Present(PublicationState::Scheduled(published))
        );
    }

    #[test]
    fn entry_to_post_fields_extracts_title() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>My Post Title</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="text">body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.title.as_deref(), Some("My Post Title"));
    }

    #[test]
    fn entry_to_post_fields_absent_title_is_none() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <content type="text">body</content>
</entry>"#;

        let entry = xml.parse::<Entry>().expect("parse entry");
        let fields =
            entry_to_post_fields(&entry, PostFormat::Markdown, UtcInstant::from(Utc::now()))
                .expect("valid body");

        assert_eq!(fields.title, None);
    }

    // -----------------------------------------------------------------------
    // post_to_entry tests
    // -----------------------------------------------------------------------

    /// Fields for the [`make_post`] test builder, bundled so the builder stays
    /// under the argument limit.
    struct MakePost {
        post_id: PostId,
        title: Option<common::post_title::PostTitle>,
        slug: Slug,
        body: PostBody,
        format: PostFormat,
        published_at: Option<UtcInstant>,
        summary: Option<common::post_summary::PostSummary>,
        tags: Vec<(Tag, TagLabel)>,
    }

    fn make_post(fields: MakePost) -> PostRecord {
        let MakePost {
            post_id,
            title,
            slug,
            body,
            format,
            published_at,
            summary,
            tags,
        } = fields;
        let tags_vec = tags
            .into_iter()
            .enumerate()
            .map(|(i, (tag_slug, tag_display))| storage::PostTag {
                post_id,
                tag_id: TagId::from(i64::try_from(i).expect("tag index fits in i64") + 1),
                tag_slug,
                tag_display,
            })
            .collect();

        PostRecord {
            post_id,
            user_id: UserId::from(1),
            author_username: "alice".parse().expect("parse username"),
            title,
            slug,
            body,
            format,
            rendered_html: storage::RenderedHtml::from_trusted("<p>html</p>"),
            created_at: UtcInstant::now(),
            updated_at: UtcInstant::now(),
            published_at,
            deleted_at: None,
            summary,
            tags: tags_vec,
        }
    }

    #[test]
    fn post_to_entry_markdown_format_becomes_text_content() {
        let post = make_post(MakePost {
            post_id: PostId::from(42),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("# Markdown Body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(
            entry.content().unwrap().content_type(),
            Some("text/markdown")
        );
        assert_eq!(entry.content().unwrap().value(), Some("# Markdown Body"));
    }

    #[test]
    fn post_to_entry_org_format_becomes_text_content() {
        let post = make_post(MakePost {
            post_id: PostId::from(42),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("* Org Body"),
            format: PostFormat::Org,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.content().unwrap().content_type(), Some("text/org"));
        assert_eq!(entry.content().unwrap().value(), Some("* Org Body"));
    }

    #[test]
    fn post_to_entry_html_format_becomes_html_content() {
        let post = make_post(MakePost {
            post_id: PostId::from(42),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("<p>HTML</p>"),
            format: PostFormat::Html,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.content().unwrap().content_type(), Some("html"));
        assert_eq!(entry.content().unwrap().value(), Some("<p>HTML</p>"));
    }

    #[test]
    fn post_to_entry_id_is_edit_uri() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.id, "https://example.com/atompub/alice/posts/7");
    }

    #[test]
    fn post_to_entry_edit_link() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        let edit_links: Vec<_> = entry.links().iter().filter(|l| l.rel() == "edit").collect();
        assert_eq!(edit_links.len(), 1);
        assert_eq!(
            edit_links[0].href(),
            "https://example.com/atompub/alice/posts/7"
        );
    }

    #[test]
    fn post_to_entry_published_post_has_alternate_link() {
        let now = Utc::now();
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::from(now)),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        let alternate_links: Vec<_> = entry
            .links()
            .iter()
            .filter(|l| l.rel() == "alternate")
            .collect();
        assert_eq!(alternate_links.len(), 1);
        // Permalink is date-based, so we check it contains the base URL and starts with /~
        assert!(
            alternate_links[0]
                .href()
                .starts_with("https://example.com/~alice")
        );
    }

    #[test]
    fn post_to_entry_draft_post_has_no_alternate_link() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: None, // No published_at = draft
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        let alternate_links: Vec<_> = entry
            .links()
            .iter()
            .filter(|l| l.rel() == "alternate")
            .collect();
        assert_eq!(alternate_links.len(), 0);
    }

    #[test]
    fn post_to_entry_preserves_genuine_title() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("My Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.title().as_str(), "My Title");
    }

    #[test]
    fn post_to_entry_emits_empty_title_for_untitled_post() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: None,
            slug: parse_slug("my-slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.title().as_str(), "");
    }

    #[test]
    fn post_to_entry_preserves_title_equal_to_slug() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("my-slug")),
            slug: parse_slug("my-slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.title().as_str(), "my-slug");
    }

    #[test]
    fn post_to_entry_summary() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: Some(parse_post_summary("This is a summary")),
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.summary().unwrap().as_str(), "This is a summary");
    }

    #[test]
    fn post_to_entry_no_summary() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.summary(), None);
    }

    #[test]
    fn post_to_entry_categories_from_tags() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![
                ("rust".parse().unwrap(), "Rust".parse().unwrap()),
                (
                    "programming".parse().unwrap(),
                    "Programming".parse().unwrap(),
                ),
            ],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        let terms: Vec<_> = entry.categories().iter().map(Category::term).collect();
        assert_eq!(terms, vec!["Rust", "Programming"]);
    }

    #[test]
    fn post_to_entry_no_tags() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(entry.categories().len(), 0);
    }

    #[test]
    fn post_to_entry_published_post_not_marked_draft() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert!(!is_draft(&entry));
    }

    #[test]
    fn post_to_entry_draft_post_marked_draft() {
        let post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: None, // No published_at = draft
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert!(is_draft(&entry));
    }

    #[test]
    fn post_to_entry_timestamps() {
        let now = Utc::now();
        let mut post = make_post(MakePost {
            post_id: PostId::from(7),
            title: Some(parse_post_title("Title")),
            slug: parse_slug("slug"),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            published_at: Some(UtcInstant::from(now)),
            summary: None,
            tags: vec![],
        });
        post.updated_at = UtcInstant::from(now);
        let entry = post_to_entry(&post, &parse_url("https://example.com/"));

        assert_eq!(
            entry.published().map(chrono::DateTime::timestamp),
            Some(now.timestamp())
        );
        assert_eq!(entry.updated().timestamp(), now.timestamp());
    }
}
