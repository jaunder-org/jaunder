//! The projector↔client seed contract (#610, ADR-0041): `PageSeed` — the initial data
//! a public page renders from — and the public-surface wire DTOs it embeds. The server
//! projector serializes `PageSeed` into the `#jaunder-seed` DOM blob; the `csr` client
//! deserializes it on boot for a byte-identical first paint. These are also the return
//! types of the media/post/tag `#[server]` fns. Pure `Serialize`/`Deserialize` data —
//! every field is a `common` type, so this module has no `leptos`/`web_sys`/`storage` coupling.

use crate::display_name::DisplayName;
use serde::{Deserialize, Serialize};

use crate::ids::PostId;
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::{self, PostFormat, RenderedHtml};
use crate::root_relative_url::RootRelativeUrl;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::theme::PublishedThemePresentation;
use crate::time::UtcInstant;
use crate::username::Username;

/// A tag row returned by the `list_tags` server fn.
///
/// `slug` is the canonical lowercase form used in URLs (`/tags/:slug`).
/// `display` is either the author-cased label from a tagging row or the
/// canonical-slug fallback from a catalog row without a separate label. The
/// autocomplete dropdown should render this to the user. When a tag has been
/// applied with multiple casings across posts, `display` reflects whichever row
/// the underlying `SELECT` returned first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagSummary {
    pub slug: Tag,
    pub display: TagLabel,
}

impl From<TagLabel> for TagSummary {
    fn from(display: TagLabel) -> Self {
        let slug = display.slug();
        Self { slug, display }
    }
}

/// A post in rendered form: everything needed to paint it, without its source.
/// Timeline listing endpoints return these, and `PostPage` also feeds one to
/// `PostCard` for a draft permalink — so a `RenderedPost` is not necessarily
/// published.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderedPost {
    pub post_id: PostId,
    pub username: Username,
    pub display_name: Option<DisplayName>,
    pub title: Option<PostTitle>,
    pub summary: Option<PostSummary>,
    pub slug: Slug,
    #[serde(deserialize_with = "render::deserialize_rendered_html")]
    // rendered-html-from-trusted:allow seed DTO rebuilds HTML serialized by Jaunder's own server (#701)
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    /// `None` for an unpublished draft.
    pub published_at: Option<UtcInstant>,
    /// Root-relative permalink of a published post; `None` for a draft (which has
    /// no public permalink), so the title renders without a link — coinciding
    /// with the projector's draft paint.
    pub permalink: Option<RootRelativeUrl>,
    /// True when the viewing user is the post author.
    pub is_author: bool,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
}

impl RenderedPost {
    /// The instant a reader should see: publication time, or creation time for a
    /// draft that has none. One definition so the projector's markup and the CSR
    /// client's cannot drift apart (ADR-0041 D2, ADR-0044).
    #[must_use]
    pub fn display_time(&self) -> UtcInstant {
        self.published_at.unwrap_or(self.created_at)
    }

    /// Whether the post has not been published. Publication time is the sole
    /// lifecycle signal on this wire DTO, so scheduled posts are not drafts.
    #[must_use]
    pub fn is_draft(&self) -> bool {
        self.published_at.is_none()
    }
}

/// The `(created_at, post_id)` keyset pair a paginated listing hands back.
///
/// One field rather than two flat `Option`s on the page: the components always
/// move together, so bundling them makes a half-cursor — which no listing ever
/// emits — unrepresentable on the wire as well as in the client's state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageCursor {
    pub created_at: UtcInstant,
    pub post_id: PostId,
}

/// Viewer-selected chronology for a web Post timeline.
///
/// This is deliberately a closed wire enum: a continuation cursor is meaningful
/// only with the direction that produced it.
#[macros::text_enum(
    error = InvalidTimelineOrder,
    message = "timeline order must be \"newest\" or \"oldest\""
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum TimelineOrder {
    #[default]
    Newest,
    Oldest,
}

/// The `(published_at, post_id, order)` keyset pair a web Post timeline hands back.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelineCursor {
    pub published_at: UtcInstant,
    pub post_id: PostId,
    pub order: TimelineOrder,
}

/// Cohesive wire input for one web Post timeline page.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePageRequest {
    pub order: TimelineOrder,
    pub cursor: Option<TimelineCursor>,
    pub limit: Option<crate::pagination::PageSize>,
}

/// A cursor-paginated page of rows.
///
/// The envelope is shared by listing endpoints. `Cursor` defaults to
/// [`PageCursor`] so every excluded listing preserves its existing wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page<Row, Cursor = PageCursor> {
    pub posts: Vec<Row>,
    /// Where the next page starts; `None` on the last page.
    pub next_cursor: Option<Cursor>,
    pub has_more: bool,
}

/// A post with the source it was authored from: everything needed to paint it,
/// plus the `body`/`format` only the authoring surfaces (editor, preview) read.
/// Returned by the `get_post`/`get_post_preview` server fns and carried by
/// [`PageSeed::Permalink`], which serves drafts as well as published posts.
///
/// A shared core plus an extension rather than a union with [`RenderedPost`]:
/// merging the two would ship a `PostBody` on every timeline row that never
/// reads one. See `docs/adr/0097-post-dto-content-weight-axis.md` (rule 2).
///
/// Nested rather than `#[serde(flatten)]`: flatten buffers the whole map through
/// serde's `Content` and re-drives any `deserialize_with` — and `post` carries
/// one on `rendered_html`. The seed travels server→client within a single
/// deploy, so an extra `"post"` level on the wire costs nothing worth that.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthoredPost {
    pub post: RenderedPost,
    pub body: PostBody,
    pub format: PostFormat,
}

