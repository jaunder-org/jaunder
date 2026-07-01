# Public Projector + leptos-CSR Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the public discoverability routes (root timeline, profile, permalink, site-tag, user-tag) as non-reactive server-side HTML + an embedded data blob + the CSR boot script, and boot the leptos-CSR client from that blob — removing the reactive runtime from the public request path (issues #178 + #179).

**Architecture:** Behind the existing `csr` cargo feature, a plain axum **projector** handler per public route fetches data via a shared plain-async fn, calls a **pure, reactivity-free render fn** in `web/src/render/`, and emits a full HTML document (`<head>` SEO + `<div id="app">` content + `#jaunder-seed` JSON blob + `/pkg/jaunder.js` boot). The CSR client reads the blob on boot, seeds first paint via the *same* render fn (coincidence by construction), and falls back to the existing `#[server]` fns for client-side navigation. The `not(csr)` default SSR build is untouched.

**Tech Stack:** Rust, leptos 0.8.x (CSR feature), axum, `leptos_axum` (server-fn handler only under csr), serde_json, wasm-bindgen, Playwright (e2e), nix (`csr-e2e-postgres-chromium` check).

## Global Constraints

- **All new behaviour lives behind `#[cfg(feature = "csr")]` (server) / the csr build (client).** The `not(csr)` SSR path in `server/src/lib.rs` and every existing SSR/hydrate component stay working unchanged. Copy verbatim: the projector router registration goes **inside the existing `#[cfg(feature = "csr")]` arm** of `create_router` (`server/src/lib.rs:135-141`), before the `ServeDir` fallback.
- **`web/src/render/` must use NO leptos reactivity** — no `signal`, `Resource`, `Suspense`, `ServerAction`, `view!`. Plain string building only (like `common/src/feed/*`). It must compile under all of `web`'s features (`ssr`, `hydrate`, `csr`) and on both `wasm32` and host targets.
- **Anonymous + cacheable:** the projector always renders the anonymous view — no per-request auth branch, clock, or nonce. Byte output for a given URL is identical for every visitor. `is_author` is always `false` in projector-rendered content.
- **Version lock:** do NOT bump `leptos`, `leptos_axum`, `wasm-bindgen`, or `wasm-bindgen-cli`. leptos 0.8.20 regresses rendering (issue #178 note); stay on the pinned versions the #177 CSR infra established.
- **Backend parity:** any test that touches storage runs on **both sqlite and postgres** using the dual-backend test template; the `test-backend-pattern` xtask guard will fail otherwise (CONTRIBUTING.md).
- **Per-commit gate:** run `cargo xtask check` (green) before each commit; the e2e slices additionally verify with the `csr-e2e-postgres-chromium` nix check. `end2end/node_modules` must be provisioned in the worktree (run the devShell shellHook once) or `tsc` fails.
- **No Co-Authored-By trailers** in commits. Every commit message references **#178 and #179**.

## File Structure

**Create:**
- `web/src/render/mod.rs` — the pure render module: `PageSeed` enum, `render_head(&PageSeed) -> String`, `render_body(&PageSeed) -> String`, and private per-kind helpers + HTML-escape util. Owns *all* public-page markup.
- `server/src/projector/mod.rs` — the axum projector: one handler per public route, the document-assembly helper, and `register(router) -> Router` called from `create_router`'s csr arm.
- `docs/adr/0041-public-projector-and-csr-client.md` — ADR recording the projector architecture + shared-pure-fn placement + blob-seed contract; amends ADR-0002.

**Modify:**
- `web/src/lib.rs` — add `pub mod render;`.
- `web/src/posts/listing.rs` — extract `fetch_user_posts`, `fetch_local_timeline`, `fetch_posts_by_tag` (ssr-gated plain-async fns taking an explicit `viewer`); rewrite the three `#[server]` fns as thin wrappers.
- `web/src/posts/mod.rs` — extract `fetch_post` (public-permalink lookup for an explicit `viewer`); rewrite `get_post` as a wrapper that adds the auth/draft-fallback + `is_author`.
- `web/src/pages/posts.rs` — `PostPage`, `UserTimelinePage`, `SiteTagPage`, `UserTagPage`: read the `PageSeed` context for first paint instead of the initial reactive fetch.
- `web/src/pages/mod.rs` — `HomePage` (root timeline): same seed-read change; provide the `PageSeed` context near the top of `App`.
- `csr/src/lib.rs` — client seed harness: read `#jaunder-seed`, deserialize `PageSeed`, `provide_context(Option<PageSeed>)`, and mount into `#app`.
- `server/src/lib.rs` — call `crate::projector::register(app)` inside the `#[cfg(feature = "csr")]` arm; add `#[cfg(feature = "csr")] mod projector;` (or unconditional `mod projector;` gated internally).
- `docs/README.md` — add the ADR-0041 row to the ADR table.

**Test:**
- Unit tests inline in `web/src/render/mod.rs` (`#[cfg(test)]`).
- `server/tests/` — projector integration tests (byte-identical, cacheable, anonymous) using the dual-backend template.
- `end2end/tests/projector.spec.ts` — crawlable (JS-off) + flash-free coincidence e2e.

---

## Task 1 (Commit 0): Foundation — pure render module + `PageSeed` + ADR

**Files:**
- Create: `web/src/render/mod.rs`
- Modify: `web/src/lib.rs` (add `pub mod render;`)
- Create: `docs/adr/0041-public-projector-and-csr-client.md`
- Modify: `docs/README.md` (ADR table row)

**Interfaces:**
- Consumes: `web::posts::listing::{TimelinePage, TimelinePostSummary}`, `web::posts::PostResponse`, `web::tags::TagSummary` (all `Serialize + Deserialize + Clone + PartialEq`, verbatim in the spec).
- Produces:
  - `pub enum PageSeed { SiteTimeline(TimelinePage), Profile { username: String, page: TimelinePage }, SiteTag { tag: String, page: TimelinePage }, UserTag { username: String, tag: String, page: TimelinePage }, Permalink(PostResponse) }` — `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`.
  - `pub fn render_head(seed: &PageSeed) -> String`
  - `pub fn render_body(seed: &PageSeed) -> String`

Rationale for the variant fields: `render_head`/`render_body` need the `username`/`tag` context (for `<title>`, permalinks, headings) that the bare `TimelinePage` doesn't carry — the timeline components get it today from the route params, so the seed must carry it.

- [ ] **Step 1: Write failing unit tests for the HTML-escape helper and `render_body` (permalink).**

Add to `web/src/render/mod.rs` (create the file with the test module first):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::PostResponse;
    use crate::tags::TagSummary;

    fn sample_post() -> PostResponse {
        PostResponse {
            post_id: 7,
            username: "alice".into(),
            title: Some("Hello & <World>".into()),
            slug: "hello".into(),
            body: "raw".into(),
            format: "markdown".into(),
            rendered_html: "<p>Hi <em>there</em></p>".into(),
            created_at: "2026-01-02T03:04:05Z".into(),
            published_at: Some("2026-01-02T03:04:05Z".into()),
            is_draft: false,
            is_author: false,
            permalink: Some("/~alice/2026/01/02/hello".into()),
            tags: vec![TagSummary { slug: "rust".into(), display: "Rust".into() }],
            summary: None,
        }
    }

    #[test]
    fn escape_replaces_markup_metacharacters() {
        assert_eq!(escape_html("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn permalink_body_escapes_title_but_injects_rendered_html_raw() {
        let html = render_body(&PageSeed::Permalink(sample_post()));
        // Title text is escaped:
        assert!(html.contains("Hello &amp; &lt;World&gt;"), "title must be escaped: {html}");
        // Pre-rendered post HTML is injected verbatim (it is already sanitized upstream):
        assert!(html.contains("<p>Hi <em>there</em></p>"), "rendered_html must be raw: {html}");
        // Semantic structure + permalink present:
        assert!(html.contains("<article"), "expected <article>: {html}");
        assert!(html.contains("/~alice/2026/01/02/hello"), "expected permalink: {html}");
        // Tag chip present:
        assert!(html.contains("Rust"), "expected tag display: {html}");
    }

    #[test]
    fn permalink_head_sets_title_and_meta() {
        let head = render_head(&PageSeed::Permalink(sample_post()));
        assert!(head.contains("<title>"), "expected <title>: {head}");
        assert!(head.contains("Hello &amp; &lt;World&gt;"), "title escaped in head: {head}");
        assert!(head.contains(r#"<meta property="og:title""#), "expected OG title: {head}");
    }

    #[test]
    fn page_seed_round_trips_through_json() {
        let seed = PageSeed::Permalink(sample_post());
        let json = serde_json::to_string(&seed).unwrap();
        let back: PageSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail (compile error / not defined).**

Run: `cargo test -p web --features ssr render::tests`
Expected: FAIL — `escape_html`, `render_body`, `render_head`, `PageSeed` not defined.

- [ ] **Step 3: Implement the render module.**

Write `web/src/render/mod.rs` above the test module. Concrete implementation:

```rust
//! Pure, non-reactive HTML rendering for the public discoverability surface.
//!
//! Shared by the server-side projector (`server::projector`) and the CSR client
//! (`web::pages`): both call the SAME fn on the SAME data, so the projector's
//! painted content and the client's first paint coincide byte-for-byte
//! (flash-free). NO leptos reactivity here — plain string building only, like
//! `common::feed`. See docs/adr/0041 and docs/inbound-data-handling.md §4.

use crate::posts::listing::{TimelinePage, TimelinePostSummary};
use crate::posts::PostResponse;
use serde::{Deserialize, Serialize};

/// The initial data a public page is rendered from — serialized into the
/// projector's `#jaunder-seed` blob and adopted by the CSR client on boot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageSeed {
    SiteTimeline(TimelinePage),
    Profile { username: String, page: TimelinePage },
    SiteTag { tag: String, page: TimelinePage },
    UserTag { username: String, tag: String, page: TimelinePage },
    Permalink(PostResponse),
}

/// Escape text for safe interpolation into HTML element/attribute content.
fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The document `<head>` inner HTML: per-page title + description + Open Graph.
#[must_use]
pub fn render_head(seed: &PageSeed) -> String {
    let (title, description) = match seed {
        PageSeed::Permalink(p) => (
            p.title.clone().unwrap_or_else(|| format!("Post by {}", p.username)),
            p.summary.clone().unwrap_or_default(),
        ),
        PageSeed::Profile { username, .. } => (format!("Posts by {username}"), String::new()),
        PageSeed::SiteTimeline(_) => ("Jaunder".to_string(), String::new()),
        PageSeed::SiteTag { tag, .. } => (format!("#{tag}"), String::new()),
        PageSeed::UserTag { username, tag, .. } => (format!("#{tag} by {username}"), String::new()),
    };
    let t = escape_html(&title);
    let d = escape_html(&description);
    format!(
        concat!(
            r#"<meta charset="utf-8" />"#,
            r#"<meta name="viewport" content="width=device-width, initial-scale=1" />"#,
            r#"<link rel="stylesheet" href="/style/jaunder.css" />"#,
            r#"<link rel="stylesheet" href="/style/jaunder-themes.css" />"#,
            "<title>{t}</title>",
            r#"<meta name="description" content="{d}" />"#,
            r#"<meta property="og:title" content="{t}" />"#,
            r#"<meta property="og:description" content="{d}" />"#,
        ),
        t = t,
        d = d,
    )
}

/// The `<div id="app">` inner HTML: the semantic, crawlable page content.
#[must_use]
pub fn render_body(seed: &PageSeed) -> String {
    match seed {
        PageSeed::Permalink(post) => render_article(post),
        PageSeed::SiteTimeline(page) => render_timeline("Jaunder", &page.posts),
        PageSeed::Profile { username, page } => {
            render_timeline(&format!("Posts by {username}"), &page.posts)
        }
        PageSeed::SiteTag { tag, page } => render_timeline(&format!("#{tag}"), &page.posts),
        PageSeed::UserTag { username, tag, page } => {
            render_timeline(&format!("#{tag} by {username}"), &page.posts)
        }
    }
}

/// One post as a permalink page.
fn render_article(post: &PostResponse) -> String {
    let title = post.title.as_deref().map_or_else(String::new, |t| {
        format!("<h1 class=\"j-post-title\">{}</h1>", escape_html(t))
    });
    let permalink = post.permalink.as_deref().unwrap_or_default();
    // `rendered_html` is already-sanitized HTML produced upstream — inject raw.
    format!(
        r#"<article class="j-post"><a class="j-post-plink" href="{plink}">{time}</a>{title}<div class="j-post-body">{body}</div>{tags}</article>"#,
        plink = escape_html(permalink),
        time = escape_html(&post.published_at.clone().unwrap_or_else(|| post.created_at.clone())),
        title = title,
        body = post.rendered_html,
        tags = render_tags(&post.tags),
    )
}

/// A list of post summaries as a timeline page.
fn render_timeline(heading: &str, posts: &[TimelinePostSummary]) -> String {
    let mut out = format!("<h1 class=\"j-timeline-title\">{}</h1>", escape_html(heading));
    if posts.is_empty() {
        out.push_str("<p>No posts yet.</p>");
        return out;
    }
    out.push_str("<div class=\"j-timeline\">");
    for post in posts {
        let title = post.title.as_deref().map_or_else(String::new, |t| {
            format!("<h2 class=\"j-post-title\">{}</h2>", escape_html(t))
        });
        out.push_str(&format!(
            r#"<article class="j-post"><a class="j-post-plink" href="{plink}">{time}</a>{title}<div class="j-post-body">{body}</div>{tags}</article>"#,
            plink = escape_html(&post.permalink),
            time = escape_html(&post.published_at),
            title = title,
            body = post.rendered_html,
            tags = render_tags(&post.tags),
        ));
    }
    out.push_str("</div>");
    out
}

fn render_tags(tags: &[crate::tags::TagSummary]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"j-post-tags\">");
    for tag in tags {
        out.push_str(&format!(
            r#"<li><a href="/tags/{slug}">{display}</a></li>"#,
            slug = escape_html(&tag.slug),
            display = escape_html(&tag.display),
        ));
    }
    out.push_str("</ul>");
    out
}
```

Then add to `web/src/lib.rs` (near the other `pub mod` lines): `pub mod render;`.

Note for the executor: confirm the exact import path of `TagSummary` (spec says `web/src/tags/mod.rs`) and `PostResponse` (`web/src/posts/mod.rs`); adjust `use` paths if the module tree differs. Match the CSS class names to those `PostDisplay`/`PostCard` already emit (inspect `web/src/pages/ui.rs`) so the client render coincides with existing styling — the class names above are the intent; align them with the real component output.

- [ ] **Step 4: Run tests to verify they pass.**

Run: `cargo test -p web --features ssr render::tests`
Expected: PASS (4 tests).

- [ ] **Step 5: Write ADR-0041 and update the ADR table.**

Create `docs/adr/0041-public-projector-and-csr-client.md` (status `accepted`, dated 2026-07-01) recording: the "SSR the data, not the components" decision; the pure render fn lives in `web/src/render/` (not `common`/`jaunder-core`, deferred); the `PageSeed` blob-seed contract; the projector runs behind the `csr` feature; and that this **amends ADR-0002** (Frontend Framework) for the web surface (web client = leptos-CSR, public surface = non-reactive projector). Add a row to the ADR table in `docs/README.md`: `| 0041 | Public projector and CSR client | accepted |` (match the table's exact column format).

- [ ] **Step 6: Gate + commit.**

Run: `cargo xtask check`
Expected: PASS (`xtask check PASSED`).

```bash
git add web/src/render/mod.rs web/src/lib.rs docs/adr/0041-public-projector-and-csr-client.md docs/README.md
git commit -m "feat(web): pure non-reactive render module + PageSeed contract (#178, #179)"
```

---

## Task 2 (Commit 1): Projector infra + client seed harness + permalink vertical

This is the fat slice: it stands up the server projector, the router seam, and the client seed harness, then wires the **permalink** route end-to-end to prove the whole pipeline coincides.

**Files:**
- Modify: `web/src/posts/mod.rs` (extract `fetch_post`)
- Create: `server/src/projector/mod.rs`
- Modify: `server/src/lib.rs` (register projector in csr arm)
- Modify: `csr/src/lib.rs` (seed harness + mount to `#app`)
- Modify: `web/src/pages/posts.rs` (`PostPage` reads seed)
- Create/modify tests: `server/tests/projector.rs`, `end2end/tests/projector.spec.ts`

**Interfaces:**
- Consumes: `web::render::{PageSeed, render_head, render_body}` (Task 1); `web::posts::PostResponse`; the storage trait `Arc<dyn PostStorage>` (from `AppState.posts`); the anonymous `ViewerIdentity`.
- Produces:
  - `web::posts::fetch_post(posts: &Arc<dyn PostStorage>, viewer: &ViewerIdentity, username: &str, year: i32, month: u32, day: u32, slug: &str) -> WebResult<Option<PostResponse>>` (ssr-only). Returns the **public** post for `viewer` (no draft fallback, no `is_author` — those stay in the `#[server]` wrapper).
  - `server::projector::register(router: Router<LeptosOptions>) -> Router<LeptosOptions>` — registers the public GET routes.
  - `server::projector::document(head: &str, body: &str, seed: &PageSeed) -> String` — assembles the full HTML document.

- [ ] **Step 1: Extract `fetch_post` and confirm the anonymous `ViewerIdentity` constructor.**

In `web/src/posts/mod.rs`, pull the public-permalink lookup out of `get_post` (spec lines 304-368) into an ssr-gated plain fn. It performs only the `get_post_by_permalink` visibility-filtered lookup with the *passed* `viewer` and maps to `PostResponse` via the existing `post_response(post, is_author)` helper. Leave the `require_auth`/draft-fallback/`is_author` logic in `get_post`.

```rust
#[cfg(feature = "ssr")]
pub async fn fetch_post(
    posts: &std::sync::Arc<dyn common::storage::PostStorage>,
    viewer: &common::viewer::ViewerIdentity, // confirm exact path of ViewerIdentity
    username: &str,
    year: i32,
    month: u32,
    day: u32,
    slug: &str,
    is_author: bool,
) -> WebResult<Option<PostResponse>> {
    use common::identifiers::{Slug, Username}; // confirm exact paths
    let username_parsed = username
        .parse::<Username>()
        .map_err(|e| InternalError::validation(e.to_string()))?;
    let slug_parsed = slug
        .parse::<Slug>()
        .map_err(|e| InternalError::validation(e.to_string()))?;
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| InternalError::validation("Invalid permalink"))?;
    let found = posts
        .get_post_by_permalink(&username_parsed, year, month, day, &slug_parsed, viewer, chrono::Utc::now())
        .await
        .map_err(InternalError::storage)?;
    Ok(found.map(|post| post_response(post, is_author)))
}
```

Then rewrite `get_post`'s success branch to call `fetch_post(&posts, &viewer, …, is_author)` after computing `viewer`/`is_author` as today; keep the draft-fallback branch unchanged.

**Executor must confirm** (grep, do not guess): the exact module path of `ViewerIdentity`, the anonymous constructor (e.g. `ViewerIdentity::anonymous()`), and the paths of `Slug`/`Username`/`PostStorage`/`InternalError`/`post_response`. Use the real ones. `boundary!` stays only in the `#[server]` wrapper.

- [ ] **Step 2: Write the failing projector integration test.**

Create `server/tests/projector.rs` using the dual-backend test template (mirror an existing `server/tests/*` that boots storage + a router on both backends — copy its harness). Test intent:

```rust
// (dual-backend template header per CONTRIBUTING — both sqlite & postgres)
// 1. Seed a published post for user "alice" at a known permalink.
// 2. Build the csr-feature router (create_router with the csr build).
// 3. GET the permalink twice.
// 4. Assert: status 200; body contains the post title and rendered_html;
//    body contains `id="jaunder-seed"` and `/pkg/jaunder.js`;
//    the two responses are byte-identical; an ETag header is present.
```

Write concrete request/assert code against the test harness's client. Assert `resp1.body == resp2.body` (byte-identical) and `resp1.headers().contains_key("etag")`.

- [ ] **Step 3: Run it to verify failure.**

Run: `cargo test -p server --features csr --test projector`
Expected: FAIL — `projector` module / routes don't exist.

- [ ] **Step 4: Implement the projector module.**

Create `server/src/projector/mod.rs`:

```rust
//! Non-reactive server-side HTML for the public discoverability routes (#178).
//! Runs only under `--features csr`; emits one cacheable document per URL:
//! `<head>` SEO + `<div id="app">` content + `#jaunder-seed` blob + CSR boot.
#![cfg(feature = "csr")]

use axum::{
    extract::{Extension, Path},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use leptos::config::LeptosOptions;
use std::sync::Arc;
use web::render::{render_body, render_head, PageSeed};

pub fn register(router: Router<LeptosOptions>) -> Router<LeptosOptions> {
    // Registered inside create_router's csr arm, BEFORE the ServeDir fallback,
    // so these public paths win over the static SPA shell.
    router
        .route("/", get(site_timeline))
        .route("/{username}", get(profile))
        .route("/{username}/{year}/{month}/{day}/{slug}", get(permalink))
        .route("/tags/{tag}", get(site_tag))
        .route("/{username}/tags/{tag}", get(user_tag))
}

/// Assemble the full cacheable HTML document from rendered head/body + blob.
pub fn document(head: &str, body: &str, seed: &PageSeed) -> String {
    let blob = serde_json::to_string(seed).unwrap_or_else(|_| "null".to_string());
    format!(
        concat!(
            "<!DOCTYPE html><html lang=\"en\"><head>{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>",
            "<script type=\"module\">import init from \"/pkg/jaunder.js\"; init();</script>",
            "</body></html>",
        ),
        head = head,
        body = body,
        // JSON is safe inside a non-executing application/json script as long as
        // "</script" can't appear; escape the sole breakout sequence.
        blob = blob.replace("</", "<\\/"),
    )
}

fn ok_html(seed: &PageSeed) -> Response {
    let body = document(&render_head(seed), &render_body(seed), seed);
    let etag = format!("\"{:x}\"", seahash::hash(body.as_bytes())); // confirm a hasher already in deps
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
    headers.insert(header::ETAG, etag.parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "public, max-age=0, must-revalidate".parse().unwrap());
    (headers, body).into_response()
}

async fn permalink(
    Extension(posts): Extension<Arc<dyn common::storage::PostStorage>>,
    Path((username, year, month, day, slug)): Path<(String, i32, u32, u32, String)>,
) -> Response {
    let viewer = common::viewer::ViewerIdentity::anonymous(); // confirm ctor
    match web::posts::fetch_post(&posts, &viewer, &username, year, month, day, &slug, false).await {
        Ok(Some(post)) => ok_html(&PageSeed::Permalink(post)),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// site_timeline / profile / site_tag / user_tag: stubs returning 404 for now;
// implemented in Tasks 3 and 4. Register them so routing is complete, but they
// return StatusCode::NOT_FOUND until their vertical lands.
async fn site_timeline() -> Response { StatusCode::NOT_FOUND.into_response() }
async fn profile(Path(_u): Path<String>) -> Response { StatusCode::NOT_FOUND.into_response() }
async fn site_tag(Path(_t): Path<String>) -> Response { StatusCode::NOT_FOUND.into_response() }
async fn user_tag(Path((_u, _t)): Path<(String, String)>) -> Response { StatusCode::NOT_FOUND.into_response() }
```

Wire it in `server/src/lib.rs`: add `mod projector;` (gated) and, inside the existing `#[cfg(feature = "csr")]` arm at `server/src/lib.rs:135-141`, change to register projector routes before the fallback:

```rust
#[cfg(feature = "csr")]
let app = {
    use tower_http::services::{ServeDir, ServeFile};
    let _ = (&state, &leptos_mailer, secure_cookies);
    let site_root = leptos_options.site_root.to_string();
    let index_html = format!("{site_root}/index.html");
    let app = crate::projector::register(app);
    app.fallback_service(ServeDir::new(&site_root).fallback(ServeFile::new(index_html)))
};
```

Note: the projector handlers read `Extension(posts)` — `posts_ext` is already layered (`server/src/lib.rs`). Confirm the `.layer(Extension(posts_ext))` runs for these routes (it is applied to the merged `app`, so yes). **Executor must confirm** a content hasher already in the dependency tree (the feed handlers compute ETags — reuse whatever they use; do NOT add `seahash` if the feed code uses something else).

- [ ] **Step 5: Run the integration test to verify it passes.**

Run: `cargo test -p server --features csr --test projector`
Expected: PASS (both backends).

- [ ] **Step 6: Implement the client seed harness + mount to `#app`.**

Rewrite `csr/src/lib.rs`'s `main` (keep the `mark_ready` inline_js + `recursion_limit`):

```rust
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn main() {
    use web::App;
    _ = console_log::init_with_level(log::Level::Debug);
    console_error_panic_hook::set_once();

    // Adopt the projector's data blob (if present) so the first paint seeds
    // from it instead of firing a fetch — coincident with the server render.
    let seed = read_seed();
    let mount = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("app"));
    match mount {
        Some(el) => {
            // Clear projector-painted content, then mount App into #app. App's
            // first render reproduces the same content from `seed` (same render
            // fn) → no visible flash.
            el.set_inner_html("");
            leptos::mount::mount_to(el.unchecked_into(), move || {
                leptos::context::provide_context(seed.clone());
                web::App()
            });
        }
        None => leptos::mount::mount_to_body(App),
    }
    mark_ready();
}

fn read_seed() -> Option<web::render::PageSeed> {
    let doc = web_sys::window()?.document()?;
    let el = doc.get_element_by_id("jaunder-seed")?;
    let json = el.text_content()?;
    serde_json::from_str(&json).ok()
}
```

**Executor must confirm** the exact leptos 0.8 mount API for mounting into a specific element with a closure that provides context (it may be `mount_to` taking `HtmlElement` + a `FnMut() -> impl IntoView`; adjust — the alternative is to `provide_context(seed)` at the very top of `App` by reading the blob inside `App` under `#[cfg(feature = "csr")]` instead of here). Whichever compiles cleanly: the requirement is that `Option<PageSeed>` is in context before the page components render. Add `serde_json`, `web-sys` (with `Element`/`Document` features) to `csr/Cargo.toml` if not present.

- [ ] **Step 7: `PostPage` reads the seed for first paint.**

In `web/src/pages/posts.rs`, `PostPage` (spec lines 113-212): before creating the `server_resource`, check the context for a matching seed and, if present, render directly from it via `web::render` (no `Suspense`). Pattern (apply the same shape in Tasks 3/4):

```rust
// At the top of the component body:
let seed = leptos::context::use_context::<Option<crate::render::PageSeed>>().flatten();
if let Some(crate::render::PageSeed::Permalink(post)) = seed {
    // First paint from the blob: identical bytes to the projector.
    let html = crate::render::render_body(&crate::render::PageSeed::Permalink(post));
    return view! { <div class="j-scroll"><div class="j-page" inner_html=html></div></div> }.into_any();
}
// else: existing server_resource + Suspense path (client-side navigation).
```

Keep the existing reactive path as the `else` for client-side navigation (no seed). This preserves interactivity on nav while making the seeded first paint coincide.

Note: the seed is single-use for the *initial* URL. On client-side nav the components take the fetch path — acceptable (§4: further navigation is API-driven). If returning early changes the function's return type, wrap both arms in `.into_any()`.

- [ ] **Step 8: Write the flash-free + crawlable e2e.**

Create `end2end/tests/projector.spec.ts` (mirror an existing spec's fixtures/imports). Two tests:
1. **Crawlable (JS off):** new context with `javaScriptEnabled: false`, publish a post via the API/fixtures, GET its permalink, assert the post title + body text are in the served HTML.
2. **Flash-free (JS on):** load the permalink, assert the post content is visible immediately and remains present after `body[data-hydrated]` appears (no disappearance/re-render swap).

These run under the `csr-e2e-postgres-chromium` config.

- [ ] **Step 9: Gate + e2e + commit.**

Run: `cargo xtask check` → PASS.
Run (e2e slice): the `csr-e2e-postgres-chromium` nix check → PASS (panic-free).

```bash
git add web/src/posts/mod.rs server/src/projector/mod.rs server/src/lib.rs csr/src/lib.rs csr/Cargo.toml web/src/pages/posts.rs server/tests/projector.rs end2end/tests/projector.spec.ts
git commit -m "feat(projector): permalink projector + CSR seed harness (#178, #179)"
```

---

## Task 3 (Commit 2): Profile + site-timeline verticals

**Files:**
- Modify: `web/src/posts/listing.rs` (extract `fetch_user_posts`, `fetch_local_timeline`)
- Modify: `server/src/projector/mod.rs` (implement `profile`, `site_timeline`)
- Modify: `web/src/pages/posts.rs` (`UserTimelinePage` reads seed), `web/src/pages/mod.rs` (`HomePage` reads seed; provide seed context)
- Modify: `server/tests/projector.rs`, `end2end/tests/projector.spec.ts`

**Interfaces:**
- Produces (ssr-only, `web/src/posts/listing.rs`):
  - `fetch_user_posts(posts: &Arc<dyn PostStorage>, viewer: &ViewerIdentity, username: &str, cursor_created_at: Option<String>, cursor_post_id: Option<i64>, limit: Option<u32>) -> WebResult<TimelinePage>`
  - `fetch_local_timeline(posts: &Arc<dyn PostStorage>, viewer: &ViewerIdentity, cursor_created_at: Option<String>, cursor_post_id: Option<i64>, limit: Option<u32>) -> WebResult<TimelinePage>`

- [ ] **Step 1: Failing integration tests** for `/` and `/:username` in `server/tests/projector.rs`: seed posts, GET both, assert 200 + titles present + `#jaunder-seed` + byte-identical on repeat.

- [ ] **Step 2: Run → FAIL** (handlers still 404). `cargo test -p server --features csr --test projector`.

- [ ] **Step 3: Extract the two fetch fns.** Move the body of `list_user_posts` / `list_local_timeline` (spec lines 52-142) into ssr-gated plain fns taking an explicit `viewer` (drop the internal `viewer_identity().await` — the caller supplies it; drop `viewer_user_id` recompute if it derives from `viewer`). Keep `parse_post_cursor`, pagination, `timeline_post_summary` mapping. Rewrite the `#[server]` fns as wrappers: compute `viewer` via `viewer_identity().await`, then call the fetch fn.

- [ ] **Step 4: Implement `profile` and `site_timeline` handlers** in `server/src/projector/mod.rs`, replacing the stubs:

```rust
async fn site_timeline(Extension(posts): Extension<Arc<dyn common::storage::PostStorage>>) -> Response {
    let viewer = common::viewer::ViewerIdentity::anonymous();
    match web::posts::fetch_local_timeline(&posts, &viewer, None, None, Some(50)).await {
        Ok(page) => ok_html(&PageSeed::SiteTimeline(page)),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn profile(
    Extension(posts): Extension<Arc<dyn common::storage::PostStorage>>,
    Path(username): Path<String>,
) -> Response {
    let viewer = common::viewer::ViewerIdentity::anonymous();
    let uname = username.strip_prefix('~').unwrap_or(&username).to_string();
    match web::posts::fetch_user_posts(&posts, &viewer, &uname, None, None, Some(50)).await {
        Ok(page) => ok_html(&PageSeed::Profile { username: uname, page }),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
```

Executor: confirm whether profile URLs carry the `~` prefix (the components `strip_prefix('~')`); match the projector's route/param handling to the real permalink scheme so `/:username` and `/~username` resolve consistently.

- [ ] **Step 5: Run integration test → PASS** (both backends).

- [ ] **Step 6: Seed-read in `UserTimelinePage` and `HomePage`.** Apply the Task 2 Step 7 pattern: if `use_context::<Option<PageSeed>>()` matches `PageSeed::Profile{..}` (resp. `SiteTimeline`), initialize the `timeline`/`has_more`/`next_cursor_*` signals from `page` and set `initial_loaded=true` synchronously (so the first render shows content with no `Loading…`), instead of relying on the `server_resource` initial fetch. Keep `server_resource`/`ServerAction` for load-more and client-nav. For `HomePage`, read its current impl in `web/src/pages/mod.rs` and mirror the `UserTimelinePage` seeding.

- [ ] **Step 7: Provide the seed context.** Ensure `Option<PageSeed>` is in context for these components. If Task 2 provided it at mount (`csr/src/lib.rs`), nothing to do; otherwise add a `#[cfg(feature = "csr")] provide_context(read_seed())` near the top of `App` in `web/src/pages/mod.rs`.

- [ ] **Step 8: Extend e2e** `projector.spec.ts` with JS-off crawlable checks for `/` and `/:username`.

- [ ] **Step 9: Gate + e2e + commit.**

```bash
git add web/src/posts/listing.rs server/src/projector/mod.rs web/src/pages/posts.rs web/src/pages/mod.rs server/tests/projector.rs end2end/tests/projector.spec.ts
git commit -m "feat(projector): profile + site-timeline verticals (#178, #179)"
```

---

## Task 4 (Commit 3): Tag-page verticals (site + user)

**Files:**
- Modify: `web/src/posts/listing.rs` (extract `fetch_posts_by_tag`; confirm the user-tag data source)
- Modify: `server/src/projector/mod.rs` (implement `site_tag`, `user_tag`)
- Modify: `web/src/pages/posts.rs` (`SiteTagPage`, `UserTagPage` read seed)
- Modify: `server/tests/projector.rs`, `end2end/tests/projector.spec.ts`

**Interfaces:**
- Produces (ssr-only): `fetch_posts_by_tag(posts, viewer, tag, cursor_created_at, cursor_post_id, limit) -> WebResult<TimelinePage>`.

- [ ] **Step 1: Confirm the user-tag data source.** `SiteTagPage` uses `list_posts_by_tag` (site-wide). Grep for how `UserTagPage` fetches (`web/src/pages/posts.rs`) — it may use a `list_user_posts_by_tag` server fn or `list_posts_by_tag` scoped by username. Extract the matching `fetch_*` fn accordingly (one for site-tag, and the user-tag one if distinct). Do not invent a fn — mirror what `UserTagPage` calls today.

- [ ] **Step 2: Failing integration tests** for `/tags/:tag` and `/:username/tags/:tag`.

- [ ] **Step 3: Run → FAIL.**

- [ ] **Step 4: Extract fetch fn(s)** from the tag `#[server]` fn(s) with explicit `viewer` (same pattern as Task 3 Step 3).

- [ ] **Step 5: Implement `site_tag` + `user_tag` handlers** (replace stubs), building `PageSeed::SiteTag { tag, page }` / `PageSeed::UserTag { username, tag, page }`. Lowercase the tag param to match `SiteTagPage`'s `to_lowercase()`.

- [ ] **Step 6: Run integration test → PASS.**

- [ ] **Step 7: Seed-read in `SiteTagPage` + `UserTagPage`** (Task 3 Step 6 pattern, matching `SiteTag`/`UserTag` variants).

- [ ] **Step 8: Extend e2e** with JS-off checks for both tag routes.

- [ ] **Step 9: Gate + e2e + commit.**

```bash
git add web/src/posts/listing.rs server/src/projector/mod.rs web/src/pages/posts.rs server/tests/projector.rs end2end/tests/projector.spec.ts
git commit -m "feat(projector): site + user tag-page verticals (#178, #179)"
```

---

## Task 5 (Commit 4): Cross-cutting guards

Fold any assertions not already covered into explicit, durable guards.

**Files:**
- Modify: `server/tests/projector.rs`, `end2end/tests/projector.spec.ts`

- [ ] **Step 1: Byte-identical + anonymous guard (all five routes).** In `server/tests/projector.rs`, add a parametrized test: for each public route, two GETs → identical bytes, and a GET with an auth cookie set → **same bytes** as anonymous (proves the projector never branches on auth). Run: `cargo test -p server --features csr --test projector` → PASS.

- [ ] **Step 2: No-`reactive_graph`-panic guard.** The `csr-e2e-postgres-chromium` config already runs 4 concurrent workers (the #173 reproduction). Add a focused e2e that hammers the public routes concurrently (rapid navigation across permalink/profile/tag) and asserts no console error / no 500 — the standing zero-panic gate (ADR-0032) catches a `reactive_graph` panic. Confirm the zero-panic assertion is active for this config.

- [ ] **Step 3: Cache-header assertion.** Assert every projector response carries `ETag` + `Cache-Control` and that a conditional `If-None-Match` request returns `304` (implement `If-None-Match` handling in `ok_html` if not already — mirror the feed handlers' 304 path).

- [ ] **Step 4: Full local gate.**

Run: `cargo xtask validate --no-e2e` → PASS (the pre-push gate).
Run: the `csr-e2e-postgres-chromium` check → PASS.

- [ ] **Step 5: Commit.**

```bash
git add server/tests/projector.rs end2end/tests/projector.spec.ts
git commit -m "test(projector): byte-identity, cache, zero-panic guards (#178, #179)"
```

---

## Self-Review notes (planner)

- **Spec coverage:** §3 architecture → Tasks 2-4 (projector in csr arm); §4.1 render module → Task 1; §4.2 fetch layer → Tasks 2-4 Step 3/1; §4.3 PageSeed → Task 1; §4.4 document → Task 2 Step 4; §4.5 client harness → Task 2 Step 6-7; §6 testing → Tasks 2-5; §7 vertical commits → Task structure; §8 acceptance → Task 5 (byte-identity, no-reactive_graph) + Task 2 (crawlable/coincidence/CSR mount/seed). All covered.
- **Known confirm-in-code points (not placeholders — existing APIs to locate, flagged inline):** `ViewerIdentity` path + anonymous ctor; `Slug`/`Username`/`PostStorage`/`InternalError`/`post_response` paths; the content hasher used by feed ETags; the exact leptos 0.8 `mount_to`/context API; the `~`-prefix username scheme; the `UserTagPage` data source; `HomePage`'s current impl. Each is an existing-code lookup, not an invented interface.
- **Type consistency:** `PageSeed` variants (Task 1) are matched consistently in the projector handlers (Tasks 2-4) and the component seed-reads (Tasks 2-4). `fetch_*` signatures all take `(&Arc<dyn PostStorage>, &ViewerIdentity, …) -> WebResult<…>`.
