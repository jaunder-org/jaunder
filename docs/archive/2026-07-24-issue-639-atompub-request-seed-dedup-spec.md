# Spec — #639: AtomPub/feed request-construction + post-seeding dedup

Issue: jaunder-org/jaunder#639 · Blocked-by #635 (merged) · Label: `dx` · Type:
Task

## Problem

Once the #635 fixture-convergence pass removed the _seeding_ noise, the
AtomPub/feed integration suite is dominated by three families of near-identical
boilerplate:

1. **Hand-rolled `perform_post_creation` blocks (~16, `atompub_posts.rs`).** A
   10-field
   `storage::perform_post_creation(state.posts.as_ref(), storage::PostCreation { … })`
   block — the **service layer** (auto-render + slug-retry) — repeated ~16 times
   in `atompub_posts.rs`, varying only in title / body / published / audience.
   **Layer boundary (corrected during plan review):** the `feed/*`,
   `projector/*`, `web/*`, and `misc/*` integration tests do **not** use
   `perform_post_creation`; they call the lower
   `state.posts.create_post(&CreatePostInput { rendered_html: RenderedHtml::from_trusted(…), slug: …, … })`
   **storage layer** with explicit pre-rendered HTML and explicit slugs. Those
   `create_post`-layer sites are a different fixture (a `create_post` builder,
   not `SeedPost`) and belong with the storage-layer dedup in **#656** — _not_
   in this issue. This issue's axis 1 is therefore the atompub suite's
   `perform_post_creation` blocks (the ~16 in `atompub_posts.rs` plus one in
   `atompub_service.rs`).
2. **Session-keyed AtomPub request construction (~70).** Every authenticated
   AtomPub request repeats `session.username` twice (in the URI _and_ the
   Basic-auth arg) and hardcodes the `/atompub/{username}/` prefix — ~54 sites
   in `atompub_posts.rs`, ~15 in `atompub_media.rs`, ~3 in `atompub_service.rs`
   (exact counts confirmed at migration).
3. **CRUD-variant test families** that are the same request shape differing only
   by URI fragment / body / expected status, written as copy-pasted
   `#[tokio::test]` fns.

This is a **behaviour-preserving test refactor**. No production code path
changes; no runtime behaviour changes. Pure DX / test quality.

## Goals

Drive the suite's request-construction and post-seeding down to its irreducible
core: a `SeedPost` fixture (axis 1), a session-keyed AtomPub request-helper
family layered on the existing `atompub_authed` (axis 2), and parameterization
of the CRUD-variant families (axis 3).

## Non-goals

- Any change to production code or handler behaviour.
- Reshaping `storage::test_support::seed_posts` (the batch, generic-post seeder
  used in-process _and_ by the out-of-process e2e `seed-posts` feature).
  `SeedPost` is a **distinct** fixture; `seed_posts` is untouched.
- Touching any `state.posts.create_post(&CreatePostInput { … })`
  **storage-layer** site. That includes both (a) the `storage`-crate contract
  tests (`storage/tests/**`, `storage/src/**` inline `#[cfg(test)]` — batch,
  slug-conflict, idempotency, rendered-html mismatch, where the literal _is_ the
  thing under test) **and** (b) the `server` integration tests that seed via
  `create_post` with explicit `rendered_html`/slug (`feed/*`, `projector/*`,
  `web/*`, `misc/*`). All `create_post`- layer dedup — contract suite and these
  integration sites alike — is **#656**'s concern; `SeedPost` (which wraps the
  higher-level `perform_post_creation`) deliberately does not reach them, and
  converting them to it would swap their explicit rendering/slug for the
  auto-render + suffix-retry path (a behaviour change).
- #640 (borrow `state` in the `post_*` request helpers) — a separate
  signature-change issue on a different helper layer. Not in scope here.

## Design decisions

### Axis 1 — `SeedPost` builder (`storage::test_support`)

