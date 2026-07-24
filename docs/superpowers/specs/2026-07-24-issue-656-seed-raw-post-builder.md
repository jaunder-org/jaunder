# Spec — #656 `SeedRawPost`: a `create_post`-storage-layer post builder

**Issue:** jaunder-org/jaunder#656 · **Milestone:** Test infrastructure & E2E ·
**Type:** Task (dx) · **Behaviour change:** none (test-only refactor).

## Problem

Across the test suite, **~90+ `CreatePostInput { … }` literals** hand-roll a ~10-field
block to seed a post via the **`state.posts.create_post(&CreatePostInput { … })` storage
layer**, differing only in the one or two fields a test actually cares about. Two
locations, **one layer**:

- **A. `storage`-crate contract suite** — `server/tests/storage/mod.rs` and the inline
  `#[cfg(test)]` block in `storage/src/posts.rs` (routine setup + contract tests).
- **B. `server` integration seeds** — `feed/{feed_handlers,feed_worker,feed_regenerate}`,
  `projector`, `web/{tags,media,posts}` (storage-layer sites only), `misc/backup_fixture`.
  Moved here from the #639 plan review: they seed at the *same `create_post` layer*, not
  the `perform_post_creation` service layer #639's `SeedPost` covers.

`storage/src/posts.rs` has no factory (~14 full literals + a `mk` closure); mod.rs has two
file-local factories (`make_create_post_input`/`make_published_create_post_input`, ~22
sites) that still don't cover tag/audience/format/summary sites. `#639`'s service-layer
`SeedPost` (slug-retry) would swallow the conflicts the contract tests assert on;
`seed_posts` is too generic. So both A and B want the *same* new fixture.

## The literal audit (the SeedUser lens)

The decisive question, applied per field (slug/title/body/rendered_html) at every adopting
site: **is the specific value load-bearing at all, and if so must the test hardcode it, or
can the fixture generate it and hand it back?** — exactly why `SeedUser` autogenerates the
username and returns `SeededUser { user_id, username }` (tests almost never chose a name;
the few that use one read `.username`). Two independent audits (2026-07-24) classified
every site:

- **Ceremony** (value never asserted/correlated) → autogenerate, drop the literal.
  **≈ 71 sites** (≈ 63 in A + ≈ 8 bare in B) collapse to `SeedRawPost::new(uid)…seed()`.
- **Read-back** (value used, but as *"the post's actual slug/title"*, not a chosen string)
  → autogenerate and return it on `SeededPost`; the test references `seeded.slug` etc.
  **≈ 9 sites** (A: `post_create_and_get_by_id_works`, `post_slug_conflict`,
  `post_round_trips`, `list_published_in_window`, `get_by_permalink_soft_deleted`, the two
  batch tests; B: feed `body.contains(title)` sites, projector `seed_published_post`,
  `web_posts::create_targeted_post`, `backup_fixture`).
- **Genuine literal** on the `create_post` path → **1**: `web_media:263`, whose **body**
  must carry the test-constructed `media_url` (a `.body()` override). The delete-media
  reference scan matches `post.body.contains(url) || post.rendered_html.contains(url)`
  (`web/src/media/api.rs:157`), and the body already carries the URL, so no rendered_html
  control is needed. Conflict tests express *sameness* (`post_slug_conflict`,
  `create_posts_conflict_rolls_back_whole_batch`) via a shared local slug, not a
  re-hardcoded string — a relation, not a chosen value.

**Partly in scope — `create_rendered_post` / `update_rendered_post` tests.** The builder
wraps `create_post` (a pre-rendered `CreatePostInput`); `create_rendered_post` /
`update_rendered_post` are the layer above that *render* `body`→HTML then store. Split by
role:

- **Out of scope — the call-under-test.** `create_rendered_post_markdown_renders_and_stores`
  (mod:5852), `_org_renders_and_stores` (5893), and the `update_rendered_post_*` update
  calls exist to assert the rendering (`"**bold**"` → `<strong>bold</strong>`). That call
  *is* the SUT; replacing it with the builder would bypass the render function under test,
  and its body literal is the subject, not noise. These stay.
