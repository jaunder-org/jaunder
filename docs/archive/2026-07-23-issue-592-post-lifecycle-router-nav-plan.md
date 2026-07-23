# #592 post-lifecycle router-navigation Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
`docs/superpowers/specs/2026-07-23-issue-592-post-lifecycle-router-nav.md` (the
"what/why"; this plan is the "how"). **Issue:** jaunder-org/jaunder#592.

**Goal:** Turn the post-lifecycle full-document reloads (publish, unpublish,
permalink misroute) into client-side router navigations, constrain the permalink
route to `~`-prefixed usernames, delete the now-dead `client::navigation`, and
lock the win with an xtask source-scan gate.

**Architecture:** Publish/unpublish switch from
`client::navigation::replace/reload` to `leptos_router` `use_navigate()`; the
one staleness hazard (in-place publish on a permalink page) is closed by
refetching `PostPage`'s resource via the existing `on_mutate` callback
convention. A custom `PossibleRouteMatch` segment (delegating to `ParamSegment`,
rejecting non-`~` captures) replaces `ParamSegment("username")` on the permalink
route so a non-`~` 5-segment URL no longer mounts `PostPage`; the unreachable
`reload()` escape is deleted. An xtask static check (the
`proffered_secret_check` family) forbids raw `window().location()` navigation in
`web/src` + `client/src`.

**Tech Stack:** Rust, Leptos 0.8 / leptos_router (declared floor `0.8.2`, lock
resolves `0.8.13`; CSR), `cargo xtask`, Playwright (`end2end/`).

## Global Constraints

- **No `Co-Authored-By` trailer** on commits (user preference).
- **Backend parity / coverage / dialect-file rules:** per `CONTRIBUTING.md`.
  (This issue touches no storage/dialect code; no dual-backend tests needed.)
- **Wasm-only code must pass wasm clippy** before commit:
  `cargo clippy -p web --target wasm32-unknown-unknown --features csr -- -D warnings`
  (the default `cargo check`/host clippy skips wasm-gated paths).
- **Per-commit gate:** the pre-commit hook runs full `cargo xtask check`; run it
  first so it passes clean (`jaunder-commit`). No editing during a gated commit.
- **`issue-592` token** stays in the plan/spec filenames (load-bearing for
  `jaunder-develop`/`jaunder-ship`).

---

## Review header

**Scope — in:**

- `web/src/pages/`: a custom `~`-only route segment + wiring it onto the
  permalink route.
- `web/src/posts/component.rs`: 3 `replace` sites → `use_navigate`; `PostPage`
  resource refetch on in-place publish; delete the `reload()` escape + its
  now-dead None branch.
- `client/src`: delete the `navigation` module (zero callers after the above).
- `xtask/src`: new `no_full_reload_check` static check + registration + unit
  tests.
- `end2end/tests/posts.spec.ts`: no-reload sentinel assertions on
  publish/unpublish; a new unpublish→`/drafts` test; prune the obsolete
  `location.replace`/`waitForHydration` caveat.
- `docs/adr/drafts/`: an ADR draft for the no-full-load invariant +
  `~`-namespace rule.