A builder mirroring the existing `SeedUser` convention, living beside it in
`storage/src/test_support.rs`, wrapping **`perform_post_creation`** — the real
slug-retry production path — so generated slugs and permalinks are authentic
(the AtomPub member/permalink tests assert on them). It is `#[cfg(...)]`-gated
exactly as `SeedUser` is (in-process test use only), distinct from `seed_posts`'
batch `create_posts` path.

```rust
let post = session.seed_post().seed(&state).await;                  // 95% case → PostRecord
let post = session.seed_post().title("Hello Title One").seed(&state).await;
```

**Aggressive defaults — a call site deviates from a default ONLY when required
for test correctness** (the `SeedUser` discipline):

| Field             | Default                        | Settable?                         |
| ----------------- | ------------------------------ | --------------------------------- |
| `user_id`         | (required arg to `new`)        | —                                 |
| `title`           | `None`                         | `.title(&str)`                    |
| `body`            | a fixed non-empty default      | `.body(impl Into<PostBody>)`      |
| `audiences`       | `vec![AudienceTarget::Public]` | `.audiences(Vec<AudienceTarget>)` |
| `format`          | `PostFormat::Markdown`         | fixed (no setter)                 |
| `published_at`    | `Some(Utc::now())` (published) | fixed (no setter)                 |
| `slug_override`   | `None`                         | fixed (no setter)                 |
| `summary`         | `None`                         | fixed (no setter)                 |
| `idempotency_key` | `None`                         | fixed (no setter)                 |
| `max_attempts`    | `100` (internal)               | —                                 |

- The default body is **non-empty** (so `derive_post_metadata` never returns
  `EmptyPost`) and title-less, so repeated bare `SeedPost::new(uid).seed()`
  calls produce **distinct** slugs via the existing collision-suffix retry.
- `.seed(&state)` is `async`, returns `PostRecord` (carries `post_id` _and_
  `slug` — both are read at call sites), and `expect()`s success (happy-path
  setup, like `SeedUser::seed`).
- **Only the three fields real call sites vary — title, body, audiences — are
  settable.** After the atompub-only re-scope, the migrated `atompub_posts.rs`
  seeds override just those; the rest (Markdown, published-now, no
  slug/summary/idempotency) are fixed defaults with **no setter**, mirroring how
  `SeedUser` exposes only the setters its callers use (no speculative setters).
  The `create_post`-layer builder in #656 owns the
  `rendered_html`/`slug`/scheduled-post vocabulary its contract tests need.