- **In scope — separable setup posts inside those tests.** The slug-occupier first call in
  `create_rendered_post_slug_conflict_returns_storage_error` (mod:5941) and the pre-update
  seed in `update_rendered_post_*` are just "a post exists" — they adopt `SeedRawPost`
  (`.published_at(now)` for the conflict's same-day relation; read `seeded.slug` for the SUT
  call's slug), while the SUT `create_rendered_post`/`update_rendered_post` call stays.

Fully **out of scope**: `seed_post_published_at` / scheduled-boundary tests; `seed_posts`;
the `web_posts` HTTP `create_post_json` sites; the `web_tags` clamp-limit bulk-tag loops.

## Design

### The builder (`storage/src/test_support.rs`)

Mirrors `SeedUser`: aggressive defaults, autogeneration, a returned fixture struct; a site
deviates from a default *only* when a test needs it. Wraps `PostStorage::create_post` /
`create_posts` **directly** (no slug-retry) so failure paths and exact HTML/slug stay under
test control. Owns its fields (no lifetime parameter — terminals emit an owned
`CreatePostInput`).

```rust
let post = SeedRawPost::new(uid).seed(&state).await;            // ceremony → SeededPost, expect()
let err  = SeedRawPost::new(bogus_uid).create(&state).await.unwrap_err(); // error path → Result
let input = SeedRawPost::new(uid).build();                      // batch → CreatePostInput (autogen slug/title resolved)
// read-back: no literal, reference what the post actually got
let post = SeedRawPost::new(uid).seed(&state).await;
assert_eq!(record.slug, post.slug);
```

**Defaults** (each field autogenerated or a sane constant):

| Field             | Default                                          |
| ----------------- | ------------------------------------------------ |
| `user_id`         | required arg to `new`                            |
| `slug`            | **autogenerated unique** `post-{n}`              |
| `title`           | **autogenerated unique** `Some("Post {n}")`      |
| `body`            | fixed non-empty Markdown                          |
| `format`          | `PostFormat::Markdown`                            |
| `rendered_html`   | derived `render(&body, &format)`                  |
| `published_at`    | `Some(Utc::now())` (**published**)                |
| `summary`         | `None`                                            |
| `audiences`       | `vec![AudienceTarget::Public]`                    |
| `idempotency_key` | `None`                                            |
| `tags`            | `vec![]`                                          |

- `slug` and `title` share one module-private monotonic counter (like `SeedUser`'s
  `SEED_SEQ`): seed *n* → `post-{n}` / `"Post {n}"`. Distinct + unique-per-`(user, day)`,
  so bare repeated seeds never collide; unique so `body.contains(title)` read-backs can't
  false-match. Resolved eagerly in `build()` so batch sites read `input.slug`/`input.title`
  off the held `CreatePostInput`.
- `rendered_html` stored `Option`; `build()` uses the override if set, else
  `render(&body, &format)`.
- **published default** matches the ~2:1 majority and #639's `SeedPost`.

**Setters — exactly the ones a real adopting site needs** (no speculative surface; `.title`,
`.idempotency_key`, **and `.rendered_html`** are **omitted** — the audits found no site that
chooses a title, sets an idempotency key, or supplies rendered HTML. Since the builder
renders `body` by default via the real `render()`, and no test forces a mismatched/verbatim
rendered_html, the field is always derived. This intentionally **departs from the issue's
"explicit rendered_html first-class" guidance**, which presupposed forced-mismatch tests
that do not exist in the current suite; add `.rendered_html()` with its regression test if
that need ever lands):

| Setter | Consumer(s) |
| --- | --- |
| `.slug(&str)` | conflict-sameness (shared local); any explicit-slug need |
| `.body(impl Into<PostBody>)` | `web_media` (media_url embed) |
| `.format(PostFormat)` | `post_format_column_round_trips_all_variants` |
| `.summary(PostSummary)` | `create_post_persists_summary` et al. |
| `.audiences(Vec<AudienceTarget>)` | `resolution_matrix`, audience round-trips, `feed_regenerate mk`, `web_posts`, backup named |
| `.draft()` / `.published_at(DateTime<Utc>)` | draft sites; fixed go-live/scheduled instants |
| `.tags(impl IntoIterator<Item = &str>)` | `feed_handlers`, `projector`, `backup`, `web_tags` happy path |

### Terminals

Three; the async two return a **`SeededPost`** (the read-back struct):

```rust
pub struct SeededPost {
    pub post_id: PostId,
    pub slug: Slug,
    pub title: PostTitle,                    // always Some (autogenerated) → non-optional here
    pub published_at: Option<DateTime<Utc>>, // None when .draft(); projector reads y/m/d for the permalink
    pub rendered_html: RenderedHtml,         // projector asserts the page embeds it
}
```

- **`.build(self) -> CreatePostInput`** — pure, no write; resolves the autogen slug/title.
  The two A batch tests and B `feed_regenerate` build a `Vec` from `.build()` (no plural
  terminal earns its place). `.build()` cannot apply `.tags()` — a `debug_assert!` guards a
  `.tags`-then-`.build()` mistake.
- **`.create(self, &state) -> Result<SeededPost, CreatePostError>`** — writes via
  `create_post`, applies any `.tags()` via `tag_post`, returns `SeededPost` on `Ok`. The
  error-path contract tests (`post_slug_conflict`, `foreign_key_violation`) call it and
  assert the `Err`.
- **`.seed(self, &state) -> SeededPost`** — `.create(…).expect(…)`; the routine happy-path
  case, like `SeedUser::seed`.

`SeededPost` fields are exactly the read-back set the audits found (`post_id`/`slug`/`title`
everywhere; `published_at` for projector's permalink; `rendered_html` for projector's page
assertion). `body` is never read back → excluded. No site needs a re-read `PostRecord`, so
the terminal does not fetch one.

### `.tags(…)` convenience

`tag_post(post_id, &TagLabel)` takes a `PostId`, so the writing terminals apply tags right
after the insert. Folds the create-then-`tag_post` two-step at `feed_handlers:110`,
`projector:43`, `backup_fixture:65`, and the `web_tags` happy-path helper. **Stays explicit**:
the `web_tags` clamp-limit bulk loops (create with `[]`, then add 60/20 tags — the loop *is*
the test) and the `web_posts` tags following an HTTP `create_post_json`.

### `BackupFixtureIds`

`misc/backup_fixture.rs` seeds two posts whose values feed a byte-for-byte NDJSON
comparison in `misc::backup_interop`. Proven autogen-safe: `populate_backup_fixture` runs
**once** and is threaded through backup→restore into every assertion (no golden file, no
two-backend divergence), so an autogenerated slug/title appears identically in both compared
dumps. Extend `BackupFixtureIds` with the seeded posts' `slug`/`title` (or embed the
`SeededPost`) so its `assert_backup_fixture_restored` reads them back instead of hardcoding
`"restored-post"`/`"Restored Post"`. `published_at` stays the shared fixed-instant helper
`fixture_published_at()` (µs-stable, a semantic `.published_at(instant)`).

