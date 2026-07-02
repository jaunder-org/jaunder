# Authenticated-owner flash handling — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the authenticated owner flash-free on the cacheable public pages
— detect auth before first paint via a localStorage marker, decorate the
projector-painted DOM additively (never re-render a different DOM), keep `/` the
enhanced public timeline, and relocate the personalized Feed to a `/app`
cockpit.

**Architecture:** A JS-readable localStorage **auth marker** (`{username}`,
advisory — the real session stays the HTTP-only cookie) is read by a tiny inline
blocking `<head>` script that sets `<html class="authed">` before first paint,
and read synchronously by the CSR components at mount so they render authed
chrome without an async `<Suspense>` swap. Owner affordances are **additive**
(own-post action column as a sibling on the byte-coincident `inner_html`
content; authed sidebar chrome into reserved slots). `current_user()` stays as a
background reconcile. See `docs/adr/0043-*` and the spec
`docs/superpowers/specs/2026-07-02-issue-181-authed-flash.md`.

**Tech Stack:** Rust, leptos-CSR (wasm), axum projector, Playwright e2e,
`cargo xtask` gate.

## Global Constraints

- **wasm-only code is gated `#[cfg(target_arch = "wasm32")]`, never
  `feature = "csr"`** — so it stays out of host coverage (memory lesson; the
  existing `mount_csr`/`read_dom_seed` do this).
- **Pure, host-reachable logic is host-tested and keeps coverage** (marker
  encode/decode, the pre-paint const, `render_post_content`); page-component
  wasm gaps are auto-approved per the coverage policy — do **not** lower a
  server-fn or storage baseline.
- The auth marker is **advisory, not a credential** — never store a
  token/secret; the server still authorizes every edit/delete by session.
- **Anonymous responses stay byte-identical/cacheable** — the projector still
  renders `ViewerIdentity::Anonymous` only; every owner adjustment is
  client-side.
- Per-task gate: `cargo xtask check` (fmt + clippy + Nix coverage/tests). **No
  `Co-Authored-By` trailer.** One clean commit per task.
- `.ts` e2e edits bust the coverage cache (~2.3 min/commit) — expected.
- ADR-0043 is already written (start phase); **its number reconciles at ship**
  (0042 is #160/PR#195's; rebase onto main before merge).

---

### Task 1: File the deferred follow-on issues

No code. Capture the separable concerns up front (per `jaunder-start` step 5) so
they can be picked up concurrently, not blocked behind this cycle. Use the
`jaunder-issues` skill; milestone "Off concurrent SSR (web re-architecture v1)"
where they belong, `web` label.

- [x] **Step 1: File "synced `/` redirect preference (owner: stay vs.
      cockpit)".** → **#201**. Body: the pre-paint redirect-pref _read_ path +
      safe stay-default land in #181; this issue adds the user-facing toggle
      UI + cross-device sync of the preference. **Depends on the §6 sync
      engine.** Reference #181, ADR-0043 D7.

- [x] **Step 2: File "empirical layout-shift (CLS) e2e assertion for authed
      flash".** → **#202**. Body: #181 verifies flash-freeness structurally
      (coincidence unit test) + pre-paint/affordance e2e. This _possible
      follow-up_ adds a Playwright bounding-box/CLS check that content doesn't
      move between first paint and post-mount. **Downside stated in the issue:**
      timing/browser-dependent flakiness — the kind of instability #182's
      parallel-e2e goal is sensitive to; only add if deterministic. Reference
      #181, ADR-0043 D8.

- [x] **Step 3: File "richer cockpit surface (read-state, inline drafts, nav
      hub)".** → **#203**. Body: #181 lands the cockpit at `/app` as the
      relocated home Feed; this issue grows it into the full owner workspace
      (read-state, inline drafts, a nav hub to the other authed pages, possibly
      re-nesting authed routes under `/app`). **Depends on the §6 sync engine.**
      Reference #181, ADR-0043 D6.

