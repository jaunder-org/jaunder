# Spec: `#[macros::server]` — one attribute derives the wire path, the span name, and the error boundary

- Date: 2026-07-30
- Issues: **#714** (primary), **#722**, **#698**
- Base: `main` at `9f3631ee` (PR #724 / issue #684 merged 19:58). All line
  citations below are re-verified against that commit.
- Revision 4 — folds in a cold spec review (4 blockers, 8 should-fix, 3 nits), a
  resolution audit (9 further items), and a regression check (6 more). Revision
  4 adds the placement rule and the `timeline` move.

## Problem

Every `#[server]` fn in `web/src` carries three strings that restate what the
source already says:

| literal                              | value                    | maintained by                    |
| ------------------------------------ | ------------------------ | -------------------------------- |
| `#[server(endpoint = "…")]`          | `/<vertical>/<ident>`    | `server-fn-endpoint` gate (#684) |
| `#[tracing::instrument(name = "…")]` | `web.<vertical>.<ident>` | `server-fn-tracing` gate (#511)  |
| `boundary!("…", { … })`              | the fn ident             | **nothing** (#714)               |

Two are machine-written and gate-enforced; the third is hand-written and
unguarded — what #714 was filed about. #722 asks the family question: when a
string duplicates what the source already encodes, do you derive it or
generate-and-gate it?

Measured facts that reshape the answer (verified against source, not assumed):

- **The `boundary!` label has no consumer.** Its sole use is a `server_fn` field
  on one `tracing` event (`host/src/error.rs:313`, parameter at `:302`). Nothing
  in `end2end/`, `server/tests/`, `elisp/`, `xtask/`, or the docs reads it.
- **That event already names the function, better.** It is emitted inside
  `#[tracing::instrument(name = "web.audiences.create")]`, and _both_ configured
  sinks render span context unconditionally — JSON
  (`display_current_span`/`display_span_list` default `true`,
  `tracing-subscriber-0.3.23/src/fmt/format/json.rs:334-342`) and plain text
  (`Format<Full>` walks `ctx.event_scope()` at `format/mod.rs:985-1000`, no flag
  guarding it). The label duplicates the span name while being strictly less
  precise: after #684 the bare `create` is ambiguous across
  audiences/posts/invites.
- **A proc macro can derive all three.** `proc_macro::Span::file()` is stable
  since 1.88.0 (`proc_macro/src/lib.rs:570`). `rust-toolchain.toml:2` is
  `channel = "stable"` — a floating channel, **not** a pin; the Nix toolchain
  must be confirmed too (R2).
- **The argument surface is tiny, with no multi-line hole.** Across all 55
  sites: `#[server]` takes `endpoint` (×55, derivable) and `input` (×4);
  `#[tracing::instrument]` takes `name` (×55, derivable), `skip_all` (×10),
  `skip(…)` (×6). No `fields`, `level`, `err`, or `ret`. A multi-line scan
  returns zero hits.
- **The transform is nearly mechanical.** 53 of 55 bodies are exactly
  `boundary!("<ident>", { … })` with nothing outside the wrapper. The exceptions
  are `posts::create` (`web/src/posts/api.rs:164-174`) and `posts::update`
  (`:310-321`), which destructure `CreateArgs`/`UpdateArgs` _before_ the
  wrapper. Wrapping the whole body is still correct — the `let` simply moves
  inside the `async move` block, and both fns already carry `skip_all` — but the
  conversion is not a blind 55× substitution.
- **The existing code already anticipates this.**
  `server_fn_coverage/extract.rs:130-131`: _"Never `"/api/" + ident` — the
  coincidence that they agree today is not load-bearing (#698 may drop the
  explicit attributes entirely)."_

## Decision

Add a `#[proc_macro_attribute]` to the `macros` crate. It derives all three
literals and wraps the body in the error boundary, so none can drift and the
boundary cannot be omitted.

Written fully-qualified (never `use`d) so it cannot be confused with leptos's
`#[server]`:

```rust
#[macros::server(skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    …
}
```

expands to:

```rust
#[::leptos::server(endpoint = "/audiences/rename")]   // absolute paths — AC-11
#[::tracing::instrument(name = "web.audiences.rename", skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    crate::error::server_boundary(async move { … }).await
}
```

### Derivation rule

`vertical` = the path segment following the `web/src/` marker in
`Span::call_site().file()`. Marker-relative, so a `--remap-path-prefix` build
(Nix) yields the same vertical as a host build. `local_file()` is **not** used —
its own docs say it must not be embedded in macro output.

- endpoint → `/<vertical>/<ident>`
- span name → `web.<vertical>.<ident>`

Both match what #684 shipped (`server_fn_endpoint_check.rs:100`,
`server_fn_tracing_check.rs:362`, vertical per `web_server_fns.rs:223-234`).

### Placement rule — what makes the key a primary key

**Every `#[server]` fn lives in `web/src/<vertical>/api.rs`. No submodules.**
The macro hard-errors on any other location.

Without this, `(vertical, ident)` is a _lossy_ projection: it takes only the
first path segment under `web/src`, so `posts/api.rs` and `posts/api/listing.rs`
share a vertical and a same-named fn in both derives one endpoint and one span
name for two functions. **The compiler does not catch that** — the glob
re-export `pub use listing::*;` (`web/src/posts/api.rs:16`) lets an item in
`api.rs` silently shadow the glob-imported name, so the pair compiles
(`server_fn_registrar_check.rs:524-528`, "verified with rustc"); one registrar
entry satisfies both and the loser silently 404s (#358).

With the rule, `(vertical, ident)` becomes a genuine **primary key enforced by
rustc**: the vertical is unique because it is a directory, and the ident is
unique within it because Rust forbids two items of one name in one module.
Uniqueness stops depending on a gate that fails open (R3) and starts depending
on the compiler.

This is a real constraint, not a formality: a vertical that outgrows one
`api.rs` cannot split its server fns into submodules. The escape hatch is
module-path derivation (`/posts/listing/<ident>`), which would be a deliberate
URL change at that point rather than a silent collision. Recorded so a future
reader need not rediscover it.

### Vertical placement — `timeline` gains the `api.rs` it should have had

Exactly one file violates the rule today: `web/src/posts/api/listing.rs`,
holding five `#[server]` fns. All five are _"Lists published, non-deleted
posts…"_ returning `WebResult<TimelinePage>` with identical cursor-pagination
parameters — they are the timeline queries. (`list_drafts` looks similar but
returns `Vec<DraftSummary>`, so it is genuinely a `posts` fn and stays.)

They move to a new **`web/src/timeline/api.rs`**, with their **four** `fetch_*`
helpers (`fetch_user_posts`, `fetch_local_timeline`, `fetch_posts_by_tag`,
`fetch_user_posts_by_tag` — `list_home_feed` has none, querying storage inline)
to `web/src/timeline/server.rs` per ADR-0070's four-file layout. The private
`page_from_rows` (`listing.rs:32`) and the file's `mod tests` (`:311-499`) move
with them. `posts/api/listing.rs` is deleted, along with `mod listing;` and
`pub use listing::*;` (`posts/api.rs:15-16`) — **which removes the shadowing
mechanism itself rather than guarding it.**

**One dependency does not travel cleanly, and its resolution is a decision this
spec must make.** `page_from_rows` — which every `fetch_*` funnels through —
calls `crate::posts::server::timeline_post_summary` (`listing.rs:18,44`). That
path is **not nameable from `crate::timeline`**: `posts/mod.rs:13` declares
`mod server;` privately and `:59` re-exports only `post_response`. **Decision:
re-export `timeline_post_summary` from `posts`** (the smaller change; it is
already `pub fn` at `web/src/posts/server.rs:9`) rather than relocating it,
since it maps a _post_ row and belongs to `posts`. The consequence is explicit:
`timeline` acquires a compile-time dependency on `posts`, which is consistent
with it already re-using `posts`' re-exported wire types, but should not be
discovered during implementation.

Everything else `listing.rs` uses is already reachable:
`crate::auth::require_auth`, `crate::viewer::viewer_identity`,
`crate::error::{…}`, and `common`/`storage` items.

`timeline` is server-less today for historical reasons, not principled ones:
ADR-0070 §5 created it as one of the "new vertical dirs where none exists" when
`pages/` dissolved into component files, and the data-fetching simply stayed
where it was. No ADR forbids a `timeline/api.rs`. The wire types are no obstacle
either — `TimelinePage` and `TimelinePostSummary` are defined in
**`common/src/seed.rs:36,62`**, not in `posts`; the timeline module doc's
"re-uses `crate::posts::{…}`" describes re-exports, not ownership.

**This changes five wire URLs and five span names** — the only value changes in
this spec, made deliberately:

| from                              | to                                   |
| --------------------------------- | ------------------------------------ |
| `/api/posts/list_by_user`         | `/api/timeline/list_by_user`         |
| `/api/posts/list_local_timeline`  | `/api/timeline/list_local_timeline`  |
| `/api/posts/list_home_feed`       | `/api/timeline/list_home_feed`       |
| `/api/posts/list_by_tag`          | `/api/timeline/list_by_tag`          |
| `/api/posts/list_by_user_and_tag` | `/api/timeline/list_by_user_and_tag` |

Span names follow (`web.posts.list_*` → `web.timeline.list_*`), as do registrar
entries (`web::posts::ListByTag` → `web::timeline::ListByTag`, registered at
`server/tests/helpers/mod.rs:61-65`).

**The Playwright suite is unaffected** — verified by an exhaustive sweep of
`end2end/` for both string literals _and_ ident-based route interception, since
ADR-0082:94-100 warns some specs match on the bare ident. Eight endpoints are
referenced in total: hardcoded URLs for `posts/create` (`posts.ts:15,31`),
`posts/update` (`feeds.spec.ts:270`), `media/upload` (`media.spec.ts:13,46,71`),
`backup/update_settings` (`backup.spec.ts:106,122`), `audiences/list_members`
and `audiences/list_mine` (`audiences.spec.ts:97-98`); plus ident-parameterised
interception via `failServerFn` (`helpers.ts:106-113`) for `auth/get_session`
(`authed-flash.spec.ts:111`), `audiences/list_mine` (`audiences.spec.ts:182`),
`audiences/list_members` (`:199`), and `audiences/list_my_subscribers` (`:230`).
**None of the five moving idents, nor their PascalCase types, appears anywhere
in `end2end/` or `elisp/`.**

The other 50 fns are already in `<vertical>/api.rs` and change nothing.

### Argument routing

| key                   | forwarded to             |
| --------------------- | ------------------------ |
| `input = …`           | `#[server]`              |
| `skip(…)`, `skip_all` | `#[tracing::instrument]` |

`endpoint` and `name` are **rejected** — the macro derives them. **`fields(…)`
is rejected**, as are `level`, `target`, `parent`, `err`, `ret`, and any
unrecognized key.

Rejecting `fields` costs nothing today (zero of 55 use it) and closes the
`skip(email)` + `fields(who = %email)` PII vector _by construction_ rather than
by checking — the same "make it unrepresentable" move as the boundary wrap. If a
future fn needs `fields`, it returns as a pass-through key **together with**
re-enabling rule 3's value-expression allowlist; the two must land as one
change.

**The macro's key list is the single source of truth for what may appear.** The
gate retains its own default-deny on unmodelled arguments anyway (AC-14) — two
independent default-denies, so adding a key to the macro cannot silently bypass
the allowlist.

### Macro structure — pure core, thin shell

`proc_macro` APIs panic outside a live expansion, so the derivation **must not**
call `Span::file()` directly:

- **Pure core** — takes the file path and parsed argument list as ordinary
  parameters, returns `Result<Derived, syn::Error>`. Fully unit-testable with
  `syn::parse_quote!`. Every error path and the happy path live here.
- **Shell** — the `#[proc_macro_attribute]`, a thin wrapper calling
  `Span::call_site().file()` and handing off.

This is what makes AC-7, AC-8, and AC-9 checkable at all; `macros` has no
`trybuild` harness (`macros/Cargo.toml:22-24`), so a compile-fail approach is
unavailable.

### Boundary

**The span and the boundary are complementary, not redundant — only the label is
redundant.** Neither emits an entry event. `#[tracing::instrument]` opens a
_span_: to the OTel exporter (`server/src/observability.rs:375-377`) that span
is itself the record of "this fn ran" — the substance of #681's capture — while
to the fmt sinks it emits nothing of its own, since no
`with_span_events`/`FmtSpan` is configured; it only decorates events raised
inside it. `server_boundary` (`web/src/error.rs:119-127`) emits exactly one
event and only on the `Err` path, and performs the wire projection; `Ok` is
silent. So the wrapper is load-bearing — without it an `InternalError` reaches
the client unprojected and the failure is never logged — and it is kept
unconditionally. What is redundant is solely the fn _identity_ passed to it,
which the enclosing span already stamps onto that same failure event. No event's
existence changes; one duplicated field goes.

The body is wrapped unconditionally. Consequently:

- `boundary!` (`web/src/lib.rs:14-19`) is **deleted**.
- `server_boundary` loses its `server_fn: &'static str` parameter.
- `emit_boundary_failure` (`host/src/error.rs:302`) loses it too, and the
  `server_fn` log field goes. The other five fields (`error.kind`,
  `error.class`, `error.public`, `error.source`, `error.context`) are genuine
  per-failure data and stay.

The emitted path is `crate::error::server_boundary`. Note this is **not** the
same mechanism as today's `$crate::error::server_boundary`
(`web/src/lib.rs:17`): `$crate` in a `macro_rules!` is a hygienic marker
resolving to the _defining_ crate, whereas a proc macro emits the bare token
`crate`, which is call-site resolved against the crate being _compiled_. They
coincide here only because `boundary!` is both defined and invoked in `web`. The
proc-macro behaviour is nonetheless the one we want: used from another crate,
`crate::error::server_boundary` resolves into _that_ crate's root and fails
loudly with a missing-item error, rather than binding silently to the wrong
thing. `extern crate self as web;` was considered and rejected — no precedent in
this repo, and it buys nothing.

Use is constrained by a path convention rather than by construction: the marker
test is a substring/`split_once` search for `web/src/` (cf.
`web_server_fns.rs:224-226`), so any path containing that segment passes (AC-8).

**Two load-bearing assumptions, stated because the current code relies on them
silently:**

1. `server_boundary` is `#[cfg(feature = "server")]` (`web/src/error.rs:114`)
   while `web` also builds for wasm32 without it. This compiles only because
   leptos's `#[server]` **discards the annotated body on the client**. Today's
   `boundary!` already depends on this.
2. `#[macros::server]` is the **outer** attribute, so it wraps the body _before_
   `#[server]` sees it — preserving (1).

Both are checked by AC-12 (wasm build + clippy), not deferred to the e2e run.

## Precondition — prove before building anything else

**P1.** A test proves the instrument span is in scope when the boundary logs a
failure. The design rests on the span carrying the fn identity; if it does not,
the label is not redundant and this spec is void (fallback: build #714's
comparing gate as originally specified).

P1 is provable **without the macro** — a plain
`#[tracing::instrument(name = "…")]` async fn calling `server_boundary`
exercises exactly the question. So it runs as task one, before any macro exists,
and its fixture is not subject to AC-8.

The existing
`server_boundary_evaluates_tracing_fields_when_subscriber_is_active`
(`web/src/error.rs:296-318`) is **not** a sufficient home as written: it
installs `fmt().with_test_writer()` and asserts only on the returned `WebError`,
capturing nothing about spans. P1 needs a layer that records the event's span
scope; no such layer exists in the tree and writing one is part of the task.

**If P1 fails, stop and re-open the design.**

## Effect on the gates

| gate                           | outcome                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `server_fn_endpoint_check.rs`  | **deleted**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `server_fn_tracing_check.rs`   | rule 1 (presence/placement) and rule 2 (span name) **deleted**. **Retained:** the per-parameter skipped-or-recordable check (`RECORDABLE_TYPES`), the nameless-parameter rule (`:382-388`), and default-deny on unmodelled arguments. **Superseded and removed with their tests:** the `fields(…)` value-allowlist, `IGNORED_ARGS` (`:102`), the `err`/`ret` rejection (`:201-207`), and `is_cfg_attr_instrument` (`:318`) with its guard and test — there is no longer a `#[tracing::instrument]` in source to wrap in a `cfg_attr` |
| `server_fn_registrar_check.rs` | **changed** — AC-3                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `server_fn_coverage`           | computed endpoint; two drift branches die — AC-9, AC-10                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| a `boundary!` gate             | **never built**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

## Acceptance criteria

### Correctness of the transform

1. **AC-1 (P1).** A test asserts a failure raised inside an instrumented fn
   emits its boundary event with the instrument span in scope, named
   `web.<vertical>.<ident>`. Runs first; failing it voids the spec.
2. **AC-2 (transform + placement).** All 55 server fns use `#[macros::server]`;
   zero occurrences of `#[server(`, `#[tracing::instrument(name =`, or
   `boundary!` remain in `web/src`. **Every one lives in
   `web/src/<vertical>/api.rs`** — verifiable by enumerating the files that hold
   them.

   Concretely that means: `web/src/timeline/api.rs` holds the five timeline
   queries and `web/src/timeline/server.rs` their five `fetch_*` helpers;
   `web/src/posts/api/listing.rs` is deleted along with `mod listing;` and
   `pub use listing::*;` (`posts/api.rs:15-16`); the five wire URLs and span
   names move from `posts` to `timeline` per the table above; and **no other URL
   changes value**, asserted by AC-4 over all 55 (span-name coverage is weaker —
   see AC-5). Registrar entries (`server/tests/helpers/mod.rs:61-65`),
   `posts/mod.rs` re-exports, and the committed coverage snapshot and seed are
   updated to match. `list_drafts` stays in `posts` — it returns
   `Vec<DraftSummary>`, not a `TimelinePage`.

   **Every consumer of the moved items, enumerated — this is the move's real
   blast radius and it reaches outside `web`:**
   - **`server/src/projector/mod.rs:42-43`** —
     `use web::posts::{fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag};`
     with call sites at `:204, :258, :294, :327`. This is the largest external
     consumer and it is in the `server` crate's **src**, not its tests.
   - **Three wasm-only call sites, invisible to a host build:**
     `web/src/posts/component.rs:30` (uses at
     `:1058, :1105, :1603, :1646, :1722, :1770`),
     `web/src/cockpit/component.rs:13` (`:38, :68`), and
     `web/src/home/component.rs:9` (`:40, :53`). Only AC-12's wasm clippy
     catches a miss here, which is a second reason that criterion is required
     rather than nice-to-have.
   - **`server/tests/web/web_posts.rs`** — the five fns' integration tests. They
     are updated in place; this spec does **not** require relocating them to a
     `web_timeline.rs`.

3. **AC-3.** `server_fn_registrar_check.rs` accepts the new form.
   `server_fn_default_named` (`:102-110`) requires **every** argument be
   `Meta::NameValue`, so `skip_all` (`Meta::Path`) and `skip(…)` (`Meta::List`)
   each trigger the hard error at `:83-89` — 16 of 55 sites. The gate must
   consider only arguments routed to `#[server]`. Since `input = …` is the sole
   routed argument and _is_ `Meta::NameValue`, this narrowing preserves
   positional-rename detection rather than defeating it. Unit tests cover a fn
   carrying `skip_all` and one carrying `skip(…)`.
4. **AC-4.** A test in `server/tests` (which can depend on `web`) asserts
   `<T as ServerFn>::PATH == "/api/<vertical>/<ident>"` for all 55, enumerated
   rather than spot-checked, **and that the 55 paths are pairwise distinct**.

   The distinctness half is the uniqueness backstop, and it matters because
   `(vertical, ident)` is a _lossy_ key: it takes only the first path segment
   under `web/src`, so `posts/api.rs` and `posts/api/listing.rs` — a pair that
   exists today — share a vertical. A same-named fn in both derives one endpoint
   and one span name for two functions. **The compiler does not catch this**:
   the glob re-export `pub use listing::*;` lets an item in `api.rs` silently
   shadow the glob-imported name, so the pair compiles
   (`server_fn_registrar_check.rs:524-528`, "verified with rustc"); one
   registrar entry satisfies both and the loser silently 404s (#358).

   The placement rule makes such a pair **impossible to write** — both fns would
   have to be in one `api.rs`, which rustc rejects. AC-4's distinctness
   assertion is therefore a backstop rather than the guard: it reads paths from
   the real generated types at runtime and never consults xtask's syn
   enumeration, so it holds even if the enumeration breaks (R3) or the placement
   rule is later relaxed.

5. **AC-5.** A **runtime** test drives a real `#[macros::server]` fn and asserts
   its emitted span is named `web.<vertical>.<ident>`. This pins three things
   AC-1 no longer can, now that P1 is macro-independent: the derived name, the
   absolute attribute paths (AC-11), and — critically — that `::leptos::server`
   is emitted **before** `::tracing::instrument`. That ordering was previously
   enforced by tracing-gate rule 1 with a stated rationale
   (`server_fn_tracing_check.rs:13-15`, "the arrangement known to produce a
   server-side span"), and rule 1 is being deleted. Neither AC-19's e2e nor the
   coverage gate would fail on a missing span (see AC-10's masking note).

   **Span-name coverage is deliberately weaker than AC-4's, and this records
   why.** There is no all-55 equivalent available: once the macro emits the
   name, it is in no source file for xtask to read, and `ServerFn` exposes
   `PATH` as a const but nothing for the span name — so the only ways to observe
   it are to drive the fn or to read a capture. Coverage is therefore layered:
   this criterion pins the _rule_ and the emission order on one fn; **AC-9's
   seed check pins the actual span name for the 53 driven fns**; the two
   undriven fns (`media/delete`, `sessions/revoke`) have no span-name assertion
   at all. Stated as a known gap rather than implied to be covered — an earlier
   draft asserted all 55 span names with no means of checking any of them.

6. **AC-6.** `boundary!`, the `server_fn` parameter, and the `server_fn` log
   field are gone from `web/src/lib.rs`, `web/src/error.rs`,
   `host/src/error.rs`.

### The macro itself

7. **AC-7.** The pure core rejects, naming the offending key: a passed
   `endpoint`, a passed `name`, a passed `fields`, and any unrecognized key.
   Unit-tested via `syn::parse_quote!`.
8. **AC-8 (placement).** The pure core accepts a path of exactly the form
   `web/src/<vertical>/api.rs` and rejects everything else, each with a message
   naming the rule. Unit-tested by passing the path as a parameter, covering: no
   `web/src/` marker at all; a fn directly under `web/src` (no vertical — a hard
   error already required by ADR-0082); and **a nested submodule such as
   `web/src/posts/api/listing.rs`**, which is the case that makes
   `(vertical, ident)` lossy and is now a compile error.
9. **AC-9 (happy path + anti-drift).** The pure core, given
   `web/src/audiences/api.rs` and ident `rename`, yields `/audiences/rename` and
   `web.audiences.rename`. **Anti-drift against xtask's independent copy of the
   rule is enforced against real runtime output:** the committed seed capture
   (`xtask/src/server_fn_coverage/testdata/otel-traces-seed.jsonl`, 187 lines
   from an actual e2e run) contains **53 distinct `/api/<vertical>/<op>` URIs**,
   ground truth produced by real macro expansion rather than a second
   restatement of the rule.

   **The test must not decide seed-presence by the endpoint it is checking.**
   Matching computed endpoints against seed URIs and skipping the misses makes
   the drift case _identical to_ the skipped case: a fn whose derivation is
   wrong is simply "absent" and passes silently. That is precisely the failure
   `extract.rs:16-22` records having already happened once — a matcher "matched
   **nothing** — and did so _silently_". So: establish presence via **span
   name + `code.namespace`** (signal 1, independent of the endpoint), then
   assert the endpoint observed for that span equals the computed one, **and
   assert the matched count is exactly 53** so a silent shrink fails loudly.

   **Residual gaps, stated rather than papered over:** the two fns absent from
   the seed (`media/delete`, `sessions/revoke`) are covered only by AC-4. And a
   fully mechanical cross-check inside xtask remains impossible —
   `xtask/Cargo.toml:1` makes it its own workspace and `:9-23` has no `web`
   dependency.

   **Sequencing (see R6):** the move changes five URIs and five span names that
   appear in 68 of the seed's 187 lines, so this test cannot pass until the seed
   is regenerated by a real e2e run. It is ordered after the move and after
   `cargo xtask server-fn-coverage regenerate`. The seed is never hand-patched —
   a hand-edited seed would assert the rule against itself.

10. **AC-10.** The coverage gate's now-unreachable endpoint branches go with
    their tests: `verdict` (`server_fn_coverage/snapshot.rs:122-131`) has a
    `None` → "bare `#[server]` with no `endpoint`" branch and a `Some(ep)` ≠
    derived branch, both vacuous once the endpoint is computed; tests at
    `:316-327`. **Also record** that `identify` (`extract.rs:111-136`) returns
    on the span-name + `code.namespace` signal and only falls through to `uri`
    on a miss — and every fn now carries a span, so the URI signal is masked at
    runtime and a wrong computed endpoint would _not_ show as lost coverage. The
    endpoint is kept (defence in depth, per ADR-0081's lesson that a single
    silently unmatched signal is exactly the failure mode) but its only live
    verification is AC-9.
11. **AC-11.** The expansion emits **absolute** attribute paths
    (`::leptos::server`, `::tracing::instrument`), not bare
    `server`/`instrument`. Attribute macros are not path-hygienic, so a bare
    path resolves against each call site's `use leptos::prelude::*` —
    reintroducing the ambiguity the fully-qualified `#[macros::server]` spelling
    exists to avoid, and breaking if that import is pruned. Whether leptos
    re-exports at `::leptos::server` or `::leptos::prelude::server` is confirmed
    at build time.
12. **AC-12.** `cargo clippy --target wasm32-unknown-unknown -- -D warnings`
    passes for `web`, proving the client build still discards the wrapped body.
13. **AC-13.** The macro's error paths are covered per the coverage policy
    (`macros` is coverage-measured). The thin shell's `Span::call_site().file()`
    call is unreachable from `cargo test`; it is exempted with a block-form
    `cov:ignore-start`/`-stop` and a required reason (`CONTRIBUTING.md:458`),
    which `:449-451` names the **only** manual acceptance path — there is no
    structural "thin-shell" exemption for this (`:428-431`'s structural
    exemption is `#[component]`/`#[client_only]` for wasm-only UI). It is a
    reviewable decision (`:539`).

### The gates

14. **AC-14.** **The gate reads `skip(…)`/`skip_all` from
    `#[macros::server(…)]`**, since no `#[tracing::instrument]` remains in
    source — the same narrowing AC-3 specifies for the registrar, and the
    substantive part of keeping these rules alive. With rules 1 and 2 gone,
    `problems_with`'s `vertical` parameter (`server_fn_tracing_check.rs:330`)
    has no remaining use and is removed with them. `server_fn_tracing_check.rs`
    retains and tests three rules: a parameter neither skipped nor on
    `RECORDABLE_TYPES` fails; **the nameless-parameter rule** (`:382-388` — a
    parameter bound by a destructuring pattern cannot be named in `skip(...)`
    unless `skip_all` covers it) fails, which is currently a **retained but
    untested** rule and gains a test here; and an argument the gate does not
    model fails default-deny. Superseded tests removed.
15. **AC-15.** The `web/src` enumeration is asserted non-empty **and equal to
    the registrar's entry count** (both already computed in
    `server_fn_registrar_check.rs`), rather than to a literal 55 that every new
    server fn would churn and invite blind bumping. Note `registered_entries`
    returns a `BTreeSet` (`:138-140`), so the comparison is against the
    **deduped** entry set — deliberately, since a duplicated `register_explicit`
    line is harmless and must not redden the gate. All three gates fail **open**
    on an empty enumeration (`problems()` returns `None` —
    `server_fn_tracing_check.rs:482-484`,
    `server_fn_registrar_check.rs:272-274`), and the enumerator predicate is
    `a.path().is_ident("server")` (`web_server_fns.rs:96`), **false** for the
    two-segment path `macros::server`. Without this criterion a stale predicate
    is a silent green across the registrar, tracing, and coverage gates at once.
    AC-2's grep does **not** catch it — it inspects `web/src` source text and
    passes regardless.
16. **AC-16.** `server_fn_endpoint_check.rs` deleted, removed from the xtask
    step registry (`xtask/src/lib.rs:35,342,380`), **and** the intra-doc links
    to it at `web_server_fns.rs:6,13` updated so the `doc_links` step passes.
17. **AC-17 (manifests).** `web/Cargo.toml` gains
    `macros = { path = "../macros" }` (the form at `common/Cargo.toml:15`).
    **`macros/Cargo.toml` gains `features = ["full"]` on `syn`**: it is
    `syn = { workspace = true }` (`:11`) against a root `syn = "2"` with **no
    features** (`Cargo.toml:98`), and syn 2's defaults
    (derive/parsing/printing/clone-impls/proc-macro) do **not** include `full`.
    Every existing macro in the crate parses only `DeriveInput`, so this has
    never been needed; an attribute macro that parses an `ItemFn` and rewrites
    its `Block` cannot compile without it. Note this touches the shared vendor,
    so the Nix build must be re-checked, not assumed — the source filter and the
    vendor derivation both.
18. **AC-18 (and its ordering constraint).** `cargo xtask validate --no-e2e`
    passes. **This cannot happen until AC-19's e2e run has produced a capture**,
    which inverts the usual order and must be reflected in the plan.
    `server_fn_coverage_check.rs:330-333` builds `seed_coverage()` from the
    **live** `web/src` inventory matched against the **committed** seed, and
    `:353-368` (`seed_capture_covers_the_committed_snapshots_fns`) asserts every
    fn the snapshot calls covered is covered in the seed. The moment the
    snapshot keys become `timeline::list_*` while the seed still carries
    `web.posts.list_*`, that test fails — and
    `cargo xtask server-fn-coverage regenerate` reads
    `CAPTURE_PATH = .xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`
    (`server_fn_coverage/io.rs:25`), re-reduced via
    `testdata/reduce-otel-capture.mjs`. `CONTRIBUTING.md:480` forbids
    hand-editing the snapshot; the same holds for the seed (R6). So the sequence
    is: move → e2e run → regenerate → `validate --no-e2e`.
19. **AC-19.** A full e2e combo passes, proving the wasm client and server agree
    on every URL. A compile is not sufficient evidence.

### Documentation and decision records

20. **AC-20.** **ADR-0082** (`docs/adr/0082-server-fn-wire-namespace.md`,
    promoted by #724) is revised — the wire scheme is unchanged but is now
    **macro-derived, not gate-written**. False after this change:
    - `:55-68` — "`endpoint` stays explicitly pinned on every fn"; "the
      `server-fn-endpoint` gate computes the expected value and … writes it"; "a
      missing `endpoint` is reported, never synthesized".
    - `:81-86` — "Endpoint uniqueness is entirely gate-enforced." That check
      dies with `server_fn_endpoint_check.rs:121-129`; the registrar's
      `(vertical, leaf)` duplicate check still catches collisions, but the ADR
      records that this defence-in-depth layer is dropped **deliberately**.
    - `:101-107` — the bullet forecasting #714 as a body-traversing gate.
      Superseded.
    - Rejected-alternatives gains `DISABLE_SERVER_FN_HASH` and
      `SERVER_FN_MOD_PATH` (neither currently mentioned) and why #698's variant
      is unsafe post-#684: bare-verb idents make `/api/<ident>` collide across
      three verticals, and #426's duplicate-leaf-name guard that #698 cited as
      its precondition was re-keyed to `(vertical, leaf)` by #684.
    - New consequence: the derived literals no longer appear anywhere in the
      tree, so the greppability argument at `server_fn_endpoint_check.rs:20-22`
      and ADR-0011:214-216 is deliberately traded away. This matters most for
      the hardcoded URLs in `end2end/tests/**` that ADR-0082:94-100 already
      flags as drift-prone (#712).
21. **AC-21.** **ADR-0011** amended: the span name is derived by the macro,
    resolving #722. Records that plain `#[tracing::instrument]` derivation
    yields `__server_<ident>` (unusable), that the macro gets a readable name
    without that coupling, and that `server_fn` as a log field is retired as
    redundant with span context, citing both formatters.
22. **AC-22.** **ADR-0070** amended: `timeline` is no longer a server-less
    vertical. §5 created it as one of the "new vertical dirs where none exists"
    when `pages/` dissolved into component files, and
    `web/src/timeline/mod.rs:5-8` records the server-less state as fact. It
    gains `api.rs` and `server.rs`, completing the four-file layout the ADR
    prescribes. The ADR also gains the **placement rule** — a vertical's
    `#[server]` fns live in its `api.rs`, never a submodule — which is what
    makes `(vertical, ident)` a compiler-enforced primary key, and is a genuine
    tightening of §1's `api.rs` bullet. Correct `mod.rs:5-8`'s claim that the
    timeline types come from `crate::posts`: they are defined in
    `common/src/seed.rs:36,62` and merely re-exported through `posts`.
23. **AC-23.** **ADR-0066** corrected: `:103-107` asserts the
    `server-fn-endpoint` gate enforces endpoint collisions "independently"
    (false once deleted); `:114` states the gate "assumes the
    `#[server(endpoint = "…")]` form" (superseded by AC-3).
24. **AC-24.** Further live files describe behavior this change removes, each
    updated. **`docs/web-style-guide.md:181-183` names `timeline/` by name as
    the canonical example of a server-less vertical that "omits `api.rs` too"**
    — that becomes false with AC-22, and it is exactly where a developer looks
    for the vertical layout. **The placement rule is recorded in
    `CONTRIBUTING.md` and `docs/web-style-guide.md:171-215` as well as
    ADR-0070**, since it is now compiler-enforced and load-bearing (R5) and
    CLAUDE.md names CONTRIBUTING.md the single source of truth. Then:
    `docs/web-style-guide.md:208`;
    `docs/adr/0013-server-submodule-pattern.md:85,103-117` (a section titled
    "The `boundary!` Macro"); `docs/adr/0016:185,224,253`; `docs/adr/0056:59`;
    `docs/adr/0059:122`; `docs/adr/0065:97`; and **`CONTRIBUTING.md`** `:25`,
    `:505-507` ("Endpoint drift fails loudly… a bare `#[server]` … is
    rejected"), `:726`. CLAUDE.md names CONTRIBUTING.md the single source of
    truth, so this is not optional.
25. **AC-25.** #698 and #722 closed referencing the ADRs above; #714 closed by
    this work.

## Separable concerns — filed as issues, not folded in

- **Retire the registrar gate via `linkme` auto-registration.** ADR-0066:28-29
  lists as rejected alternative B "auto-register … **via a wrapper attribute
  macro in the `macros` crate**" — rejected because that macro did not exist. It
  now will, so the alternative's principal cost is already paid, and the
  registrar gate plus its hand-maintained list in `server/tests/helpers/mod.rs`
  may no longer be the right answer. Its own cycle.

## Non-goals

- Changing any wire URL or span name value **other than the five timeline
  queries moving from `posts` to `timeline`** (AC-2). The other 50 are
  mechanism-only.
- Moving the PII allowlist into the macro. It stays in xtask, reviewable, with
  its per-entry rationale.
- `DISABLE_SERVER_FN_HASH` / `SERVER_FN_MOD_PATH`. Rejected; recorded in AC-20.
- The TypeScript-side URL guard (#712), which remains open.

## Risks

- **R1 — P1 is false.** Mitigated by making it task one and macro-independent;
  failure voids the spec rather than surfacing late.
- **R2 — path remapping.** Marker-relative extraction is insensitive to prefix
  remapping, and a missing marker is a hard error (AC-8), never a silently wrong
  endpoint. Confirm under an actual Nix build, and confirm the Nix toolchain is
  ≥1.88 — the `rust-toolchain.toml` channel is floating, not pinned.
- **R3 — the enumerator predicate goes stale and every gate fails open.**
  Covered only by AC-15.
- **R4 — the wire has no runtime guard left but AC-5 and AC-19.** With rule 1
  deleted and the URI signal masked (AC-10), a silently missing span would be
  caught by AC-5 alone.
- **R5 — the placement rule is now load-bearing for uniqueness.**
  `(vertical, ident)` is a primary key _only because_ one `api.rs` holds a
  vertical's server fns; relax that and the key silently goes lossy again, with
  the compiler unable to help (glob shadowing makes a within-vertical duplicate
  compile, and the loser 404s — #358). Mitigations, in order of independence:
  the macro's own compile error (AC-8), AC-4's enumeration- independent
  pairwise-distinctness assertion, and the registrar's duplicate-leaf check.
  Deleting the endpoint gate (AC-16) removes a fourth layer, which is acceptable
  only because AC-8 replaces gate-checking with a compile error.

  Considered and rejected: keying on the full module path
  (`web.posts.api.listing.list`), which #722 correctly notes is collision-proof
  by construction. Rejected because it changes all 55 wire URLs and span names,
  and because the placement rule achieves the same guarantee while changing
  five. It remains the escape hatch if a vertical ever genuinely outgrows a
  single `api.rs` — recorded so a future reader need not rediscover it.

- **R6 — the move is the only value-changing part of this spec, and its blast
  radius is wider than the macro's.** Five URLs, five span names, five registrar
  entries, `posts/mod.rs` re-exports, `server/tests` call sites, and the
  committed coverage snapshot and seed all change together. Playwright is
  verified unaffected (its hardcoded URLs are `backup/update_settings`,
  `media/upload`, `audiences/list_members`, `audiences/list_mine` only).

  **The seed invalidation is not marginal: 68 of its 187 lines (36%) carry a
  `/api/posts/list_*` URI or a `web.posts.list_*` span name**, because
  `list_local_timeline` fires on nearly every page load. So this is a
  task-ordering constraint, not a caution: the move lands, then
  `cargo xtask server-fn-coverage regenerate` runs against a real e2e capture,
  and only then can AC-9 pass. The seed is never hand-patched — editing it to
  match the expected values would make AC-9 assert the rule against itself.
