//! Post↔`AtomPub` entry mapping boundary.
//!
//! This module is the single coupling point between Jaunder's `Post`/`PostRecord`
//! and the `AtomPub` wire format. It converts between the storage representation
//! and the `Entry` type for both incoming (create/update) and outgoing
//! (collection member) operations.

use chrono::{DateTime, Utc};
use common::atompub::{is_draft, set_draft, set_j_slug, Category, Content, Entry, Link, Text};
use common::tag::TagLabel;
use storage::{PostFormat, PostRecord};

/// The post-shaped data carried by an incoming `AtomPub` `Entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostFields {
    /// Explicit title carried by the entry (`None` when absent/blank).
    pub title: Option<String>,
    /// Raw source body (in the selected format).
    pub body: String,
    /// Format/markup language of the body.
    pub format: PostFormat,
    /// Optional summary/excerpt.
    pub summary: Option<String>,
    /// Categories/tags extracted from the entry (author labels).
    pub categories: Vec<TagLabel>,
    /// Whether the entry is marked as draft.
    pub is_draft: bool,
    /// Explicit publication time from the entry's `<published>` element
    /// (`None` when absent). A future time schedules the post; a past time
    /// backdates it. The inverse of [`post_to_entry`]'s `published` mapping.
    pub published: Option<DateTime<Utc>>,
}

/// The wire `atom:content` `type` for a post format (ADR-0023). `Html` uses the
/// `html` token (markup), NOT `text/html` (which would mean escaped text).
fn format_to_wire(format: &PostFormat) -> &'static str {
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

