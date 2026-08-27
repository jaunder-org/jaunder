//! The projector↔client seed contract (#610, ADR-0041): `PageSeed` — the initial data
//! a public page renders from — and the public-surface wire DTOs it embeds. The server
//! projector serializes `PageSeed` into the `#jaunder-seed` DOM blob; the `csr` client
//! deserializes it on boot for a byte-identical first paint. These are also the return
//! types of the media/post/tag `#[server]` fns. Pure `Serialize`/`Deserialize` data —
//! every field is a `common` type, so this module has no `leptos`/`web_sys`/`storage` coupling.

use serde::{Deserialize, Serialize};

use crate::ids::PostId;
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::{PostFormat, RenderedHtml, deserialize_rendered_html};
use crate::root_relative_url::RootRelativeUrl;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::time::UtcInstant;
use crate::username::Username;

/// A tag row returned by the `list_tags` server fn.
///
/// `slug` is the canonical lowercase form used in URLs (`/tags/:slug`).
/// `display` is the case-preserving form the author most recently used; the
/// autocomplete dropdown should render this to the user. When a tag has been
/// applied with multiple casings across posts, `display` reflects whichever
/// row the underlying `SELECT` returned first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagSummary {
    pub slug: Tag,
    pub display: TagLabel,
}

/// A post in rendered form: everything needed to paint it, without its source.
/// Timeline listing endpoints return these, and `PostPage` also feeds one to
/// `PostCard` for a draft permalink — so a `RenderedPost` is not necessarily
/// published.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderedPost {
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub summary: Option<PostSummary>,
    pub slug: Slug,
    #[serde(deserialize_with = "deserialize_rendered_html")]
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

/// A cursor-paginated page of rows.
///
/// The envelope is shared by listing endpoints, while each endpoint retains its
/// own row type and the specialized logic that derives its cursor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page<Row> {
    pub posts: Vec<Row>,
    /// Where the next page starts; `None` on the last page.
    pub next_cursor: Option<PageCursor>,
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

/// The initial data a public page is rendered from — serialized into the
/// projector's `#jaunder-seed` blob and adopted by the CSR client on boot.
///
/// Variants carry the route context (`username` / `tag`) the bare
/// [`Page`] lacks but the heading, title, and permalinks need — the reactive
/// components get it from the route params today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSeed {
    SiteTimeline(Page<RenderedPost>),
    Profile {
        username: Username,
        page: Page<RenderedPost>,
    },
    SiteTag {
        tag: Tag,
        page: Page<RenderedPost>,
    },
    UserTag {
        username: Username,
        tag: Tag,
        page: Page<RenderedPost>,
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
            title: None,
            summary: None,
            slug: "hello".parse().unwrap(),
            rendered_html: RenderedHtml::from_trusted("<p>hi</p>"),
            created_at: instant(),
            published_at,
            permalink: None,
            is_author: false,
            tags: Vec::new(),
        }
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

    fn page(next_cursor: Option<PageCursor>) -> Page<RenderedPost> {
        Page {
            posts: Vec::new(),
            has_more: next_cursor.is_some(),
            next_cursor,
        }
    }

    /// The cursor is wire data (#569): both shapes the server emits (a last page,
    /// and a page with more behind it) must survive the projector's seed blob.
    #[test]
    fn timeline_page_round_trips_with_and_without_a_cursor() {
        for original in [
            page(None),
            page(Some(PageCursor {
                created_at: instant(),
                post_id: PostId::from(7),
            })),
        ] {
            let json = serde_json::to_string(&original).unwrap();
            let back: Page<RenderedPost> = serde_json::from_str(&json).unwrap();
            assert_eq!(back, original);
        }
    }
    /// The generic envelope must retain this pre-cutover JSON byte sequence for
    /// rendered timeline rows, including the declaration order of its fields.
    #[test]
    fn rendered_page_serializes_to_the_pre_cutover_bytes() {
        let page = Page {
            posts: vec![rendered_post(Some(instant()))],
            next_cursor: Some(PageCursor {
                created_at: instant(),
                post_id: PostId::from(7),
            }),
            has_more: true,
        };

        assert_eq!(
            serde_json::to_string(&page).unwrap(),
            r#"{"posts":[{"post_id":1,"username":"alice","title":null,"summary":null,"slug":"hello","rendered_html":"<p>hi</p>","created_at":"2026-07-19T10:30:00Z","published_at":"2026-07-19T10:30:00Z","permalink":null,"is_author":false,"tags":[]}],"next_cursor":{"created_at":"2026-07-19T10:30:00Z","post_id":7},"has_more":true}"#
        );
    }
}
