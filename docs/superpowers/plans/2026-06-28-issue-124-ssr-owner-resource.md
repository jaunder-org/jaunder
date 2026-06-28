# SSR Owner Capture at the Resource Layer — Implementation Plan (issue #124)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop server functions invoked from SSR `Resource` fetchers from panicking on `expect_context` by capturing the reactive owner at fetcher-invocation in one sanctioned constructor, `server_resource`, and forbidding raw `Resource::new` in `web`.

**Architecture:** Add `server_resource(source, fetcher)` that wraps the fetcher's future in `ScopedFuture::new_untracked` (capturing the live owner, holding a strong ref across polls — #89's mechanism applied at the layer where the owner is still alive). Migrate all `web/src` `Resource::new` sites to it; a static guard fails the gate on any raw `Resource::new`. Handlers are unchanged.

**Tech Stack:** Rust, Leptos 0.8.19 (`leptos::prelude::Resource`, `leptos::reactive::computed::ScopedFuture`), the `web` crate.

**Spec:** `docs/superpowers/specs/2026-06-28-issue-124-ssr-owner-resource.md`. **ADR:** `docs/adr/0016-…md` (second addendum, #124 — already written this cycle).

## Global Constraints

- **`server_resource` is the only sanctioned `Resource` constructor in `web`.** Raw `leptos::prelude::Resource::new` is banned outside its definition (static guard).
- **Zero per-handler change:** server fns keep `boundary!` + `expect_context` (#89 covers the `/api` path). Only the `Resource` layer changes.
- **Owner capture mechanism:** `ScopedFuture::new_untracked(fetcher(s))`, constructed inside the fetcher (a current owner — the resource's reactive owner — is always present there, so no `Owner::current().is_some()` guard is needed, unlike `server_boundary`).
- **`Action`/`ServerAction` is out of scope** — its future runs only on client dispatch (post-hydration, owner live); confirmed no SSR-detached dispatch.
- **No `Co-Authored-By`.** Worktree only (`worktree-issue-124-userstorage-context`); review vs `wt-base-issue-124`. Per-task gate `cargo xtask check`; e2e is ship-only.

---

### Task 1: `server_resource` constructor + deterministic owner-lifetime test

**Files:**
- Modify: `web/src/error.rs` (add `scoped_fetcher_future` + `server_resource` near `server_boundary`; add a test to the `owner_lifetime` module at ~`error.rs:875`)
- Modify: `web/src/lib.rs` (re-export: `pub use error::server_resource;`)

**Interfaces:**
- Produces: `web::server_resource<T, S, Fut>(source, fetcher) -> leptos::prelude::Resource<T>` (re-exported at crate root as `crate::server_resource`).

- [ ] **Step 1: Write the failing test** in the `owner_lifetime` module (`web/src/error.rs`, inside `mod owner_lifetime`, reusing its existing `Marker`/`YieldOnce`/`step` helpers):

```rust
    /// #124: a Resource fetcher's future, wrapped by `scoped_fetcher_future`
    /// (what `server_resource` applies), keeps its context across an owner drop —
    /// even when the owner's strong ref is gone before the future is first polled,
    /// the SSR-resource case `server_boundary` could not cover.
    #[test]
    fn scoped_fetcher_future_keeps_context_across_owner_drop() {
        let owner = Owner::new();
        owner.set();
        provide_context(Marker(7));

        // Build the future exactly as `server_resource` does, then drop our owner
        // ref *before the first poll* (the SSR-resource detachment).
        let mut fut = Box::pin(crate::error::scoped_fetcher_future(async {
            let pre = use_context::<Marker>();
            YieldOnce(false).await;
            let post = use_context::<Marker>();
            (pre, post)
        }));
        drop(owner);

        let mut cx = Context::from_waker(Waker::noop());
        assert!(step(fut.as_mut().poll(&mut cx)).is_none());
        let (pre, post) =
            step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
        assert_eq!(pre, Some(Marker(7)), "context present at first (detached) poll");
        assert_eq!(post, Some(Marker(7)), "context survives the await");
    }
```

- [ ] **Step 2: Run — verify it FAILS to compile** (`scoped_fetcher_future` undefined).

Run: `cargo test -p web --lib owner_lifetime::scoped_fetcher_future_keeps_context_across_owner_drop`
Expected: FAIL — `cannot find function scoped_fetcher_future`.

- [ ] **Step 3: Implement** in `web/src/error.rs` (place near `server_boundary`, NOT `#[cfg(feature = "ssr")]`-gated — resources run on client + server):

```rust
/// Wrap a `Resource` fetcher's future so the reactive owner — captured here, while it
/// is still current (the resource's own owner) — is held by a *strong* ref and
/// re-applied on every poll. This keeps server-fn context (storage trait objects and
/// request `Parts`) alive even when the future is later polled on a worker thread
/// detached from the owner. The owner is live only at fetcher invocation; an
/// `async fn` body has no synchronous prologue, so this cannot live in the handler.
/// `new_untracked` so context reads don't create spurious reactive subscriptions.
/// See the ADR-0016 #124 addendum. (Extends #89's `server_boundary` mechanism to the
/// Resource layer.)
fn scoped_fetcher_future<Fut>(
    fut: Fut,
) -> leptos::reactive::computed::ScopedFuture<Fut>
where
    Fut: std::future::Future,
{
    leptos::reactive::computed::ScopedFuture::new_untracked(fut)
}

/// The sanctioned way to create a `Resource` in `web`: identical to
/// `leptos::prelude::Resource::new`, but wraps the fetcher's future via
/// [`scoped_fetcher_future`] so server-fn context survives SSR polling on a worker
/// thread (issue #124). Raw `Resource::new` is banned in `web` (static guard).
pub fn server_resource<T, S, Fut>(
    source: impl Fn() -> S + Send + Sync + 'static,
    fetcher: impl Fn(S) -> Fut + Send + Sync + 'static,
) -> leptos::prelude::Resource<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    S: PartialEq + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    #[allow(clippy::disallowed_methods)] // the one sanctioned Resource::new (issue #124)
    leptos::prelude::Resource::new(source, move |s| scoped_fetcher_future(fetcher(s)))
}
```

Add the re-export to `web/src/lib.rs` (near `pub use pages::App;`): `pub use error::server_resource;`

(If `ScopedFuture<Fut>` is not `Send` when `Fut: Send` and the compiler rejects the `Resource::new` `Fut: Send` bound, add `where Fut: Send` is already present; if a `ScopedFuture: Send` issue arises, box the future: `move |s| scoped_fetcher_future(Box::pin(fetcher(s)))` — but try the direct form first.)

- [ ] **Step 4: Run — verify PASS.**

Run: `cargo test -p web --lib owner_lifetime`
Expected: PASS (the new test + the existing #89 tests).

- [ ] **Step 5: Commit.**

```bash
git add web/src/error.rs web/src/lib.rs
git commit -m "feat(web): add server_resource owner-capturing Resource constructor (#124)"
```

---

### Task 2: Migrate all `web/src` `Resource::new` sites to `server_resource`

Mechanical and uniform: at each site replace the constructor `Resource::new(` → `server_resource(` (arguments unchanged — bare-fn-ref fetchers like `get_post_preview` work because `server_resource` calls `fetcher(s)` then wraps), and add `use crate::server_resource;` to the file's imports (pages glob only `leptos::prelude::*`, which does not bring in `server_resource`). One commit (must compile as a whole).

**Files & sites** (31 total; each `Resource::new(` → `server_resource(`):
- `web/src/pages/posts.rs`: lines 30, 136, 228, 301, 462, 600, 602, 840, 1011, 1165
- `web/src/pages/ui.rs`: 185, 519, 651, 1059, 1060
- `web/src/pages/audiences.rs`: 45, 47, 158
- `web/src/pages/media.rs`: 34, 39
- `web/src/pages/backup.rs`: 11
- `web/src/pages/site.rs`: 11
- `web/src/pages/sessions.rs`: 11
- `web/src/pages/auth.rs`: 11
- `web/src/pages/invites.rs`: 13, 14
- `web/src/pages/home.rs`: 39
- `web/src/pages/email.rs`: 13, 67
- `web/src/pages/profile.rs`: 11, 70

**Interfaces:** Consumes `crate::server_resource` (Task 1).

- [ ] **Step 1: Add the import** to each of the 14 files above — `use crate::server_resource;` (next to the existing `use leptos::prelude::*;`).

- [ ] **Step 2: Replace each `Resource::new(` with `server_resource(`** at the listed sites (arguments unchanged). The 3 bare-fn-ref sites become `server_resource(post_id_param, get_post_preview)` (posts.rs:600), `server_resource(post_id_param, post_audience_selection)` (posts.rs:602), `server_resource(token, verify_email)` (email.rs:67).

- [ ] **Step 3: Compile + clippy.**

Run: `cargo xtask check --no-test`
Expected: PASS — compiles; no `Resource::new` left in `web/src` except the sanctioned one in `error.rs`. Fix any `Send`/bound mismatch surfaced (see Task 1 Step 3 note about boxing).

- [ ] **Step 4: Run web tests** (behavior unchanged).

Run: `cargo test -p web`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add web/src/pages
git commit -m "refactor(web): route all Resource creation through server_resource (#124)"
```

---

### Task 3: Static guard — ban raw `Resource::new` in `web/src`

Make a forgotten wrapper a gate failure. **Primary:** a scanning unit test (deterministic, no clippy-version risk). **Alternative considered:** clippy `disallowed-methods` on `leptos::prelude::Resource::new` — viable only if clippy can target the inherent generic method; if Step 2 verifies it fires, that may replace the scan test, but the scan test is the committed guard.

**Files:**
- Create: `web/src/resource_guard.rs` (a `#[cfg(test)]`-only scan test module) — or add the test to an existing test module in `web/src/lib.rs`.
- Modify: `web/src/lib.rs` (`#[cfg(test)] mod resource_guard;` if a new file).

**Interfaces:** none (test-only).

- [ ] **Step 1: Write the guard test.**

```rust
//! Static guard (#124): `server_resource` is the only sanctioned Resource
//! constructor in web; a raw `Resource::new` bypasses the owner-capture and
//! reintroduces the SSR `expect_context` panic.
#[cfg(test)]
mod guard {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("read_dir web/src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn no_raw_resource_new_in_web() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // The single sanctioned use lives in the server_resource definition.
        let sanctioned = src.join("error.rs");
        let mut files = Vec::new();
        rs_files(&src, &mut files);

        let mut offenders = Vec::new();
        for file in files {
            if file == sanctioned {
                continue;
            }
            let text = fs::read_to_string(&file).expect("read source");
            for (i, line) in text.lines().enumerate() {
                if line.contains("Resource::new(") {
                    offenders.push(format!("{}:{}", file.display(), i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "raw `Resource::new(` is banned in web (issue #124) — use `server_resource`:\n{}",
            offenders.join("\n"),
        );
    }
}
```

(Wire it in: this module can live directly in `web/src/lib.rs` under `#[cfg(test)]`, or as `#[cfg(test)] mod resource_guard;` pointing at a new file. Note: the guard scans for the literal `Resource::new(`; this plan/spec text and the test's own message use `Resource::new` without the `(` or live outside `src`, so they don't self-trip. The `error.rs` sanctioned line is excluded by skipping that file.)

- [ ] **Step 2: Verify the guard catches a violation.** Temporarily add `let _ = leptos::prelude::Resource::new(|| (), |_| async {});` to a page file, run the test, confirm it FAILS naming that line, then remove it.

Run: `cargo test -p web --lib guard::no_raw_resource_new_in_web`
Expected: FAIL with the temp line listed; PASS after removing it.

- [ ] **Step 3 (optional): probe clippy `disallowed-methods`.** Add to `clippy.toml`:

```toml
disallowed-methods = [
  { path = "leptos::prelude::Resource::new", reason = "use web::server_resource (issue #124): raw Resource::new drops the SSR reactive owner" },
]
```

Run: `cargo xtask check --no-test`. If clippy flags raw `Resource::new` at a temp call site (and the `#[allow]` in `server_resource` suppresses the definition), keep it as a belt-and-suspenders alongside the scan test. If it does NOT bind to the inherent method, revert the `clippy.toml` change and rely on the scan test alone.

- [ ] **Step 4: Run the gate.**

Run: `cargo xtask check --no-test`
Expected: PASS (guard green, since Task 2 removed all raw uses).

- [ ] **Step 5: Commit.**

```bash
git add web/src/lib.rs web/src/resource_guard.rs clippy.toml
git commit -m "test(web): static guard banning raw Resource::new outside server_resource (#124)"
```

---

### Task 4: Final verification (incl. e2e zero-panic)

**Files:** none.

- [ ] **Step 1: Full validate (with e2e on both backends)** — confirms the `/` SSR no longer panics under the #93 zero-panic gate.

Run: `nix develop .#ci -c cargo xtask validate`
Expected: exit 0; `nix-e2e` green (no `reactive_graph` panic in the journal); coverage clean. (The deterministic Task 1 test is the primary proof; e2e is corroboration — the panic was flaky, so a green e2e plus the unit test + static guard is the confidence, not e2e alone.)

- [ ] **Step 2: Review the branch diff vs the fork point.**

Run: `git diff wt-base-issue-124..HEAD --stat`
Expected: only `web/src/error.rs`, `web/src/lib.rs`, `web/src/pages/*` (the 14 files), the guard module, optionally `clippy.toml`, and the already-committed `docs/**` (spec + ADR addendum). No stray files; handlers (server fns) untouched.

---

## Self-Review

**Spec coverage:** `server_resource` constructor (Task 1) · `ScopedFuture::new_untracked` at fetcher (Task 1) · uniform migration of all 31 sites (Task 2) · static guard banning raw `Resource::new` (Task 3) · deterministic owner-lifetime proof (Task 1 test) · `Action` out of scope (Global Constraints; confirmed client-only dispatch) · ADR-0016 addendum (written in brainstorming) · e2e zero-panic corroboration (Task 4). All spec sections map to a task.

**Placeholder scan:** the only conditional is Task 3 Step 3 (clippy probe), which has a concrete keep/revert instruction and a committed primary (scan test) — not a placeholder. Task 1 Step 3's `Send`-boxing note is a concrete fallback, not vague.

**Type consistency:** `server_resource<T, S, Fut>` / `scoped_fetcher_future<Fut>` names and signatures are used identically in Task 1 (def), Task 1 test (`crate::error::scoped_fetcher_future`), and Task 2 (call sites use `server_resource`). `Resource<T>` return type matches the migrated sites' default-codec usage.
