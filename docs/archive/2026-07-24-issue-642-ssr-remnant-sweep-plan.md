# Plan — #642: SSR-remnant mechanical sweep

Spec:
[`docs/superpowers/specs/2026-07-24-issue-642-ssr-remnant-sweep.md`](../specs/2026-07-24-issue-642-ssr-remnant-sweep.md)
(the "what/why"; this plan is the "how"). Issue: jaunder-org/jaunder#642.

## Review header

**Goal.** Delete SSR-era dead machinery and reword SSR-as-current comments left
by the CSR migration. No behavior change (one sanctioned exception: AC-6
password_reset read may flip untracked→tracked if the CSR check warrants).

**Scope — in:** AC-1..AC-6 from the spec. **Out:** §3 (`read_signal!`, already
gone via #643), the four #643 carve-out sites (already converted), everything on
the spec's "Explicitly NOT remnants" list. No separable concerns to file — the
sweep is self-contained; #643 is closed.

**Tasks (one commit each):**

1. AC-1 — remove `LeptosOptions` threading (server src + test mirrors), then
   trim server's `leptos` dep iff it still compiles.
2. AC-2 — remove SSR-era `LocalSet` test wrappers; reword `router.rs` header;
   drop the degenerate `home_response_contains_app_content`.
3. AC-3 — the two PostCard effects → `Effect::new`.
4. AC-4 — drop `leptos_meta/ssr` + `leptos_router/ssr` iff gate + wasm clippy
   green.
5. AC-5 — remove `recursion_limit` iff the coverage-instrumented build stays
   green.
6. AC-6 — reword the 13 reword-only stale SSR/hydration comments.
7. AC-6 (carved-out) — investigate + resolve password_reset's untracked read.

**Key risks/decisions.**

- Tasks 1+2+5 share `server/` files (`lib.rs`, the two test files) — sequenced
  1→2, and 5 after 1, so each compiles standalone.
- AC-1/AC-4/AC-5 are compile-gated **decisions**: the task's done-state is
  "removed and green" OR "kept + one-line note on the issue naming what needs
  it." Never guess — let the gate decide.
- AC-4/AC-5 need the wasm-clippy / coverage-Nix gates that host `check` skips —
  those runs are the verification, not an afterthought.

**For agentic workers.** Drive with **`jaunder-iterate`**; delegate a task to a
subagent via **`jaunder-dispatch`** where useful (Task 6's multi-file reword is
the natural candidate). Tick checkboxes in real time.

## Global constraints

- Server crate package is `jaunder`; per-task server checks use
  `cargo nextest run -p jaunder`. Web checks add
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`.
- Commit only after `cargo xtask check` is clean (pre-commit hook runs it); **no
  `Co-Authored-By` trailer**; request review before merge.
- Re-locate every site by content (line numbers are post-#643 audit snapshots;
  `main` moves). Preserve any still-real constraint when rewording (e.g.
  wasm-cleanliness) — change only the vehicle.
- No new `cov:ignore` (this is deletion + reword).

---

## Task 1 — AC-1: Remove dead `LeptosOptions` threading

**Files:**

- `server/src/commands.rs` — delete the
  `LeptosOptions::builder().output_name("jaunder") .env(...).site_addr(bind)`
  block (~540-543); drop the now-unused
  `use leptos::prelude::{Env, LeptosOptions};` (~23) — keep any other
  `leptos::prelude` import the file still needs (split the use if it imported
  more). Update `prepare_server` so it no longer builds/passes options.
- `server/src/lib.rs` — remove the `leptos_options: LeptosOptions` param from
  `create_router` (~35) and the `.with_state(leptos_options)` call (~134). If
  the router now needs `.with_state(())` or none, follow what axum requires.
  Drop the `LeptosOptions` import if unused.
- `server/src/projector/mod.rs` — reword the doc (~53) that references composing
  onto "the live `Router<LeptosOptions>`" to the current bare-`Router` reality.
- `server/tests/helpers/mod.rs` — delete `test_options()` (~93-94) and its
  `LeptosOptions` import (~14); remove its call sites (~344, 459, 673) so
  callers invoke `create_router` without options.
- `server/tests/web/router.rs` — remove the inline `LeptosOptions::builder()`
  (~57) and the import (~18); fix the `create_router` call.
- `server/tests/misc/commands.rs` — remove the inline `LeptosOptions::builder()`
  (~146) and the import (~16); fix the call.

**Then (empirical dep trim):** with the above compiling, try trimming
`server/Cargo.toml`'s `leptos` dep (~27) and dev-dep (~73) — narrow features or
remove entirely if the crate still uses nothing from `leptos` beyond what
remains (`provide_context` likely keeps it). The `ssr` feature is expected to
stay (server-fn body gate). **Done-state:** whatever compiles green, OR
unchanged with a one-line note on the issue.

**Verify:** `cargo nextest run -p jaunder` (server unit + integration) PASS;
`cargo xtask check` green. Grep proof: `rg 'LeptosOptions|test_options' server/`
returns nothing.

**Commit:** `refactor(server): drop dead LeptosOptions threading (#642)`

## Task 2 — AC-2: Remove SSR-era `LocalSet` test wrappers

**Files:**

- `server/tests/misc/commands.rs` — unwrap the `LocalSet` in
  `after_init_server_responds_to_health_check` and
  `prepare_server_binds_and_builds_serving_router`; delete the SSR-as-current
  comments (~155-157, ~201). The test body runs directly on the `#[tokio::test]`
  runtime.
- `server/tests/web/router.rs` — unwrap the `LocalSet` in every test body (~31,
  61, 98, 125, …); reword the module header (~10) so it no longer claims the
  tests assert "routing / SSR"; **delete** `home_response_contains_app_content`
  (~96) — its `html.contains("Jaunder")` duplicates the adjacent shell-embed
  test.

**Verify:** `cargo nextest run -p jaunder` PASS (fewer tests: the dropped one
gone). Grep proof:
`rg 'LocalSet|home_response_contains_app_content|routing / SSR' server/tests`
returns nothing.

**Commit:**
`test(server): drop SSR-era LocalSet wrappers and degenerate shell test (#642)`

## Task 3 — AC-3: PostCard effects → `Effect::new`

**Files:**

- `web/src/posts/component.rs` — the two PostCard effects (~228 delete, ~236
  unpublish): `Effect::new_isomorphic(...)` → `Effect::new(...)`. The sibling
  comment (~244-247) is already CSR-worded; leave it. (The EditPostPage comment
  at ~1173-1176 is AC-6, Task 6 — leave here.)

**Verify:** `cargo check -p web` +
`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` green. Grep
proof: `rg 'new_isomorphic' web/src` returns nothing (the last comment reference
dies in Task 6).

**Commit:**
`refactor(web): PostCard effects use Effect::new, not _isomorphic (#642)`

## Task 4 — AC-4: Feature-flag trim (compile-checked)

**Files:**

- `web/Cargo.toml` — in the `server` feature, drop `"leptos_meta/ssr"` (~67) and
  `"leptos_router/ssr"` (~68). **Keep** `"leptos/ssr"` (~66) and
  `"dep:leptos_axum"` (~69).

**Verify:** `cargo xtask check` AND
`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` green.
**Done-state:** dropped + green, OR reverted with a one-line note on the issue
naming what needs them.

**Commit:**
`build(web): drop leptos_meta/router ssr features unused post-CSR (#642)`

## Task 5 — AC-5: `recursion_limit` re-check

**Files:**

- `server/src/lib.rs` — try removing `#![recursion_limit = "512"]` (~4) and its
  comment (~1-3).

**Verify:** `cargo xtask check` green **including** the coverage-instrumented
Nix build (that build is where the limit historically bit). **Done-state:**
removed + green, OR keep the attribute and rewrite the comment to state the
_current_ reason (not "monomorphizing `web::App`'s route tuple").

**Commit (either):** `refactor(server): drop stale recursion_limit (#642)` or
`docs(server): recursion_limit comment states current reason (#642)`

## Task 6 — AC-6: Stale-comment reword sweep (reword-only)

Reword each so it no longer asserts retired SSR/hydration mechanics as live;
preserve still-real constraints. Candidate to **dispatch** (multi-file, purely
textual) — brief must restate house rules and forbid `ctx_*`.

**Files (spec AC-6 list, minus password_reset which is Task 7):**

- `common/src/mailer.rs:5`, `server/src/mailer/mod.rs:5-6` — "compiled to
  WebAssembly via `web`/`hydrate`" → keep wasm-cleanliness, fix vehicle (entry
  crate is `csr`, no `hydrate` build).
- `host/src/metrics.rs:1` — "shared by `web` (SSR), …".
- `web/src/posts/api.rs:35`, `web/src/auth/api.rs:16-17` — "SSR-only imports" /
  "crate-level SSR dependencies" → they gate on `feature="server"` for
  `#[server]` bodies, not SSR.
- `web/src/app/component.rs:48-51` — "leaks … into the hydrated DOM" → CSR
  mounts a DOM, no hydration.
- `web/src/route_segments.rs:88` — "so SSR route-list / link generation is
  unaffected" → client-side link generation is what matters.
- `web/src/app/render.rs:96-104` — `render_discovery` doc "the reactive SSR
  render did" → the CSR components are the spec now.
- `web/src/timeline/component.rs:52-54` — "post-hydration `Effect`".
- `web/src/avatar/component.rs:16-17` — "so SSR and reactive output coincide" →
  the twin is the server _projector_.
- `web/src/cockpit/component.rs:47` — "Client-only effect …" (every effect is
  now).
- `web/src/posts/component.rs:1173-1176` — "never fires server-side" / "would
  needlessly schedule on the server".
- `server/src/projector/mod.rs:403-408` — "or hydration 404s on projector
  routes" → it's a CSR boot/mount.
- `server/tests/atompub/atompub_rsd.rs:94` — "Rendering the user page
  (server-side) hoists the EditURI …" → discovery links come from the
  shell/projector path now (#198).

**Verify:** `cargo xtask check` green (doc comments still lint/fmt);
`rg -i 'hydrat|SSR' common/src server/src host/src web/src` returns only the
known residue named in the spec's AC-6 observable.

**Commit:** `docs: reword SSR-as-current comments left by CSR migration (#642)`

## Task 7 — AC-6 carve-out: password_reset untracked read

**Investigate:** `web/src/password_reset/component.rs:71-75` currently reads the
query map via `.with_untracked(...)`, justified by a hydration race. Under CSR
there is no hydration pass. Determine whether the untracked read still serves a
CSR purpose (compare `registration/component.rs:44`, already CSR-worded, and how
the router populates the query map on CSR mount).

- If the untracked read is still wanted → keep it, reword the comment to the
  real CSR reason.
- If not → switch to a tracked read matching the registration twin; reword.

**Verify:** `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
green. **If the read semantics changed:** `cargo xtask e2e-local password_reset`
(positional filter → `password_reset.spec.ts`; token consumption still works).
No new `cov:ignore`.

**Commit:** `docs(web): password_reset read states real CSR reason (#642)` (or
`fix(web): …` if the read flips tracked).

---

## Final conformance gate (before ship)

- `cargo xtask validate --no-e2e` green (verify-only; runs the coverage Nix
  build).
- `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` green.
- `cargo xtask e2e-local posts` — PostCard delete/unpublish flows unregressed.
- Every AC observable from the spec satisfied;
  `git diff wt-base-issue-642..HEAD` shows only deletions, mechanical
  substitutions, and comment rewordings (plus the one sanctioned password_reset
  read change, if it landed).
