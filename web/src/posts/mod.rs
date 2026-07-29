//! The **posts** vertical (ADR-0070, amended #530).
//!
//! This module is **wiring only**: module declarations and re-exports, no items
//! of its own. The single-post lifecycle `#[server]` endpoints and wire types
//! live in [`api`] (with the timeline/listing surface in its `listing`
//! submodule); host-only marshalling for the `#[server]` bodies lives in the
//! `server` leaf. Re-exports keep the stable `crate::posts::…` paths external
//! call sites and the server-fn registrar depend on.

mod api;

#[cfg(feature = "server")]
mod server;

// The `#[component]` UI and browser-bound code — wasm-only by its `mod`
// declaration (ADR-0070), so the file carries no cfg gates of its own.
#[cfg(target_arch = "wasm32")]
mod component;

// The pure post-render twins (host-compiled leaf, ADR-0070): plain-string HTML
// builders shared by the projector (`crate::app::render`) and the reactive
// `PostDisplay`, reachable crate-wide as `crate::posts::render::…`.
pub(crate) mod render;

// Pure, host-tested parsing/formatting logic (ADR-0070 §6, ADR-0055): the
// permalink route-param decoder and the draft-row display computation, extracted
// out of the wasm-only components so they stay host-compiled and coverage-measured.
mod parse;

// Re-exported at the (public) `crate::posts::…` path so the pure `parse` fns are
// reachable exported items on the host build too — consumed only by the wasm-only
// `component`, an unexported `parse` fn would fail the host build as `dead_code`.
pub use parse::{draft_row_display, parse_permalink_route, DraftRowDisplay, PermalinkRoute};

// The API surface — re-exported so external call sites and the server-fn
// registrar keep the stable `crate::posts::…` paths despite living in `api.rs`.
pub use api::{
    create, delete, get, get_audience_selection, get_default_audience_selection, get_preview,
    list_by_tag, list_by_user, list_by_user_and_tag, list_drafts, list_home_feed,
    list_local_timeline, publish, unpublish, update, Create, CreateArgs, CreateResult, Delete,
    DraftSummary, Get, GetAudienceSelection, GetDefaultAudienceSelection, GetPreview, ListByTag,
    ListByUser, ListByUserAndTag, ListDrafts, ListHomeFeed, ListLocalTimeline, Publish,
    PublishResult, Unpublish, Update, UpdateArgs, UpdateResult,
};

// Server-only shared fetch helpers, consumed by the `server` crate's public
// projector (one query, no drift). Re-exported from `api` (their home is its
// `listing` submodule).
#[cfg(feature = "server")]
pub use api::{
    fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
};

// Re-exported for the `server` crate's public projector, which maps the fetched
// record the same way this vertical does (one projection, no drift). `post_response`
// is a wire-type builder that stays in `web`; the projector imports the effectful
// `fetch_post_record` straight from `storage`.
#[cfg(feature = "server")]
pub use server::post_response;

// The wasm-only reactive UI (ADR-0070): the post widgets and the routed page
// components (moved from `pages/`, #323). Re-exported so the `pages/` router keeps
// the stable `crate::posts::…` paths; the private helpers (`marker_matches`,
// `audience_checkbox`, `permalink_first_paint`, the `render_draft_row` builder) and the
// private `DraftList` subcomponent stay unexported.
#[cfg(target_arch = "wasm32")]
pub use component::{
    AudiencePicker, ComposerFields, CreatePostPage, DraftsPage, EditPostPage, InlineComposer,
    PostCard, PostCreateForm, PostDisplay, PostPage, SiteTagPage, UserTagPage, UserTimelinePage,
};