**On shedding `user_id` and `&state` (the interview ask "get rid of them if
possible"):**

- `&state` is **irreducible**. The per-test `TestEnv` architecture
  (ADR-0033/0053) passes the DB handle explicitly everywhere; there is no
  ambient handle, and every sibling fixture (`SeedUser::seed(&state)`,
  `seed_posts(&state, …)`) takes it. Keeping it is consistency, not ceremony.
- `user_id` is **shed at every #639 call site** via a thin forwarder on the
  session fixture: `impl SeededSession { fn seed_post(&self) -> SeedPost }` (in
  `server/tests/helpers`, returning `SeedPost::new(self.user_id)`). All atompub
  seeding sites hold a `session`, so every one writes
  `session.seed_post()…seed(&state).await` and never repeats `session.user_id`.
  (`SeedPost::new(user_id)` stays public for session-less callers — e.g. storage
  tests — but #639 has none after the re-scope.) The forwarder is analogous to
  how `create_user_and_session` wraps `SeedUser`.

### Axis 2 — session-keyed AtomPub request helpers (`server/tests/helpers/mod.rs`)

Layered on the existing `atompub_authed`. The path argument is the **suffix
after `/atompub/{username}/`**; the helper prepends the prefix and pulls
username + token from the `SeededSession`, so
`session.username`/`session.token`/the prefix each appear once.

| Helper                                                                | Expands to                                                                                                                             |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `atompub(&session, method, suffix)` → `Builder`                       | method + full URI + Basic auth, **chainable** (the extra-header base)                                                                  |
| `atompub_get(&session, suffix)` → `Request`                           | GET, empty body                                                                                                                        |
| `atompub_send_xml(&session, method, suffix, xml)` → `Request`         | Basic auth + `application/atom+xml` body                                                                                               |
| `atompub_post_xml` / `atompub_put_xml` (thin over `atompub_send_xml`) | the POST/PUT create/update case                                                                                                        |
| `atompub_upload(&session, slug, bytes)` → `Request`                   | media POST: `image/png` + `slug` header + bytes                                                                                        |
| `atompub_at(&session, method, uri)` → `Builder`                       | Basic auth from the session against a **verbatim** URI (not suffix-prefixed) — the create→capture-`Location`→PUT/DELETE follow-up case |

- The chainable `atompub(&session, method, suffix)` builder is the composition
  point for the extra-header cases (`If-Match`, `Idempotency-Key`, media `slug`,
  a non-default content type): callers chain `.header(…)`/`.body(…)` exactly as
  they do onto `atompub_authed` today.
- **Two request shapes cannot use the suffix helpers** and are handled
  explicitly: (a) **cross-user negative tests** authenticate as the session user
  but target a _foreign_ username (`/atompub/bob/media`) — they keep a literal
  foreign prefix (the point of the test); (b) **`Location`-follow-up requests**
  (create → capture the response `Location` header → authenticated
  PUT/DELETE/GET against that absolute URI) use
  `atompub_at(&session, method, location)`, which still pulls auth from the
  session so `session.username`/`session.token` are never doubled — only the URI
  is verbatim.
- The odd media cases (a non-`image/png` content type; a `slug` like `".."`) use
  the chainable builder rather than `atompub_upload`, so `atompub_upload` stays
  the fixed `image/png` happy path.
- `atompub_authed` / `atompub_xml` may remain as the primitives the new helpers
  are built on; the acceptance is that **no test call site** hand-repeats the
  username or the prefix, not that the primitives are deleted.

### Axis 3 — parameterize the CRUD-variant families (aggressive)

Use the existing `#[apply(backends_matrix)]` reuse template (documented
specifically to compose with a test's own local `#[case]`/`#[values]` rows) plus
local `#[case]` rows for the input variants. Default to **collapsing** any
CRUD-variant family that differs only by data (URI fragment / body / expected
status) into a single parameterized test; leave a family unrolled **only** when
its variants differ structurally (different setup, assertions, or seeding), and
when so leave a one-line comment stating why.

Families explicitly in scope (from the survey): the collection GET/limit
variants, the media upload variants, and the feed-format `{rss, atom, json}`
triples — plus any other family the sweep surfaces that meets the
data-only-varying bar.

## Acceptance criteria

Each is observable by `dev-cycle-ship`'s conformance review against the branch
diff:

- **AC1 — no hand-rolled `perform_post_creation` for routine setup in the
  atompub suite.** In `server/tests/atompub/*` (every `perform_post_creation`
  site — the ~16 in `atompub_posts.rs` plus the one tagged-post seed in
  `atompub_service.rs`), no test constructs a `storage::PostCreation { … }`
  literal for routine post setup; each such site is a `SeedPost` builder chain
  (or `session.seed_post()`). Explicitly **excluded** (may keep literals, by
  design): (i) every `state.posts.create_post(&CreatePostInput { … })`
  **storage-layer** site — the `storage`-crate contract suite _and_ the
  `feed/*`, `projector/*`, `web/*`, `misc/*` integration sites — all of which
  are #656's concern; (ii) tests that deliberately exercise
  `perform_post_creation`'s own contract (idempotency-key dedup, empty-post) in
  `post_service.rs`. The plan enumerates any borderline site it leaves as a
  literal with a one-line reason.
- **AC2 — `SeedPost` shape.** `storage::test_support` exports a `SeedPost`
  builder with the defaults and setters in the table above; `new(user_id)` +
  `.seed(&state)` returns `PostRecord`; a bare
  `SeedPost::new(uid).seed(&state).await` compiles and yields a published,
  public, Markdown post with a non-empty body and a unique slug.
- **AC3 — session forwarder.** `SeededSession::seed_post()` returns a `SeedPost`
  pre-owned by the session's user; every authenticated AtomPub post-seeding site
  uses it (no `session.user_id` threaded into `SeedPost::new`).
- **AC4 — no doubled username / hardcoded own-user prefix.** No authenticated
  AtomPub request repeats `session.username` twice (URI _and_ auth arg), and no
  request against the **session user's own** resources hardcodes a literal
  `/atompub/{session.username}/` prefix. Carve-outs (not violations): (a)
  cross-user negative tests targeting a _foreign_ username keep the literal
  foreign prefix; (b) `Location`-follow-up requests use
  `atompub_at(&session, …)` against the captured absolute URI. The measurable
  core: after the refactor, `session.username` appears at most once per request
  and `session.token` never travels beside a re-typed username.
- **AC5 — parameterization.** Every genuinely-uniform copy-paste CRUD-variant
  family in the **atompub** files is collapsed with
  `#[apply(backends_matrix)]` + `#[case]`. Outcome of the aggressive sweep (the
  "documented decision" the axis allows):
  - **Collapsed:** the DELETE `If-Match` family
    (`delete_with_{stale,matching, without,wildcard}_if_match_*` → one
    `delete_if_match_precondition`, 4 cases).
  - **Already de-duplicated, nothing to collapse:** most of the suite was
    parameterized by #635 (cursor-validation, forbidden-ops, format-media-type,
    empty-entry, the media cross-user family). The survey's "feed-format
    `{rss,atom,json}` triple" is **not** a copy-paste family — it is a single
    test looping an in-body `(ext, content_type)` table
    (`feed_handlers.rs::handler_returns_correct_content_type_per_format`); a
    loop→`#[case]` rewrite would be cosmetic, and that file is a
    `create_post`-layer file owned by **#656**, so it is left as-is.
  - **Left unrolled (structurally non-uniform):** the idempotency family
    (`create_with_{same,fresh,without}_idempotency_key_*`) differs by request
    count and assertion shape (dedup vs distinct-loc vs plain-201), so `#[case]`
    would obscure rather than clarify — each keeps its `AC-S{1,2,3}` intent
    comment.
- **AC6 — setters only where load-bearing.** A `SeedPost` chained setter appears
  **only** where the test asserts on (or otherwise requires) that varied field;
  every other post-seeding site is the bare `SeedPost::new(uid).seed(&state)` /
  `session.seed_post().seed(&state)` form. (This is the measurable restatement
  of "~95% bare" — a reviewer checks each chained setter against a corresponding
  assertion.)
- **AC7 — green.** `cargo xtask validate` passes (behaviour-preserving; the same
  tests assert the same things through the new fixtures/helpers).

## Verification

`cargo xtask validate` — the full local gate (static + clippy + coverage + e2e).
Because this is a test-only refactor, "green" is the behaviour-preservation
proof: every migrated test still exercises and asserts the same surface, just
through the shared fixtures. No new production code, so no new coverage
obligations beyond the fixtures themselves being exercised by their consumers.

## Separable concerns

- **#640** — borrow `state` in the `post_*` request helpers. Already its own
  tracked issue (the second half of this batch).
- **#656** (filed from this cycle, `blocked-by #639`) — a
  `create_post`-storage-layer post builder covering **all** `create_post`
  `CreatePostInput` literals: the `storage`-crate contract suite
  (`server/tests/storage/mod.rs`, `storage/src/posts.rs`) **and** the `server`
  integration sites that seed at that layer with explicit `rendered_html`/slug
  (`feed/*`, `projector/*`, `web/*`, `misc/*`). The latter were moved out of
  #639 during this plan review once it was found they use `create_post` (storage
  layer), not `perform_post_creation` (service layer) — a `SeedPost` migration
  would swap their explicit rendering/slug for auto-render + retry. #656 is
  sequenced after #639 to mirror the `SeedPost` shape and avoid a naming clash.