- [x] **Step 4: Record the three issue numbers** in this plan file (done above:
      #201/#202/#203) so the cycle's follow-ons are traceable. Commit:

```bash
git add docs/superpowers/plans/2026-07-02-issue-181-authed-flash.md
git commit -m "docs(issue-181): file authed-flash follow-on issues"
```

---

### Task 2: The auth marker module (`web::auth::marker`)

**Files:**

- Create: `web/src/auth/marker.rs`
- Modify: `web/src/auth/mod.rs` (add `pub mod marker;`)

**Interfaces:**

- Produces:
  - `pub fn encode_marker(username: &str) -> String` — the localStorage value
    (JSON `{"username":"…"}`). Pure, host-testable.
  - `pub fn decode_marker(raw: &str) -> Option<String>` — parse the value back
    to the username, `None` on malformed. Pure, host-testable.
  - `pub const MARKER_KEY: &str = "jaunder_auth";`
  - `#[cfg(target_arch = "wasm32")] pub fn read() -> Option<String>` — read
    `MARKER_KEY` from `localStorage` and `decode_marker`.
  - `#[cfg(target_arch = "wasm32")] pub fn set(username: &str)` — write
    `encode_marker(username)`.
  - `#[cfg(target_arch = "wasm32")] pub fn clear()` — remove `MARKER_KEY`.

The JSON shape matches the pre-paint script's parser (Task 3) — keep them in
sync (the script reads `.username`).

- [x] **Step 1: Write the failing tests** (`web/src/auth/marker.rs`, in-file
      `#[cfg(test)]` — this is the crate's convention and NOT a dialect file):

```rust
#[cfg(test)]
mod tests {
    use super::{decode_marker, encode_marker};

    #[test]
    fn round_trips_username() {
        let raw = encode_marker("alice");
        assert_eq!(raw, r#"{"username":"alice"}"#);
        assert_eq!(decode_marker(&raw), Some("alice".to_string()));
    }

    #[test]
    fn decode_rejects_malformed() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None);
    }

    #[test]
    fn encode_escapes_json() {
        // A quote in a username must not break the JSON the pre-paint script parses.
        assert_eq!(decode_marker(&encode_marker(r#"a"b"#)), Some(r#"a"b"#.into()));
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p jaunder-web round_trips_username` Expected: FAIL —
`encode_marker`/`decode_marker` not defined. (Crate name: confirm with
`cargo metadata`; the web crate is `web` in-tree, package name per its
`Cargo.toml` — use that in `-p`.)

- [x] **Step 3: Write the implementation** (`web/src/auth/marker.rs`):

```rust
//! The client-side **auth marker** (#181, ADR-0043): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation. The pre-paint `<head>` script
//! (`render::PREPAINT_SCRIPT`) reads the SAME key + `.username` field.

use serde::{Deserialize, Serialize};

/// The localStorage key holding the marker. Kept in sync with the pre-paint script.
pub const MARKER_KEY: &str = "jaunder_auth";

#[derive(Serialize, Deserialize)]
struct Marker<'a> {
    username: &'a str,
}

/// The localStorage value for `username` (JSON `{"username":"…"}`).
#[must_use]
pub fn encode_marker(username: &str) -> String {
    serde_json::to_string(&Marker { username }).unwrap_or_default()
}

/// Parse a marker value back to its username, `None` when malformed/empty.
#[must_use]
pub fn decode_marker(raw: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Owned {
        username: String,
    }
    let m: Owned = serde_json::from_str(raw).ok()?;
    (!m.username.is_empty()).then_some(m.username)
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read + decode the marker from localStorage (browser-only).
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn read() -> Option<String> {
    let raw = storage()?.get_item(MARKER_KEY).ok().flatten()?;
    decode_marker(&raw)
}

/// Write the marker for `username` (browser-only).
#[cfg(target_arch = "wasm32")]
pub fn set(username: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(MARKER_KEY, &encode_marker(username));
    }
}

/// Remove the marker (browser-only).
#[cfg(target_arch = "wasm32")]
pub fn clear() {
    if let Some(s) = storage() {
        let _ = s.remove_item(MARKER_KEY);
    }
}
```

Add `pub mod marker;` to `web/src/auth/mod.rs` (near the top, after the
imports).

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p jaunder-web marker::` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add web/src/auth/marker.rs web/src/auth/mod.rs
git commit -m "feat(web): client-side auth marker (advisory localStorage presence)"
```

---

### Task 3: Pre-paint `<head>` script — const, projector wiring, shell, drift guard

**Files:**

- Modify: `web/src/render/mod.rs` (add `PREPAINT_SCRIPT` const + a `render_head`
  or `document`-level insertion point; add a drift-guard test)
- Modify: `server/src/projector/mod.rs` (`document()` inserts the script into
  `<head>`)
- Modify: `csr/index.html` (paste the same `<script>` into `<head>`)

**Interfaces:**

- Produces: `pub const PREPAINT_SCRIPT: &str` in `web::render` — the exact
  `<script>…</script>` element, inserted verbatim into every `<head>`.

The script: reads `localStorage['jaunder_auth']`, and if a `.username` is
present sets `document.documentElement.classList.add('authed')` and a
`data-user` attribute. It also reads the (currently unwritten) redirect-pref key
`jaunder_home_redirect` and, only if it equals `"app"` **and** the path is
exactly `/`, redirects to `/app` — the stay-default means this never fires until
a future issue writes the key (ADR-0043 D7/D10). Inline + blocking + first in
`<head>`.

- [x] **Step 1: Write the failing tests** (`web/src/render/mod.rs` tests
      module):

```rust
#[test]
fn prepaint_script_is_inline_blocking_and_reads_the_marker() {
    let s = super::PREPAINT_SCRIPT;
    assert!(s.starts_with("<script>") && s.ends_with("</script>"), "{s}");
    // No async/defer/src — a network round-trip would defeat pre-paint.
    assert!(!s.contains("src=") && !s.contains("defer") && !s.contains("async"), "{s}");
    // Reads the same key + field the marker module writes.
    assert!(s.contains("jaunder_auth"), "{s}");
    assert!(s.contains(".username"), "{s}");
    assert!(s.contains("classList") && s.contains("authed"), "{s}");
}
```

And a drift guard asserting the static shell carries it verbatim:

```rust
#[test]
fn index_html_shell_contains_the_prepaint_script() {
    // The projector's SPA-shell fallback IS csr/index.html; it must carry the
    // identical pre-paint script so authed-only / shell-fallback pages pre-paint too.
    let index = include_str!("../../../csr/index.html");
    assert!(
        index.contains(super::PREPAINT_SCRIPT),
        "csr/index.html must embed render::PREPAINT_SCRIPT verbatim (drift guard)"
    );
}
```

(Confirm the relative path from `web/src/render/mod.rs` to `csr/index.html`
resolves; adjust the `../` depth if the crate root differs.)

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p jaunder-web prepaint` Expected: FAIL —
`PREPAINT_SCRIPT` undefined; index.html lacks it.

- [x] **Step 3: Add the const** (`web/src/render/mod.rs`, near `DEFAULT_THEME`):

```rust
/// The pre-paint auth-detection script (#181, ADR-0043). A tiny inline, blocking
/// `<head>` script: reads the localStorage auth marker (`jaunder_auth`, same key
/// as `auth::marker`) and marks `<html class="authed" data-user=…>` BEFORE first
/// paint, so CSS reserves the authed layout and the SPA boots already knowing.
/// Never external/deferred (a round-trip would guarantee paint-then-swap). The
/// redirect-pref (`jaunder_home_redirect`) read path is present with the safe
/// stay-default (nothing writes the key yet — see ADR-0043 D7/D10). Bytes are
/// identical for every visitor → cacheability intact. Kept byte-identical in
/// `csr/index.html` (drift-guarded by a unit test).
pub const PREPAINT_SCRIPT: &str = "<script>(function(){try{\
var m=localStorage.getItem('jaunder_auth');\
if(m){var u=JSON.parse(m).username;\
if(u){var e=document.documentElement;e.classList.add('authed');e.setAttribute('data-user',u);\
if(localStorage.getItem('jaunder_home_redirect')==='app'&&location.pathname==='/'){location.replace('/app');}}}\
}catch(_){}})();</script>";
```

- [x] **Step 4: Insert it in the projector `<head>`**
      (`server/src/projector/mod.rs`, `document()`), first thing inside
      `<head>`:

```rust
    format!(
        concat!(
            "<!DOCTYPE html><html lang=\"en\"><head>{prepaint}{head}</head><body>",
            "<div id=\"app\">{body}</div>",
            "<script type=\"application/json\" id=\"jaunder-seed\">{blob}</script>",
            "<script type=\"module\">import init from \"/pkg/jaunder.js\"; init();</script>",
            "</body></html>",
        ),
        prepaint = web::render::PREPAINT_SCRIPT,
        head = head,
        body = body,
        blob = blob.replace("</", "<\\/"),
    )
```

- [x] **Step 5: Paste the same script into `csr/index.html`** as the first child
      of `<head>` (before the `<meta charset>`), byte-identical to
      `PREPAINT_SCRIPT`:

```html
<head>
  <script>
    (function () {
      try {
        var m = localStorage.getItem("jaunder_auth");
        if (m) {
          var u = JSON.parse(m).username;
          if (u) {
            var e = document.documentElement;
            e.classList.add("authed");
            e.setAttribute("data-user", u);
            if (
              localStorage.getItem("jaunder_home_redirect") === "app" &&
              location.pathname === "/"
            ) {
              location.replace("/app");
            }
          }
        }
      } catch (_) {}
    })();
  </script>
  <meta charset="utf-8" />
</head>
```

- [x] **Step 6: Run the tests, verify they pass**

Run:
`cargo nextest run -p jaunder-web prepaint && cargo nextest run -p jaunder index_html`
Expected: PASS (the projector document test lives with the server crate; the
drift guard lives in `web`).

- [x] **Step 7: Add a projector-document assertion** so the projector wiring is
      covered (`server/src/projector/mod.rs` tests):

```rust
#[test]
fn document_head_starts_with_the_prepaint_script() {
    use super::document;
    use web::render::PageSeed;
    let doc = document(&PageSeed::SiteTimeline(web::posts::TimelinePage {
        posts: vec![],
        next_cursor_created_at: None,
        next_cursor_post_id: None,
        has_more: false,
    }));
    assert!(doc.contains(web::render::PREPAINT_SCRIPT), "{doc}");
    assert!(doc.contains("<head><script>(function()"), "prepaint is first in head: {doc}");
}
```

- [x] **Step 8: Commit**

```bash
git add web/src/render/mod.rs server/src/projector/mod.rs csr/index.html
git commit -m "feat(web): pre-paint auth-detection script on projector + shell (single source, drift-guarded)"
```

---

### Task 4: Write / clear the marker on login, register, logout

**Files:**

- Modify: the client components that handle login/register/logout success.
  Locate with
  `rg -n 'ServerAction::<Login>|ServerAction::<Register>|ServerAction::<Logout>|<ActionForm' web/src/pages`
  and `rg -n 'LoginPage|RegisterPage|LogoutPage' web/src/pages`.

**Interfaces:**

- Consumes: `web::auth::marker::{set, clear}` (Task 2, wasm-only).

On a successful `login`/`register` action the client knows the submitted
username; write the marker. On `logout`, clear it. The server-side
`set_session_cookie` / `clear_session_cookie` are unchanged — the marker is the
_client_ mirror of that.

- [x] **Step 1: Find the success-handling sites.** Run the `rg` above; identify
      the `Effect`/callback that fires on `action.value()` = `Ok(_)` for each of
      login/register/logout (mirroring `home.rs`'s `Effect::new` pattern at
      `web/src/pages/home.rs:76`).

- [x] **Step 2: Write the marker on login/register success.** In each success
      effect, guarded wasm-only (the surrounding component already runs only in
      the browser, but gate the `marker` calls to be safe):

```rust
#[cfg(target_arch = "wasm32")]
{
    // `username` is the value submitted to the action (already in scope for the form).
    crate::auth::marker::set(&username);
}
```

- [x] **Step 3: Clear the marker on logout success** (LogoutPage's success
      effect):

```rust
#[cfg(target_arch = "wasm32")]
{
    crate::auth::marker::clear();
}
```

- [x] **Step 4: Verify it compiles for wasm and host.**

Run: `cargo xtask check --no-test` Expected: clippy + fmt clean; no `dead_code`
on the host build (the `marker` wasm fns are `#[cfg(target_arch = "wasm32")]`,
so the host build won't see them — ensure no host-only reference to them).

- [x] **Step 5: Commit**

```bash
git add web/src/pages/<login/register/logout files>
git commit -m "feat(web): mirror the session into the auth marker on login/register/logout"
```

---

### Task 5: Sidebar — marker-driven synchronous authed render (kill the async gate)

**Files:**

- Modify: `web/src/pages/ui.rs` (`Sidebar`, lines 1032-1154)

**Interfaces:**

- Consumes: `web::auth::marker::read` (wasm), `render::render_sidebar`,
  `current_user`, `current_user_is_operator`.

Today the sidebar gates the authed build on the async `current_user()` inside
`<Suspense>` — the paint-then-swap. Read the marker **synchronously** to choose
anon vs. authed at mount; keep `current_user()`/`operator` as background
reconcile (and for the operator-only admin links, which the marker doesn't
carry). On host (non-wasm) `marker::read` is absent, so the initial state is
`None` (anon) — fine, the sidebar is wasm-only chrome.

- [x] **Step 1: Write a failing e2e-adjacent unit** — the pure marker match is
      already covered (Task 2). The sidebar is wasm-reactive (out of host
      coverage), so its correctness is pinned by the e2e in Task 9. Here, add a
      host-compilable guard that the authed sidebar's username source is the
      marker, not only the resource: extract the authed-sidebar _builder_ into a
      pure-ish helper if it reduces risk, OR rely on Task 9's e2e. **Decision:**
      rely on Task 9's e2e (the sidebar view is intrinsically wasm/DOM); this
      task's gate is compile + clippy + the Task 9 e2e. Skip a bespoke unit test
      here.

- [x] **Step 2: Restructure `Sidebar`** so the initial authed/anon choice is
      synchronous from the marker. Replace the `<Suspense>`-gated body with a
      signal seeded from the marker and reconciled by the resource:

```rust
#[component]
pub fn Sidebar(#[prop(optional)] active: Option<String>) -> impl IntoView {
    let active_key = active.unwrap_or_default();
    let location = use_location();

    // Synchronous boot source (#181): the marker decides authed-vs-anon at mount,
    // so there is no async <Suspense> swap. `current_user()` reconciles below.
    let owner = RwSignal::new(marker_username_on_boot());
    let operator = crate::server_resource(move || location.pathname.get(), |_| current_user_is_operator());

    // Reconcile: correct a stale marker (dead session → clear; live session with a
    // missing marker → set) without gating first paint.
    let reconcile = crate::server_resource(move || location.pathname.get(), |_| current_user());
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(res) = reconcile.try_get() {
            match res {
                Ok(Some(u)) => { crate::auth::marker::set(&u); if owner.get_untracked().as_deref() != Some(u.as_str()) { owner.set(Some(u)); } }
                Ok(None) => { crate::auth::marker::clear(); if owner.get_untracked().is_some() { owner.set(None); } }
                Err(_) => {}
            }
        }
    });

    let anon_html = crate::render::render_sidebar(&active_key);
    view! {
        <aside class="j-sidebar">
            {move || match owner.get() {
                None => view! { <div style="display:contents" inner_html=anon_html.clone()></div> }.into_any(),
                Some(username) => authed_sidebar(&active_key, &username, matches!(operator.get(), Some(Ok(true)))).into_any(),
            }}
        </aside>
    }
}

/// Boot-time marker read: `Some(username)` in the browser when the marker is set,
/// `None` on the host build (the sidebar only ever renders in wasm).
fn marker_username_on_boot() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    { crate::auth::marker::read() }
    #[cfg(not(target_arch = "wasm32"))]
    { None }
}
```

Extract the existing authed-sidebar `view!` (ui.rs:1073-1147) into
`fn authed_sidebar(active_key: &str, username: &str, is_operator: bool) -> impl IntoView`
so both the initial render and the reconciled render share it. Keep the existing
markup verbatim (brand, search, nav, sources, footer avatar) — only its inputs
change from awaited values to the params.

- [x] **Step 3: Verify compile + clippy.**

Run: `cargo xtask check --no-test` Expected: clean. Watch for `Effect`/signal
`Send`+ownership issues typical of leptos closures; clone
`active_key`/`anon_html` into each closure as the original did.

- [x] **Step 4: Commit**

```bash
git add web/src/pages/ui.rs
git commit -m "feat(web): sidebar reads the auth marker synchronously (no async authed swap)"
```

---

### Task 6: Converge `PostDisplay` authored branch onto the coincident `inner_html` seam

**Files:**

- Modify: `web/src/pages/ui.rs` (`PostDisplay`, lines 292-384)
- Modify: `web/src/render/mod.rs` (add a coincidence unit test)

**Interfaces:**

- Consumes: `render::render_post_content`, `render::render_avatar` / `Avatar`.
- The authored branch must produce, for the content column, the **same DOM** the
  anonymous branch (`render_post_inner`) and the projector emit — plus the
  action column as an additive sibling. The action column (`children()`) stays
  reactive (it carries edit/delete handlers `inner_html` can't).

Target authored structure (coincides with
`<article inner_html=render_post_inner>` when no children, per
`render_post_inner`'s shape: avatar + flex wrapper + content div):

```rust
Some(children) => {
    let inner_content = crate::render::render_post_content(&view); // view built as in the None arm
    view! {
        <article class="j-post">
            <Avatar name=post.username.clone() size=38 />
            <div style="min-width:0;display:flex;gap:8px;align-items:flex-start">
                <div style="flex:1;min-width:0" inner_html=inner_content></div>
                {children()}
            </div>
        </article>
    }
    .into_any()
}
```

This is the structure the `render_post_content` doc comment already anticipates
("slots this into the same content `<div>` via `inner_html` and overlays the
reactive action column as a sibling"). It removes the hand-rebuilt reactive
header/title/body markup (ui.rs:338-375) that diverged from the projector — the
divergence that made the authored path unable to coincide.

> **Regression watch:** memory records that a _prior_ unification regressed the
> authed home-feed re-render. That attempt put the WHOLE article (incl. action
> column) into `inner_html`, dropping the reactive handlers. This task keeps the
> action column reactive — only the content column is `inner_html`. Task 9's e2e
> re-checks the authed home feed explicitly.

- [x] **Step 1: Write the failing coincidence test** (`web/src/render/mod.rs`
      tests) — the anonymous inner and the author content-column share one
      source:

```rust
#[test]
fn author_content_column_equals_anonymous_content() {
    // The authored PostDisplay injects render_post_content via inner_html; assert
    // it is exactly the content the anonymous render_post_inner wraps, so the
    // author article's content coincides with the projector's anonymous paint.
    let ctx = TagCtx::ForUser("alice".into());
    let view = PostView {
        username: "alice", title: Some("T"), banner: None, summary: None,
        rendered_html: "<p>b</p>", time: "2026-01-01 00:00",
        permalink: "/~alice/x", tags: &[], tag_ctx: &ctx, is_author: true,
    };
    let content = render_post_content(&view);
    // render_post_inner = avatar + flex wrapper + <div flex:1>{content}</div>
    let inner = render_post_inner(&view);
    assert!(inner.contains(&content), "author inner_html content must be a substring of the anonymous inner: {inner}");
}
```

- [x] **Step 2: Run it, verify it passes already** (both call
      `render_post_content`) — this test _locks_ the seam so a future edit to
      either side can't silently diverge. If it fails, the render layer already
      drifted; fix before proceeding.

Run:
`cargo nextest run -p jaunder-web author_content_column_equals_anonymous_content`
Expected: PASS (guard test).

- [x] **Step 3: Rewrite the `Some(children)` arm of `PostDisplay`** to the
      target structure above. Build `view` (the `PostView`) once before the
      `match` so both arms share it (the `None` arm already builds it). Remove
      the divergent reactive markup.

- [x] **Step 4: Verify compile + clippy + the render tests.**

Run: `cargo xtask check` Expected: clean; all `render` tests pass.

- [x] **Step 5: Commit**

```bash
git add web/src/pages/ui.rs web/src/render/mod.rs
git commit -m "refactor(web): author post layout shares the coincident inner_html content seam"
```

---

### Task 7: `/` stays the enhanced public timeline — remove the Local→Feed swap; own-post affordances via marker

**Files:**

- Modify: `web/src/pages/home.rs` (`HomePage`, lines 24-257)
- Modify: `web/src/pages/ui.rs` (`PostCard` — show the action column when the
  marker username matches the post author)

**Interfaces:**

- Consumes: `web::auth::marker::read` (wasm), the seeded `SiteTimeline`.

`HomePage` must stay **Local** for everyone (no `TimelineMode::Feed`): remove
the Feed branch and the `current_user()`-driven mode swap. The owner's own posts
in the Local timeline gain the action column, decided **client-side** from the
marker (`post.username == marker_username`) — no viewer-aware re-fetch, so the
affordance appears synchronously at mount into a CSS-reserved gutter (Task 8).

- [x] **Step 1: Simplify `HomePage`.** Delete `TimelineMode::Feed`, the
      `initial_page` `server_resource` that called `current_user()` +
      `list_home_feed`, and the mode-swap `Effect`. Keep: the seed adoption
      (Local, lines 39-47), `list_local_timeline` for load-more, and the Local
      view branch (topbar + hero + posts). The page is now single-mode Local;
      drop the `match read_timeline_mode()` wrapper, rendering the Local branch
      directly. `InlineComposer` is **removed** from `/` (it moves to the
      cockpit, Task 8).

- [x] **Step 2: Own-post affordance in `PostCard`.** `PostCard` (ui.rs:388)
      already keys its action column on `post.is_author`. Add a client-side
      owner match so the owner's posts in the Local timeline show the column
      even though the seed/anonymous data has `is_author = false`:

```rust
let is_author = post.is_author || marker_matches(&post.username);
```

where `marker_matches` is a small helper:

```rust
fn marker_matches(author: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    { crate::auth::marker::read().as_deref() == Some(author) }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = author; false }
}
```

(The server still authorizes the actual edit/delete by session — the marker only
gates the affordance's visibility.)

- [x] **Step 3: Write/adjust the failing test.** `HomePage`/`PostCard` are
      wasm-reactive (out of host coverage); their behavior is pinned by Task 9's
      e2e ("owner on `/` sees the local timeline, not a personal feed; own posts
      show edit"). Add the e2e there. Here, ensure the pure `marker_matches`
      host arm returns `false` (compile guard) — no separate unit needed. If a
      `home.rs` host unit exists for `TimelineMode`, update it for the removed
      variant.

- [x] **Step 4: Verify compile + clippy + full check.**

Run: `cargo xtask check` Expected: clean. Confirm no dangling references to
`TimelineMode::Feed`, `list_home_feed`, or `InlineComposer` in `home.rs`.

- [x] **Step 5: Commit**

```bash
git add web/src/pages/home.rs web/src/pages/ui.rs
git commit -m "feat(web): / stays the enhanced public timeline; own-post affordances from the marker"
```

---

### Task 8: The `/app` cockpit route hosting the relocated home Feed

**Files:**

- Create: `web/src/pages/cockpit.rs` (the relocated Feed branch)
- Modify: `web/src/pages/mod.rs` (route + re-export)
- Modify: `web/src/pages/ui.rs` (`Sidebar` nav — add an "App"/cockpit item; mark
  `/app` active) and `render::NAV_ITEMS` if the cockpit link belongs in the
  shared nav
- Modify: `web/src/render/mod.rs` CSS reserve rules live in Task with CSS
  (below); the `NAV_ITEMS` addition (if any) here

**Interfaces:**

- Produces: `pub fn CockpitPage() -> impl IntoView` — `InlineComposer` +
  `list_home_feed` + `PostCard`s + load-more (the current `home.rs` Feed branch,
  lines 188-211 plus its resource/effect plumbing), authed-only.

- [x] **Step 1: Create `CockpitPage`** in `web/src/pages/cockpit.rs` by moving
      the Feed-mode logic out of `home.rs`: the `list_home_feed` fetch, the
      `current_user()` gate (here it _is_ legitimately async — an
      anonymous/expired visitor to `/app` must bounce), the `InlineComposer`,
      the `PostCard` list, and load-more. On `current_user()` = `None`/error →
      redirect to `/login` (mirror the existing authed-route bounce —
      `rg -n 'redirect\("/login"\)|navigate.*login' web/src/pages` for the
      established pattern).

- [x] **Step 2: Register the route** (`web/src/pages/mod.rs`, inside `<Routes>`,
      before `ParamSegment("username")` so the static segment wins):

```rust
    <Route path=StaticSegment("app") view=CockpitPage />
```

Add `mod cockpit; pub use cockpit::CockpitPage;` and include it in the `ui`
re-export block as needed.

- [x] **Step 3: Sidebar nav entry.** Add a cockpit nav item so the owner can
      reach `/app` (and it shows active there). Either extend
      `render::NAV_ITEMS` with
      `("app", "App", Icons::HOME_or_new, Some("/app"), true)` (authed-only, so
      it only appears in the authed sidebar and never in the anonymous
      `render_sidebar`), or add it directly in `authed_sidebar` (Task 5). Prefer
      `NAV_ITEMS` for one source of truth; confirm the anonymous
      `render_sidebar` still omits it (it filters `auth_required`).

- [x] **Step 4: Test.** Cockpit is wasm-reactive → pinned by Task 9 e2e
      ("bookmark `/app` → lands directly in the feed+composer, zero clicks;
      anonymous `/app` → `/login`"). Verify compile + clippy.

Run: `cargo xtask check` Expected: clean.

- [x] **Step 5: Commit**

```bash
git add web/src/pages/cockpit.rs web/src/pages/mod.rs web/src/pages/ui.rs web/src/render/mod.rs
git commit -m "feat(web): /app cockpit route hosting the relocated home feed"
```

---

### Task 9: CSS — reserve the authed layout so decoration causes no reflow

**Files:**

- Modify: `csr/style/jaunder.css` (confirm the path with
  `rg -n 'j-sidebar|j-sb-foot|j-post' csr/style/*.css`)

**Interfaces:** none (styling). The pre-paint script sets `html.authed` before
paint; these rules reserve space for the authed chrome (footer avatar row, the
own-post action-column gutter) so that when the wasm client fills those slots
the content around them does not move.

- [ ] **Step 1: Add reserve rules.** For example (adapt to the real class names
      / measurements found in the stylesheet):

```css
/* #181: pre-paint auth reserves the authed chrome's space so the CSR fill
   (footer avatar, own-post action column) causes no reflow. */
html.authed .j-sb-foot {
  min-height: 44px;
}
html.authed .j-post {
  /* reserve the action-column gutter width */
}
```

Measure the actual authed footer height and action-column width from the
rendered components and reserve exactly that, so the fill is invisible
movement-wise.

- [ ] **Step 2: Verify via the app** (manual, or defer to Task 10 e2e). Since
      CSS has no unit test, its correctness is the "no reflow" e2e in Task 10.

Run: `cargo xtask check` Expected: clean (a `.css` edit does not bust the
coverage cache; only `.ts` does).

- [ ] **Step 3: Commit**

```bash
git add csr/style/jaunder.css
git commit -m "style(web): reserve authed-chrome layout under html.authed (no-reflow)"
```

---

### Task 10: e2e — pre-paint auth + affordance presence across the authed flows

**Files:**

- Modify/create: `end2end/tests/*.spec.ts` (find the authed specs with
  `rg -n 'login|authed|owner|drafts' end2end/tests`)

**Interfaces:** Playwright. Assert the pre-paint contract and the enhance
behavior without brittle pixel diffing (ADR-0043 D8).

- [ ] **Step 1: Pre-paint class assertion.** As a logged-in owner, navigate to
      `/` (a projector page) and assert `document.documentElement` carries
      `authed` **synchronously** — read it as early as possible (e.g. right
      after `page.goto`, before network idle), and assert `data-user` equals the
      username. This proves auth is known before the wasm client's async work.

```ts
await page.goto("/");
const authed = await page.evaluate(() =>
  document.documentElement.classList.contains("authed"),
);
expect(authed).toBe(true);
```

- [ ] **Step 2: `/` stays public, own-post affordance.** As the owner on `/`,
      assert the local-timeline topbar (`jaunder.local`) is present (NOT the
      personal-feed "Your home feed"), and that the owner's own post shows an
      edit affordance.

- [ ] **Step 3: Cockpit bookmarkability.** Navigate directly to `/app`; assert
      the composer + feed render (zero intermediate clicks). Then as an
      anonymous context, navigate to `/app` and assert the bounce to `/login`.

- [ ] **Step 4: Sidebar no async swap.** As the owner, assert the authed sidebar
      footer avatar + authed nav items are present after boot; as anonymous,
      assert they are absent (the anon sidebar).

- [ ] **Step 5: Run the e2e locally.**

Run (background, per CLAUDE.md long-run guidance):
`cargo xtask e2e sqlite chromium` Expected: PASS. (`tsc` gate also runs — keep
the specs type-clean.)

- [ ] **Step 6: Commit**

```bash
git add end2end/tests/<specs>
git commit -m "test(e2e): pre-paint auth, enhanced /, cockpit bookmarkability, sidebar chrome"
```

---

### Task 11: Docs finalize + full gate

**Files:**

- `docs/adr/0043-*` (already written — verify), `docs/README.md` (row — verify),
  `docs/hub-architecture.md` §8 (terms — verify), the spec (already updated).

- [ ] **Step 1: Verify the ADR/README/glossary** land the final decisions
      (relocated-Feed cockpit, `/`-no-swap D10). They were authored in the start
      phase; reconcile any wording with what actually shipped.

- [ ] **Step 2: Run the full local gate.**

Run: `cargo xtask validate` Expected: PASS across
`{sqlite,postgres}×{chromium,firefox}`. Investigate any
`already been disposed`/panic (there should be none — this issue removes
reactive gates, it does not add SSR).

- [ ] **Step 3: Commit any doc reconciliation.**

```bash
git add docs/
git commit -m "docs(issue-181): reconcile ADR-0043 + glossary with shipped design"
```

---

## Self-Review notes

- **Spec coverage:** D1→T2/T3; D2→T2; D3→T5 (+T7 home reconcile removal);
  D4→T5/T6 (+T9 CSS, +T7 own-post); D5→T3; D6→T8; D7→T3 (read path); D8→T6
  unit + T10 e2e; D9→ADR (start) + T11; D10→T7. Follow-ons→T1.
- **Type consistency:** `marker::{encode_marker,decode_marker,read,set,clear}`,
  `render::PREPAINT_SCRIPT`, `marker_matches`, `marker_username_on_boot`,
  `authed_sidebar`, `CockpitPage` — used consistently across tasks.
- **Right-sizing:** each task ends at an independently gate-able deliverable;
  the reactive tasks (T5–T8) lean on T10 e2e because the views are intrinsically
  wasm/DOM (out of host coverage), which is the established pattern for
  `web/pages`.