**Scope — out:** login/logout hook (#591, landed); pre-paint `/`→`/app` redirect
(`render/mod.rs:42`); tightening the other username-first SPA routes
(`/{username}`, `/{username}/tags/{tag}`) to `~`-only — a noted non-goal (spec
"Out of scope"). **Not filed as a separate issue** (it fixes no known bug —
those routes swallow no server URL — and filing would over-fragment); it is
recorded as a known limitation in the Task 6 ADR instead. _Reviewer: say so if
you'd rather it be a tracked follow-up._

**Tasks:**

1. Custom `~`-only route segment: define, wire onto the permalink route, delete
   the `reload()` escape + dead None branch. (host unit tests)
2. Publish/unpublish → `use_navigate`; `PostPage` refetch on in-place publish.
3. Delete `client::navigation`; add the xtask `no_full_reload_check` gate (+
   prove it bites).
4. E2E: no-reload assertions on publish/unpublish, new unpublish→`/drafts` test,
   prune the caveat.
5. Full gate + wasm-clippy + local e2e sweep.
6. ADR draft for the no-full-load invariant + `~`-namespace rule.

**Key risks / decisions:**

- **Interception reality (spec §"Framework facts"):** leptos intercepts _all_
  same-origin `<a>` clicks and renders `<Routes fallback>` on no-match — no
  server handoff. Accepting "Page not found." for a non-`~` 5-seg in-app nav is
  correct _only because_ the interview (Q1) established nothing in-app links
  there. Task 1 adds **no** reload/handoff machinery (a blanket one would
  infinite-loop against the server's shell fallback).
- **In-place publish staleness (spec AC#2):** the one real behavior trap; Task 2
  closes it with an explicit refetch, verified by a same-UTC-day e2e (Task 4).
- **Gate must bite in `cargo xtask check`:** it's a host source-scan (not
  clippy) precisely because clippy doesn't lint the wasm-gated
  `window().location()` calls in the default gate.

---

## Task 1: `~`-only permalink route segment

**Files:**

- Create: `web/src/route_segments.rs` — **crate root, not under `pages/`**:
  `pages` is `#[cfg(target_arch = "wasm32")]` (wasm-only), so a module under it
  is never host-compiled/tested. The matcher is pure `leptos_router` logic (no
  `web_sys`), so it lives ungated at the crate root and is host-tested there.
- Modify: `web/src/lib.rs` (add `pub mod route_segments;`, ungated)
- Modify: `web/src/pages/mod.rs` (`use crate::route_segments::TildeUsername;`;
  swap the permalink route's first segment `ParamSegment("username")` →
  `TildeUsername("username")`)
- Modify: `web/src/posts/component.rs:1205-1224` (`PostPage`'s `post` Resource
  error branch: delete the `reload()` escape; the `username: None` arm returns a
  client-side validation error like the invalid-`slug` arm)
- Test: in-file `#[cfg(test)] mod tests` in `web/src/route_segments.rs`

**Interfaces:**

- Consumes:
  `leptos_router::{ParamSegment, PossibleRouteMatch, PartialPathMatch, PathSegment}`
  (all re-exported at crate root via `pub use matching::*`).
- Produces: `pub struct TildeUsername(pub &'static str)` implementing
  `PossibleRouteMatch`. Its `test` matches a first segment **iff** the captured
  value starts with `~` (else `None`); capture keeps the `~` (so
  `parse_permalink_params`'s existing `strip_prefix('~')` is unchanged).

- [ ] **Step 1: Write the failing tests** in `web/src/pages/route_segments.rs`

```rust
#[cfg(test)]
mod tests {
    use super::TildeUsername;
    use leptos_router::PossibleRouteMatch;

    #[test]
    fn matches_tilde_username_and_captures_with_tilde() {
        let seg = TildeUsername("username");
        let m = seg.test("/~alice").expect("should match a ~-prefixed segment");
        assert_eq!(m.matched(), "/~alice");
        assert_eq!(m.remaining(), "");
        let params = m.params();
        assert_eq!(params[0], ("username".into(), "~alice".into()));
    }

    #[test]
    fn matches_tilde_username_with_trailing_path() {
        // The leading segment of a full permalink; the rest stays in `remaining`.
        let m = TildeUsername("username")
            .test("/~alice/2026/01/01/hello")
            .expect("should match the first segment");
        assert_eq!(m.matched(), "/~alice");
        assert_eq!(m.remaining(), "/2026/01/01/hello");
    }

    #[test]
    fn rejects_non_tilde_first_segment() {
        assert!(TildeUsername("username").test("/media").is_none());
        assert!(TildeUsername("username").test("/media/2026/01/01/x").is_none());
        assert!(TildeUsername("username").test("/app").is_none());
    }

    #[test]
    fn rejects_empty_and_root() {
        assert!(TildeUsername("username").test("").is_none());
        assert!(TildeUsername("username").test("/").is_none());
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p web route_segments` Expected: FAIL — `TildeUsername`
not defined.

- [ ] **Step 3: Implement `TildeUsername` against the tests**

Signature: `impl PossibleRouteMatch for TildeUsername`. Delegate matching to
`ParamSegment` (reusing its UTF-8-correct segment split), then reject the match
unless the captured value begins with `~`. Every branch (match-with-tilde,
trailing-path, non-tilde, empty/root) is pinned by Step 1, so the body follows
from the tests:

```rust
//! A permalink-route segment that matches a username **only** when it is `~`-prefixed,
//! so a non-`~` same-segment-count URL (e.g. `/media/2026/01/01/x`) no longer matches the
//! SPA permalink route and falls to `<Routes fallback>` instead of mounting `PostPage`
//! (#592). The server owns `~`-prefixed permalinks by a literal `~` route
//! (`server/src/projector/mod.rs`); this mirrors that ownership on the client. Capture
//! keeps the `~` so `crate::posts::parse_permalink_params` strips it exactly as before.

use leptos_router::{ParamSegment, PartialPathMatch, PathSegment, PossibleRouteMatch};

/// A `ParamSegment` that only matches a `~`-prefixed first segment. Field is the param key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TildeUsername(pub &'static str);

impl PossibleRouteMatch for TildeUsername {
    fn optional(&self) -> bool {
        false
    }

    fn test<'a>(&self, path: &'a str) -> Option<PartialPathMatch<'a>> {
        let matched = ParamSegment(self.0).test(path)?;
        // Inspect via `matched()` (borrows `&self`, returns `&'a str` tied to `path`) — NOT
        // `params()`, which takes `self` by value and would move `matched` before the
        // `then_some(matched)` below (E0382). The matched segment keeps its leading `/`.
        matched
            .matched()
            .trim_start_matches('/')
            .starts_with('~')
            .then_some(matched)
    }

    fn generate_path(&self, path: &mut Vec<PathSegment>) {
        path.push(PathSegment::Param(self.0.into()));
    }
}
```

- [ ] **Step 4: Wire it onto the permalink route** in `web/src/pages/mod.rs`

Add near the other imports: `mod route_segments;` and
`use route_segments::TildeUsername;` (and drop `ParamSegment` from the
`leptos_router::{…}` import only if it becomes unused elsewhere — it is still
used by other routes, so keep it). In the permalink `<Route>` (currently lines
150-159), change the first tuple element from `ParamSegment("username")` to
`TildeUsername("username")`; leave `year`/`month`/`day`/`slug` as
`ParamSegment`.

- [ ] **Step 5: Delete the `reload()` escape** in `web/src/posts/component.rs`

In `PostPage`'s `post` Resource (lines 1205-1224), the `username: None` arm
currently calls `client::navigation::reload()` then returns
`Err(WebError::validation("Invalid permalink"))`. With `TildeUsername` guarding
the route, a `None` username now means only a `~`-prefixed-but-unparseable
username (a malformed permalink, never a server URL), so it must 404 client-side
exactly like the invalid-`slug` arm below it. Delete the
`client::navigation::reload();` line and its explanatory comment; keep the
`return Err(...)`. Result:

```rust
let Some(username) = username else {
    // A `~`-prefixed but unparseable username is a malformed permalink (the route's
    // `TildeUsername` segment guarantees the `~`; a non-`~` URL never reaches here), so
    // 404 client-side without a round-trip — matching the invalid-slug arm below (#592).
    return Err(WebError::validation("Invalid permalink"));
};
```

- [ ] **Step 6: Run tests + host build, verify pass**

Run: `cargo nextest run -p web route_segments` Expected: PASS. Run:
`cargo check -p web` Expected: builds (the `client::navigation::reload` removal
leaves the `reload` primitive unused — that's cleaned up in Task 3; `check`
still passes).

- [ ] **Step 7: Commit**

```bash
git add web/src/pages/route_segments.rs web/src/pages/mod.rs web/src/posts/component.rs
git commit -m "feat(web): ~-only permalink route segment; drop the misroute reload (#592)"
```

Run `cargo xtask check` first (`jaunder-commit`).

---

## Task 2: Publish/unpublish → router navigation + in-place-publish refetch

**Files:**

- Modify: `web/src/posts/component.rs`
  - `PostCard` publish `Effect` (≈237-245): `replace` → `use_navigate` + fire
    `on_mutate`.
  - `PostPage` (≈1177-1268): add a refetch version signal, key `post` on it,
    pass `on_mutate` into its `PostCard`.
  - `PostPage` `on_unpublish` (≈1226-1228): `replace("/drafts")` →
    `use_navigate("/drafts")`.
  - `EditPostPage` publish `Effect` (≈1576-1582): `replace` → `use_navigate`.
- Test: behavior is wasm/leptos UI — verified by the Task 4 e2e + wasm clippy,
  not a host unit test. (No host-testable pure fn is introduced here.)

**Interfaces:**

- Consumes: `leptos_router::hooks::use_navigate`,
  `leptos_router::NavigateOptions`; `TildeUsername` route from Task 1; the
  existing `on_mutate: Option<Callback<()>>` prop on `PostCard`
  (component.rs:197).
- Produces: no new public surface; `client::navigation::{replace,reload}`
  becomes **zero-caller** after this task (consumed by Task 3's deletion).

- [ ] **Step 1: `PostCard` publish → navigate + on_mutate** (component.rs
      ≈233-245)

At the top of `PostCard`, obtain the navigator once:
`let navigate = use_navigate();`. Rewrite the publish `Effect` so that on
success it navigates client-side to the canonical permalink **and** runs
`on_mutate` (so a permalink page that stays put still refetches):

```rust
let navigate = use_navigate();
Effect::new(move |_| {
    if let Some(Ok(published)) = publish_action.value().get() {
        // Publishing can move the permalink (created_at- → published_at-based); navigate
        // there client-side. When it does NOT move (same-UTC-day publish → identical URL)
        // the navigate is a no-op, so also fire `on_mutate` to refetch the current page's
        // resource — otherwise the permalink page would keep showing the draft state (#592).
        navigate(&published.permalink, NavigateOptions::default());
        if let Some(cb) = on_mutate {
            cb.run(());
        }
    }
});
```

Add imports: `use leptos_router::hooks::use_navigate;` and
`use leptos_router::NavigateOptions;`.

- [ ] **Step 2: `PostPage` refetch wiring** (component.rs ≈1177-1268)

`PostPage`'s `post` Resource is currently keyed on route params alone. Add a
version signal and fold it into the key, and pass an `on_mutate` that bumps it
into the `PostCard` (currently only `on_unpublish` is passed, line ≈1264):

```rust
let refetch = RwSignal::new(0u32);
let post = Resource::new(
    move || (post_data(), refetch.get()),
    |((username, year, month, day, slug), _): ((Option<Username>, i32, u32, u32, Option<Slug>), u32)| async move {
        // ...existing body, unchanged (the `_` is the refetch tick)...
    },
);
let on_mutate = Callback::new(move |()| refetch.update(|v| *v += 1));
```

and in the `<PostCard .../>` invocation add `on_mutate=on_mutate` alongside the
existing `on_unpublish=on_unpublish`. (Adjust the closure's param pattern to
destructure the new `(params, tick)` tuple; the fetch body is otherwise
unchanged.)

- [ ] **Step 3: `PostPage` unpublish → navigate** (component.rs ≈1226-1228)

```rust
let navigate = use_navigate();
let on_unpublish = Callback::new(move |()| {
    navigate("/drafts", NavigateOptions::default());
});
```

(`DraftsPage` refetches on mount — its Resource keys on the publish/delete
action versions — so the just-unpublished post appears without extra
invalidation.)

- [ ] **Step 4: `EditPostPage` publish → navigate** (component.rs ≈1576-1582)

```rust
let navigate = use_navigate();
Effect::new(move |_| {
    if let Some(Ok(ref updated)) = update_post_action.value().get() {
        if updated.published_at.is_some() {
            navigate(&updated.permalink, NavigateOptions::default());
        }
    }
});
```

(Editor → permalink is always a route change → fresh `PostPage` mount → refetch;
no `on_mutate` needed here.)

- [ ] **Step 5: Build + wasm clippy**

Run: `cargo check -p web` Expected: builds; `client::navigation` now has zero
callers (an unused-import/dead-code note is fine — resolved in Task 3). Run:
`cargo clippy -p web --target wasm32-unknown-unknown --features csr -- -D warnings`
Expected: clean (catches wasm-only lints the host gate skips).

- [ ] **Step 6: Commit**

```bash
git add web/src/posts/component.rs
git commit -m "feat(web): publish/unpublish via router navigation, refetch on in-place publish (#592)"
```

Run `cargo xtask check` first.

---

## Task 3: Delete `client::navigation`; add the no-full-reload gate

**Files:**

- Delete: `client/src/navigation.rs`
- Modify: `client/src/lib.rs` (remove `pub mod navigation;` + its doc-comment,
  lines 21-23)
- Create: `xtask/src/steps/no_full_reload_check.rs`
- Modify: `xtask/src/lib.rs` (add `pub mod no_full_reload_check;` beside the
  other checks ≈line 21-25; add `steps::no_full_reload_check::run(&mut result);`
  in **both** the Fix block ≈290-298 and the Check block ≈321-329)
- Test: in-file `#[cfg(test)] mod tests` in `no_full_reload_check.rs`

**Interfaces:**

- Consumes: `crate::result::{CommandResult, StepResult}` (same as
  `proffered_secret_check`); scans roots `web/src`, `client/src`.
- Produces: `pub fn run(result: &mut CommandResult)`; pure
  `fn violations(source: &str) -> Vec<usize>` and
  `pub fn problems(scanned: &[(String, String)]) -> Option<String>`
  (host-unit-tested).

- [ ] **Step 1: Delete the module + declaration**

```bash
git rm client/src/navigation.rs
```

Remove `pub mod navigation;` and its preceding doc-comment (lines 21-23) from
`client/src/lib.rs`. Run: `cargo check -p web` and `cargo check -p client`
Expected: both build (Task 2 removed the last callers).

- [ ] **Step 2: Write the failing gate tests** in
      `xtask/src/steps/no_full_reload_check.rs`

```rust
#[cfg(test)]
mod tests {
    use super::{problems, violations};

    #[test]
    fn flags_location_replace_assign_reload_set_href() {
        assert_eq!(violations("    window().location().replace(&url);\n"), vec![1]);
        assert_eq!(violations("    window().location().assign(&url);\n"), vec![1]);
        assert_eq!(violations("    window().location().set_href(&url);\n"), vec![1]);
        assert_eq!(violations("    let _ = window().location().reload();\n"), vec![1]);
    }

    #[test]
    fn ignores_string_replace_and_use_location() {
        // String::replace — no `.location()` on the line.
        assert!(violations(r#"    let s = json.replace("a", "b");"#).is_empty());
        // leptos_router `use_location()` — a free fn, not a `.location()` chain.
        assert!(violations("    let loc = use_location();\n").is_empty());
    }

    #[test]
    fn ignores_comment_lines() {
        assert!(violations("    // window().location().replace(x) is forbidden\n").is_empty());
    }

    #[test]
    fn problems_reports_path_line_and_recovery() {
        let detail = problems(&[(
            "web/src/x.rs".to_string(),
            "    window().location().replace(&url);\n".to_string(),
        )])
        .expect("a problem");
        assert!(detail.contains("web/src/x.rs:1"));
        assert!(detail.contains("router navigation"));
    }

    #[test]
    fn clean_tree_reports_none() {
        assert_eq!(
            problems(&[("web/src/x.rs".to_string(), "    navigate(&url, opts);\n".to_string())]),
            None
        );
    }
}
```

- [ ] **Step 3: Run the tests, verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml no_full_reload`
Expected: FAIL — `violations`/`problems` not defined. (xtask is
workspace-excluded; use `--manifest-path xtask/Cargo.toml`.)

- [ ] **Step 4: Implement the check against the tests**

Model on `proffered_secret_check.rs` (pure `violations`/`problems` + a `run`
that scans roots and hard-fails a missing root). `violations` flags any
non-comment line where `.location()` is followed by a navigation method. Every
branch (each nav method, `String::replace`, `use_location`, comment, clean) is
pinned by Step 2:

```rust
//! The `no-full-reload` static check (#592): forbids raw `window.location` navigation in
//! `web/src` + `client/src`, so post-lifecycle flows stay client-side SPA navigation. It
//! is a host source-scan, not clippy `disallowed-methods`, because these are wasm-gated
//! call sites the default `cargo xtask check` clippy pass never lints. No allowlist: after
//! #592 there are zero legitimate callers (the pre-paint `/`→`/app` redirect is a JS
//! string, and leptos `use_location()` is a free fn — neither is a `.location()` chain).
//! Accepted limitation (as in `proffered_secret_check`): matching is per-line, so a chain
//! split across lines by the formatter could evade it — a guardrail, not an adversary.

use std::path::{Path, PathBuf};

use crate::result::{CommandResult, StepResult};

const NAV_METHODS: &[&str] = &[".replace(", ".assign(", ".reload(", ".set_href("];
const POLICED_ROOTS: &[&str] = &["web/src", "client/src"];

/// 1-based line numbers where a `.location()` receiver is navigated (`replace`/`assign`/
/// `reload`/`set_href`). Comment lines are skipped. Pure — unit-tested directly.
fn violations(source: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        if raw.trim_start().starts_with("//") {
            continue;
        }
        if let Some(loc) = raw.find(".location()") {
            if NAV_METHODS.iter().any(|m| raw[loc..].contains(m)) {
                out.push(i + 1);
            }
        }
    }
    out
}