/// Maps an incoming `AtomPub` `Entry` to Jaunder post fields.
///
/// Per ADR-0023, the entry's content `type` carries the storage format as a media
/// type, parsed leniently by [`wire_to_format`]: `text/org`→Org,
/// `text/markdown`→Markdown, `html`/`xhtml`/`text/html`→Html, and bare `text`
/// (or absent/unknown) falls back to the user's `default_format`. Body is the
/// content value (empty string when the entry carries no content).
#[must_use]
pub fn entry_to_post_fields(entry: &Entry, default_format: PostFormat) -> PostFields {
    let (ctype, value) = entry
        .content()
        .and_then(|c| c.value().map(|v| (c.content_type(), v)))
        .unwrap_or((None, ""));

    let format = wire_to_format(ctype, default_format);

    let body = value.to_string();
    let title = {
        let trimmed = entry.title().as_str().trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    let summary = entry.summary().map(|t| t.as_str().to_string());
    // atom `<category term>` values are arbitrary RFC-4287 protocol strings (the
    // atom `Entry` model holds them as `String`, not our domain tag) — this is the
    // boundary where a conforming term becomes a `TagLabel`. `entry_to_post_fields`
    // is infallible, so an invalid term is silently skipped here (the sole ingest
    // skip — `post_tag_diff` no longer filters, since every `TagLabel` it receives
    // is already valid). Dropping a malformed term keeps one bad category from
    // failing the whole entry (R5).
    let categories = entry
        .categories()
        .iter()
        .filter_map(|c| c.term().parse::<TagLabel>().ok())
        .collect();
    let is_draft = is_draft(entry);
    // Any incoming `j:slug` is deliberately ignored (ADR-0023): the slug is a
    // read-only server property, derived here from the title/body, never the wire.
    // Inverse of `post_to_entry`'s `published: post.published_at.map(fixed_offset)`:
    // read the entry's `<published>` (a fixed-offset datetime) back to UTC.
    let published = entry.published().map(|d| d.with_timezone(&Utc));

    PostFields {
        title,
        body,
        format,
        summary,
        categories,
        is_draft,
        published,
    }
}

/// Builds the `AtomPub` member `Entry` for a post.
///
/// `base_url` is the site's absolute origin (no trailing slash), e.g.
/// `https://example.com`. Per ADR-0023 the content is emitted in *native source*
/// form, with the post's format carried in the `atom:content` `type` as a media
/// type via [`format_to_wire`]: Org→`text/org`, Markdown→`text/markdown`,
/// Html→`html`. The stable id and the `rel="edit"` link both point at the member
/// edit URI; a public `rel="alternate"` link is added only for published posts.
#[must_use]
pub fn post_to_entry(post: &PostRecord, base_url: &str) -> Entry {
    let username = &*post.author_username;
    let edit_uri = format!("{base_url}/atompub/{username}/posts/{}", post.post_id);

    // Content: the post's format becomes the wire media `type` (native source form).
    let content_type = format_to_wire(&post.format);
    let body = post.body.clone();

    // Links: always an `edit` link; a public `alternate` only when published.
    let mut links = vec![Link {
        rel: "edit".into(),
        href: edit_uri.clone(),
        ..Default::default()
    }];
    if post.published_at.is_some() {
        links.push(Link {
            rel: "alternate".into(),
            href: format!("{base_url}{}", post.permalink()),
            ..Default::default()
        });
    }

    let mut entry = Entry {
        id: edit_uri,
        title: Text::plain(post.title.clone().unwrap_or_else(|| post.slug.to_string())),
        content: Some(Content {
            content_type: Some(content_type.to_string()),
            value: Some(body),
            ..Default::default()
        }),
        summary: post.summary.clone().map(Text::plain),
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
        published: post.published_at.map(|d| d.fixed_offset()),
        updated: post.updated_at.fixed_offset(),
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
    use chrono::{DateTime, Utc};

    // -----------------------------------------------------------------------
    // format_wire seam tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_wire_round_trips_every_format() {
        for f in [PostFormat::Org, PostFormat::Markdown, PostFormat::Html] {
            let wire = format_to_wire(&f);
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
        assert_eq!(wire_to_format(Some("text/org"), d.clone()), PostFormat::Org);
        assert_eq!(
            wire_to_format(Some("text/markdown"), d.clone()),
            PostFormat::Markdown
        );
        assert_eq!(
            wire_to_format(Some("text/markdown; variant=GFM"), d.clone()),
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
        assert_eq!(wire_to_format(Some("text"), d.clone()), d.clone()); // bare text → default
        assert_eq!(wire_to_format(None, d.clone()), d.clone()); // absent → default
        assert_eq!(wire_to_format(Some("application/x-weird"), d.clone()), d); // unknown → default
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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        // Default is Markdown, but the explicit media type selects Org.
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        // Default is Org, but the explicit media type selects Markdown.
        let fields = entry_to_post_fields(&entry, PostFormat::Org);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Org);

        assert_eq!(fields.format, PostFormat::Org);
        assert_eq!(fields.body, "some text");
    }

    #[test]
    fn entry_to_post_fields_no_content_element_yields_empty_body() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.body, "");
        assert_eq!(fields.format, PostFormat::Markdown);
    }

    #[test]
    fn entry_to_post_fields_summary_extraction() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <summary>This is a summary</summary>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.summary, Some("This is a summary".to_string()));
    }

    #[test]
    fn entry_to_post_fields_no_summary() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.summary, None);
    }

    #[test]
    fn entry_to_post_fields_categories_extraction() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <category term="rust"/>
  <category term="programming"/>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.categories, vec!["rust", "programming"]);
    }

    #[test]
    fn entry_to_post_fields_no_categories() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.categories, Vec::<TagLabel>::new());
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
  <category term="rust"/>
  <category term="not a tag"/>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.categories, vec!["rust".parse::<TagLabel>().unwrap()]);
    }

    #[test]
    fn entry_to_post_fields_draft_marker_detection() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
  <app:control>
    <app:draft>yes</app:draft>
  </app:control>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert!(fields.is_draft);
    }

    #[test]
    fn entry_to_post_fields_no_draft_marker() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Test</title>
  <id>id</id>
  <updated>2026-05-31T00:00:00Z</updated>