/// Server-resolved presentation for a public route.
///
/// The projector seed and every public navigation response use this one
/// envelope. The theme is deliberately resolved by the server from route
/// ownership, never from the viewer or browser-local state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicPresentation<Page> {
    pub theme: PublishedThemePresentation,
    pub page: Page,
}

/// The initial data a public page is rendered from — serialized into the
/// projector's `#jaunder-seed` blob and adopted by the CSR client on boot.
///
/// Variants carry the route context (`username` / `tag`) the bare
/// [`Page`] lacks but the heading, title, and permalinks need — the reactive
/// components get it from the route params today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSeed {
    SiteTimeline {
        order: TimelineOrder,
        page: Page<RenderedPost, TimelineCursor>,
    },
    Profile {
        username: Username,
        order: TimelineOrder,
        page: Page<RenderedPost, TimelineCursor>,
    },
    SiteTag {
        tag: Tag,
        order: TimelineOrder,
        page: Page<RenderedPost, TimelineCursor>,
    },
    UserTag {
        username: Username,
        tag: Tag,
        order: TimelineOrder,
        page: Page<RenderedPost, TimelineCursor>,
    },
    Permalink(AuthoredPost),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instant() -> UtcInstant {
        "2026-07-19T10:30:00Z".parse().unwrap()
    }

    fn rendered_post(published_at: Option<UtcInstant>) -> RenderedPost {
        RenderedPost {
            post_id: PostId::from(1),
            username: "alice".parse().unwrap(),
            display_name: None,
            title: None,
            summary: None,
            slug: "hello".parse().unwrap(),
            rendered_html: crate::test_support::rendered_html("<p>hi</p>"),
            created_at: instant(),
            published_at,
            permalink: None,
            is_author: false,
            tags: Vec::new(),
        }
    }

    #[test]
    fn tag_summary_from_label_preserves_display_and_derives_canonical_slug() {
        let summary = TagSummary::from("Rust".parse::<TagLabel>().unwrap());

        assert_eq!(summary.slug, "rust");
        assert_eq!(summary.display, "Rust");
    }

    #[test]
    fn rendered_post_derives_draft_state_and_omits_it_from_the_wire() {
        let draft = rendered_post(None);
        let published = rendered_post(Some("2026-07-19T10:30:00Z".parse().unwrap()));
        let scheduled = rendered_post(Some("2026-07-20T10:30:00Z".parse().unwrap()));

        assert!(draft.is_draft());
        assert!(!published.is_draft());
        assert!(!scheduled.is_draft());

        let json = serde_json::to_value(draft).unwrap();
        assert!(json.get("is_draft").is_none(), "wire shape: {json}");

        let deserialized: RenderedPost = serde_json::from_value(json).unwrap();
        assert!(deserialized.is_draft());
    }

    fn timeline_page(next_cursor: Option<TimelineCursor>) -> Page<RenderedPost, TimelineCursor> {
        Page {
            posts: Vec::new(),
            has_more: next_cursor.is_some(),
            next_cursor,
        }
    }

    /// Timeline cursors retain both the publication key and their direction across
    /// the projector seed boundary.
    #[test]
    fn timeline_page_round_trips_with_and_without_a_cursor() {
        for original in [
            timeline_page(None),
            timeline_page(Some(TimelineCursor {
                published_at: instant(),
                post_id: PostId::from(7),
                order: TimelineOrder::Oldest,
            })),
        ] {
            let json = serde_json::to_string(&original).unwrap();
            let back: Page<RenderedPost, TimelineCursor> = serde_json::from_str(&json).unwrap();
            assert_eq!(back, original);
        }
    }
    /// The generic envelope must retain this pre-cutover JSON byte sequence for
    /// an empty rendered timeline page, including the declaration order of its
    /// fields.
    #[test]
    fn rendered_page_serializes_to_the_pre_cutover_bytes() {
        let page: Page<RenderedPost> = Page {
            posts: Vec::new(),
            next_cursor: None,
            has_more: false,
        };

        assert_eq!(
            serde_json::to_string(&page).unwrap(),
            r#"{"posts":[],"next_cursor":null,"has_more":false}"#
        );
    }

    #[test]
    fn public_presentation_serializes_the_server_resolved_theme_with_the_page() {
        let presentation = PublicPresentation {
            theme: crate::theme::PublishedThemePresentation::built_in(crate::theme::Theme::Reader),
            page: PageSeed::SiteTimeline {
                order: TimelineOrder::Newest,
                page: Page {
                    posts: vec![],
                    next_cursor: None,
                    has_more: false,
                },
            },
        };

        assert_eq!(
            serde_json::to_string(&presentation).unwrap(),
            r#"{"theme":{"identity":{"kind":"built_in","value":"reader"},"revision":null,"stylesheet_url":"/style/jaunder-themes.css","logo_url":null,"header_url":null},"page":{"SiteTimeline":{"order":"newest","page":{"posts":[],"next_cursor":null,"has_more":false}}}}"#
        );
    }
}