/// Failure detail across scanned files, or `None` when clean. Pure — unit-tested.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(source) {
            lines.push(format!(
                "{path}:{ln}: raw `window.location` navigation is forbidden — use \
                 leptos_router `use_navigate()` for router navigation (#592, ADR)"
            ));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

// `rust_files` + `run` mirror proffered_secret_check.rs verbatim (recursive `.rs` collect;
// a missing root hard-fails; step name "no-full-reload"). Copy that structure.
```

Write `rust_files` and `run` identically to `proffered_secret_check.rs`
(§159-197), with the step name `"no-full-reload"` and `POLICED_ROOTS` above.

- [ ] **Step 5: Register + run tests**

Add `pub mod no_full_reload_check;` and the two `run` calls in
`xtask/src/lib.rs`. Run:
`cargo nextest run --manifest-path xtask/Cargo.toml no_full_reload` Expected:
PASS.

- [ ] **Step 6: Prove the gate bites**

Temporarily add `let _ = web_sys::window().unwrap().location().replace("/x");`
to a real line in some `web/src` file. Run: `cargo xtask check` Expected: FAIL,
with a `no-full-reload` step naming that file:line. Then **revert** the
deliberate violation and re-run: Run: `cargo xtask check` Expected: PASS (clean
tree; the `no-full-reload` step is green).

- [ ] **Step 7: Commit**

```bash
git add client/src/lib.rs xtask/src/steps/no_full_reload_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): forbid raw window.location nav; delete dead client::navigation (#592)"
```

(The `git rm` from Step 1 is already staged.) Run `cargo xtask check` first.

---

## Task 4: E2E — no-reload assertions + prune the caveat

**Files:**

- Modify: `end2end/tests/posts.spec.ts`
  - "draft lifecycle: create, view, edit, and publish" (≈260-341): wrap the
    publish-from-permalink step with the `__jaunderNoReload` sentinel
    (creation + publication in one test land on the same UTC day → identical
    permalink → exercises the in-place refetch, spec AC#2), asserting the
    sentinel survives.
  - Add a new test: **unpublish from a permalink navigates to /drafts without a
    reload** (spec AC#3) — no such test exists today.
  - "editing a published post freezes the slug" (≈718-742): delete the obsolete
    comment at lines 730-736 (`location.replace` + "waitForHydration races in
    Firefox because body[data-hydrated] is already set"); the `waitForSelector`
    assertion stays.
- Test framework: Playwright; run via the host runner.

**Interfaces:**

- Consumes: the `__jaunderNoReload` sentinel pattern from `auth.spec.ts:87-108`
  (`page.evaluate` sets `window.__jaunderNoReload = true`; after the flow,
  assert it is still `true`); existing helpers (`goto`, `click`,
  `waitForSelector`, `SEL`).

- [ ] **Step 1: Sentinel-guard the publish-from-permalink step** (posts.spec.ts
      ≈327-341)

Before the `Publish` click,
`await page.evaluate(() => { (window as ...).__jaunderNoReload = true; });`.
After landing on the published permalink (draft banner gone), assert the
sentinel survived (mirror `auth.spec.ts:102-107`). This test creates then
immediately publishes → same UTC day → identical permalink, so a passing
assertion proves the in-place refetch (AC#2) with no document reload.

- [ ] **Step 2: Add the unpublish→/drafts no-reload test**

A new `test(...)` that: creates + publishes a post, opens its permalink, sets
the sentinel, clicks the permalink `PostCard`'s `Unpublish`,
`await page.waitForURL(\`${BASE_URL}/drafts\`)`, asserts the just-unpublished post is listed on `/drafts`, and asserts the sentinel survived (AC#3). (Use the same `SEL`/action selectors the delete/publish tests use for the `.j-post-acts`
buttons.)

- [ ] **Step 3: Prune the caveat + assert editor-publish is reload-free**
      (posts.spec.ts 718-742, AC4)

Delete the comment block at 730-736 referencing `location.replace()` and the
`body[data-hydrated]`/Firefox-`waitForHydration` race; replace with a one-line
note that publish now navigates client-side and `waitForSelector(".j-tag-list")`
waits for the destination permalink. This test edits an already-published post
and saves (`publish=true`) → the `EditPostPage` publish→navigate path — so also
set `window.__jaunderNoReload = true` before the save click and assert it
survived after landing on the permalink (covers AC4's no-reload behavior, which
no other test observes). (Grep `helpers.ts`/`hydration.ts` for any remaining
`location.replace`/`waitForURL`-after-reload caveat and prune those too; spec §5
note — the issue's cited `end2end/CLAUDE.md` is absent from this worktree.)

- [ ] **Step 4: Run the affected specs locally**

Run: `cargo xtask e2e-local posts` (host runner, ~3 min; auto-seeds
testoperator). Expected: PASS (chromium+firefox × the local backend). Full
`{sqlite,postgres}×{chromium,firefox}` matrix runs in CI.

- [ ] **Step 5: Commit**

```bash
git add end2end/tests/posts.spec.ts
# plus end2end/tests/helpers.ts / hydration.ts if pruned
git commit -m "test(e2e): no-reload publish/unpublish, unpublish→drafts, drop reload caveat (#592)"
```

Run `cargo xtask check` first.

---

## Task 5: Full local gate sweep

**Files:** none (verification task).

- [ ] **Step 1: Wasm clippy** —
      `cargo clippy -p web --target wasm32-unknown-unknown --features csr -- -D warnings`
      → clean.
- [ ] **Step 2: Full validate (no e2e)** — `cargo xtask validate --no-e2e` →
      green (static + clippy + coverage + the new `no-full-reload` check).
      Foreground, generous timeout.
- [ ] **Step 3: E2E** — `cargo xtask e2e-local posts` green locally; rely on CI
      for the full four-combo matrix at PR time.
- [ ] **Step 4:** No commit (verification only). Fix + fold any failure back
      into the owning task's commit (`jaunder-commit` fixup discipline).

---

## Task 6: ADR — no-full-load invariant + `~`-namespace rule

**Files:**

- Create: `docs/adr/drafts/<slug>.md` (numberless draft;
  `cargo xtask adr promote` numbers it and updates the README table at ship —
  `jaunder-adr` flow).

**Interfaces:** consumes the `jaunder-adr` draft convention; produced draft is
promoted by `jaunder-ship`.

- [ ] **Step 1: Write the ADR draft** (via `jaunder-adr`)

Record: (1) **No in-app full document loads** — within a live SPA session all
navigation is client-side `leptos_router`; raw `window.location` navigation is
forbidden in `web/src`/`client/src` and enforced by the xtask `no-full-reload`
gate; the only document loads left are cold entry, server-owned non-HTML
resources, and the pre-paint `/`→`/app` redirect. (2) **`~`-namespace route
ownership** — SPA user-URL routes are `~`-prefixed, matched by `TildeUsername`,
mirroring the server's literal-`~` projector routes; a non-`~` same-segment URL
falls to `<Routes fallback>`. Note the **known limitation**: only the 5-segment
permalink route is tightened (where the reload bug lived); the other
username-first routes are left as `ParamSegment` (they swallow no server URL) —
a possible future tightening, not a tracked issue. Link the interview Q1
link-inventory dependency (spec AC#6). Relate to ADR-0044 (CSR/SSR-removal
lineage) and #591.

- [ ] **Step 2: Commit**

```bash
git add docs/adr/drafts/<slug>.md
git commit -m "docs(adr): no-full-load SPA navigation invariant + ~-namespace rule (#592)"
```

Run `cargo xtask check` first (the `adr_check` step validates draft shape).

---

## Self-review

- **Spec coverage:** AC1 → Task 2 Step 1 (+Task 4 Step 1 sentinel); AC2 → Task 2
  Step 2 (+Task 4 Step 1 same-day in-place test); AC3 → Task 2 Step 3 (+Task 4
  Step 2 new test); AC4 → Task 2 Step 4 (+Task 4 Step 3 editor-publish
  sentinel); AC5 → Task 1 tests; AC6 → Task 1 Steps 4-5; AC7 → Task 3 Step 1;
  AC8 → Task 3 Step 6; AC9 → Task 4/5; AC10 → Task 4 Step 3; AC11 → Task 6. All
  covered, each with an observable assertion.
- **Type consistency:** `TildeUsername(&'static str)`,
  `on_mutate: Callback<()>`, `use_navigate`/`NavigateOptions`,
  `violations`/`problems`/`run` — names match across tasks and mirror
  `proffered_secret_check`'s shape.
- **Separable concern:** the other-username-routes tightening is recorded (ADR
  known limitation), not filed, to avoid over-fragmenting; flagged for the
  reviewer in the Review header.