</entry>"#;

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert!(!fields.is_draft);
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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

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

        let entry = common::atompub::entry_from_xml(xml).expect("parse entry");
        let fields = entry_to_post_fields(&entry, PostFormat::Markdown);

        assert_eq!(fields.title, None);
    }

    // -----------------------------------------------------------------------
    // post_to_entry tests
    // -----------------------------------------------------------------------

    /// Fields for the [`make_post`] test builder, bundled so the builder stays
    /// under the argument limit.
    struct MakePost<'a> {
        post_id: i64,
        title: Option<&'a str>,
        slug: &'a str,
        body: &'a str,
        format: PostFormat,
        published_at: Option<DateTime<Utc>>,
        summary: Option<&'a str>,
        tags: Vec<(String, String)>,
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
                tag_id: i64::try_from(i).expect("tag index fits in i64") + 1,
                tag_slug: tag_slug.parse().expect("parse tag"),
                tag_display: tag_display.parse().expect("parse tag label"),
            })
            .collect();

        PostRecord {
            post_id,
            user_id: 1,
            author_username: "alice".parse().expect("parse username"),
            title: title.map(std::string::ToString::to_string),
            slug: slug.parse().expect("parse slug"),
            body: body.to_string(),
            format,
            rendered_html: "<p>html</p>".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            published_at,
            deleted_at: None,
            summary: summary.map(std::string::ToString::to_string),
            tags: tags_vec,
        }
    }

    #[test]
    fn post_to_entry_markdown_format_becomes_text_content() {
        let post = make_post(MakePost {
            post_id: 42,
            title: Some("Title"),
            slug: "slug",
            body: "# Markdown Body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(
            entry.content().unwrap().content_type(),
            Some("text/markdown")
        );
        assert_eq!(entry.content().unwrap().value(), Some("# Markdown Body"));
    }

    #[test]
    fn post_to_entry_org_format_becomes_text_content() {
        let post = make_post(MakePost {
            post_id: 42,
            title: Some("Title"),
            slug: "slug",
            body: "* Org Body",
            format: PostFormat::Org,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.content().unwrap().content_type(), Some("text/org"));
        assert_eq!(entry.content().unwrap().value(), Some("* Org Body"));
    }

    #[test]
    fn post_to_entry_html_format_becomes_html_content() {
        let post = make_post(MakePost {
            post_id: 42,
            title: Some("Title"),
            slug: "slug",
            body: "<p>HTML</p>",
            format: PostFormat::Html,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.content().unwrap().content_type(), Some("html"));
        assert_eq!(entry.content().unwrap().value(), Some("<p>HTML</p>"));
    }

    #[test]
    fn post_to_entry_id_is_edit_uri() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.id, "https://example.com/atompub/alice/posts/7");
    }

    #[test]
    fn post_to_entry_edit_link() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

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
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        let alternate_links: Vec<_> = entry
            .links()
            .iter()
            .filter(|l| l.rel() == "alternate")
            .collect();
        assert_eq!(alternate_links.len(), 1);
        // Permalink is date-based, so we check it contains the base URL and starts with /~
        assert!(alternate_links[0]
            .href()
            .starts_with("https://example.com/~alice"));
    }

    #[test]
    fn post_to_entry_draft_post_has_no_alternate_link() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: None, // No published_at = draft
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        let alternate_links: Vec<_> = entry
            .links()
            .iter()
            .filter(|l| l.rel() == "alternate")
            .collect();
        assert_eq!(alternate_links.len(), 0);
    }

    #[test]
    fn post_to_entry_title_from_post() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("My Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.title().as_str(), "My Title");
    }

    #[test]
    fn post_to_entry_title_falls_back_to_slug() {
        let post = make_post(MakePost {
            post_id: 7,
            title: None, // No title
            slug: "my-slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.title().as_str(), "my-slug");
    }

    #[test]
    fn post_to_entry_summary() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: Some("This is a summary"),
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.summary().unwrap().as_str(), "This is a summary");
    }

    #[test]
    fn post_to_entry_no_summary() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.summary(), None);
    }

    #[test]
    fn post_to_entry_categories_from_tags() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![
                ("rust".to_string(), "Rust".to_string()),
                ("programming".to_string(), "Programming".to_string()),
            ],
        });

        let entry = post_to_entry(&post, "https://example.com");

        let terms: Vec<_> = entry.categories().iter().map(Category::term).collect();
        assert_eq!(terms, vec!["Rust", "Programming"]);
    }

    #[test]
    fn post_to_entry_no_tags() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(entry.categories().len(), 0);
    }

    #[test]
    fn post_to_entry_published_post_not_marked_draft() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(Utc::now()),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert!(!is_draft(&entry));
    }

    #[test]
    fn post_to_entry_draft_post_marked_draft() {
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: None, // No published_at = draft
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert!(is_draft(&entry));
    }

    #[test]
    fn post_to_entry_timestamps() {
        let now = Utc::now();
        let post = make_post(MakePost {
            post_id: 7,
            title: Some("Title"),
            slug: "slug",
            body: "body",
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            tags: vec![],
        });

        let entry = post_to_entry(&post, "https://example.com");

        assert_eq!(
            entry.published().map(DateTime::timestamp),
            Some(now.timestamp())
        );
        assert_eq!(entry.updated().timestamp(), now.timestamp());
    }
}