## Adoption

- **List A**: delete both mod.rs factories; convert their sites, the tag/audience/format/
  summary literals, the ~14 posts.rs literals + `mk` closure, and the batch/rollback `Vec`s.
  Default is now *published* — the former draft-factory sites gain `.draft()`.
- **List B**: convert the storage-layer `state.posts.create_post` sites; extend
  `BackupFixtureIds`; keep the fixed-instant `.published_at(…)` and `.audiences(…)` sites;
  fold single/multi-tag happy paths into `.tags(…)`.
- Read-back sites reference `seeded.slug`/`seeded.title`/`seeded.published_at`/
  `seeded.rendered_html`; conflict-sameness uses a shared local slug.

## Behaviour-preservation hazards (must NOT silently drift to a default)

- **`web_media:263`** — the `.body(…)` must carry the test's `media_url`; the delete-media
  reference scan matches `body.contains(url) || rendered_html.contains(url)`, and the body
  carries it (the builder's default render of that body would too).
- **`misc/backup_fixture`** — every stored column is byte-compared; autogen is safe *because*
  populate runs once, but the fixed `fixture_published_at()` instant must remain (never
  `now()`), and the extended `BackupFixtureIds` read-back must reflect the actual seeded
  values.
- **`feed_worker:314,364`** and **`web_posts:1096`** — `published_at` is a specific
  go-live/scheduled instant; keep `.published_at(instant)`.
- **`create_rendered_post`/`update_rendered_post` bodies** — genuine literals, but on the
  render path, **out of scope** (do not touch).

## Acceptance criteria

- **AC1 — builder.** `storage::test_support` exports `SeedRawPost` (`new`, the setters
  above, `.build`/`.create`/`.seed`) and `SeededPost`; a bare
  `SeedRawPost::new(uid).seed(&state).await` persists one published, Public, Markdown post
  with autogenerated unique slug + title and returns them on `SeededPost`.
- **AC2 — literals gone.** No routine post-setup site in A or B hand-rolls a
  `CreatePostInput { … }`; the two mod.rs factories are deleted; read-back sites carry no
  slug/title literal (they reference `seeded.*`); the only surviving content literal is
  `web_media`'s `media_url` body/rendered.
- **AC3 — contract/fidelity tests surface only their subject.** Batch tests build via
  `.build()` and still assert order / whole-batch rollback; error-path tests use `.create`
  and assert the `Err`; the audience/format/summary tests carry only their semantic setter.
- **AC4 — behaviour-preserving.** The four hazards above are honoured; List B still seeds
  the exact instants / media_url it asserts on.
- **AC5 — self-tests.** `SeedRawPost` carries `#[apply(backends)]` tests (like `SeedUser`):
  defaults create a published Public Markdown post with distinct autogenerated slug+title
  across two seeds, and a default `rendered_html` equal to `render(body)`; `.draft`,
  `.slug`, `.body`, `.summary`, `.audiences`, `.tags`, `.format` overrides apply; `.create`
  surfaces `SlugConflict` on a forced duplicate.
- **AC6 — green.** `cargo xtask validate` passes; coverage non-regressing. Backup fidelity
  is exercised by **`misc::backup_interop`** (the module with the `#[test]`s), not
  `backup_fixture` (fixture-only).

## Notes

- **No ADR** — a test fixture convention, like `SeedUser` / `seed_posts` / #639's `SeedPost`.
- **Relationship to #639** — same axis, different layer/files; List B moved here from #639's
  plan review. #639 not yet merged; `SeedRawPost` mirrors the shipped `SeedUser`, so it
  does not hard-depend on #639 (only the `SeedPost` *name* is reserved — hence `SeedRawPost`).
- **Divergence from `SeedPost`**: `SeedRawPost` autogenerates the **title** (default `Some`),
  where `SeedPost`/`seed_post_input` default `None` — a deliberate choice so feed
  `body.contains(title)` sites go bare+read-back rather than carry title literals. If a
  future site needs an untitled post, add a `.untitled()` setter then (no current consumer).
