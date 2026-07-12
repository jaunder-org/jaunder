# Spec — atompub: idempotency key for post create (#79)

**Issue:** #79 (milestone #4). Prevent **duplicate-on-retry** when creating a
post: the client sends an `Idempotency-Key` on the create POST and the server
dedups on it, so a retry after a lost response returns the original post instead
of creating a second one. Matters most on flaky/mobile networks. Independent
follow-on.

Spans **server** (accept + dedup the key) and the **emacs client** (generate an
ephemeral key + auto-retry the create so the key is actually resent).

## Background (current behavior)

- `server/src/atompub/posts.rs::collection_post` (POST create) has **no
  `HeaderMap`** and no dedup: every POST calls `storage::perform_post_creation`
  and returns `201`. Media, by contrast, dedups by sha256
  (`server/src/atompub/media.rs`) via a unique constraint, returning `200` on a
  re-upload — the pattern to mirror.
- `perform_post_creation` (`storage/src/post_service.rs`) runs a slug-retry loop
  over `create_post`, which does `pool.begin()` → `INSERT … RETURNING post_id` →
  `commit` (`storage/src/posts.rs::write_post_in_tx`); a slug clash maps to
  `CreatePostError::SlugConflict` and retries.
- The emacs client (`elisp/jaunder-publish.el`) has **no retry**: on non-2xx it
  errors and leaves the file pristine. The create POST passes no extra headers.
- No idempotency mechanism, and **no GC/sweeper**, exist anywhere. Latest
  migration 0022.

## Why not persist the key in the buffer

A key persisted as a buffer property (`JAUNDER_IDEMPOTENCY_KEY`) is **wrong**:
if the create's response is lost (post `P1`/`v1` created server-side, client
never got the ID), the user edits to `v2` and re-publishes → the persisted key
dedups → the server returns the stale `v1` and the `v2` edit is silently
stranded. A static key conflates _"retry the same create"_ with _"publish my
edit."_ So the key must **not** outlive one publish attempt.

## Design (resolved)

### Client — ephemeral key + automatic retry (`elisp/jaunder-publish.el`)

On a **create** (no `JAUNDER_ID`):

- Generate **one** idempotency key per publish call, held in a local — no buffer
  property, nothing written to disk. Use a **self-contained** generator
  (`org-id` is not `require`d by the client, so `org-id-uuid` would
  `void-function` in a batch env): an opaque token from local entropy, e.g.
  `(md5 (format "%s%s%s" (random) (float-time) (emacs-pid)))` in a
  `jaunder--idempotency-key` helper.
- Send it as an `Idempotency-Key` request header on the create POST (the 5th
  `extra-headers` arg of `jaunder--http-request`, as the PUT path already uses
  for `If-Match`).
- **Auto-retry the create** with the **same** key: up to **3 total attempts**,
  retrying only **transient** failures — a signalled transport error
  (network/timeout) or an HTTP **5xx** `:status` — never a 4xx (won't succeed on
  retry) and never a 2xx (success). Backoff `~1s` then `~2s` (via `sleep-for`).
  On exhaustion, error as today.
- Because the key lives only for the call, the canonical case is covered without
  staleness: the first POST creates the post but the response is lost → the
  client's request errors → it retries with the same key → the server dedups →
  returns the original → write-back. The user cannot have edited mid-loop. A
  later manual re-publish gets a _fresh_ key and is a genuine new operation, so
  edits always go live.

Update (PUT) is out of scope for the retry/key (already idempotent via
`If-Match`).

### Server — atomic in-transaction dedup

- `collection_post` reads `headers.get("idempotency-key")` (opaque, trimmed,
  non-empty; ignored if absent → create as today). Adding a header extractor
  would make it an 8-arg handler; rather than suppress
  `clippy::too_many_arguments`, **bundle the four storage Extensions**
  (`PostStorage`, `SubscriptionStorage`, `UserConfigStorage`,
  `SiteConfigStorage`) into a single `PostServices` custom extractor
  (`impl FromRequestParts`, pulling each `Extension` in turn). The handler then
  takes `services: PostServices, auth_user, Path, headers, body` (5 args). Since
  `member_put` carries the **same** four storages (and today an
  `#[expect(clippy::too_many_arguments)]`), apply `PostServices` there too and
  **remove that existing suppression** — a small consistency cleanup so no post
  handler suppresses the lint.
- The key is threaded through the full create chain and registered **in the same
  transaction** as the post INSERT. Mind the existing types: `PostCreation<'a>`
  already has a lifetime so it takes `idempotency_key: Option<&'a str>`, but
  `CreatePostInput` (and `RenderedPostContent`) are **owned/lifetime-free** —
  they take `Option<String>`. The chain is `PostCreation` →
  `RenderedPostContent` → `render_post_input` → `CreatePostInput` →
  `write_post_in_tx` (via `create_rendered_post`); every other builder of these
  structs — the batch `seed_post_input` and all storage/handler tests — sets the
  new field to `None`.
- `write_post_in_tx`, after the post row is written, does
  `INSERT INTO idempotency_keys (user_id, key, post_id)`. Attribution is **by
  which statement's `.map_err` fires**, not by constraint introspection: a
  unique violation on the _idempotency_keys_ insert maps to a new
  `CreatePostError::IdempotencyConflict` (the post-insert violation still maps
  to `SlugConflict`). `IdempotencyConflict` is **not** retried by the slug loop
  — `perform_post_creation`'s exhaustive `match` gets a new non-retry arm
  propagating `PerformCreationError::IdempotencyConflict`.
- On that conflict the handler looks up the existing post via a new
  `PostStorage::post_id_for_idempotency_key(user_id, key) -> Option<i64>` (a
  plain generic `SELECT … WHERE user_id=$1 AND key=$2` on `PostStore<DB>` — like
  `get_post_by_id`, **no** per-dialect method needed), re-fetches it
  (`get_post_by_id`), and returns it with **`200 OK`** (same
  `Location`/`ETag`/body shape as the `201`). The 200 branch **skips
  `apply_categories`** — the original post already carries its tags; the retry's
  categories are dropped (consistent with "return the original"). A fresh key →
  post + key committed atomically, then `apply_categories` as today →
  **`201 Created`**.
- Wiring the new error variants: `PerformCreationError::IdempotencyConflict`
  needs an arm in **both** exhaustive `From` impls
  (`storage/src/post_service.rs` `→ InternalError` and
  `server/src/atompub/mod.rs` `→ HandlerError`) even though the handler
  intercepts the conflict before it reaches the `HandlerError` conversion.
- Concurrency: two overlapping same-key creates each run their own transaction;
  the second's key INSERT blocks on / then violates the unique row, so its whole
  transaction (post included) rolls back — no duplicate — and it returns the
  original.

### Storage

- New migration **`0023_create_idempotency_keys.sql`** in **both**
  `storage/migrations/sqlite` and `storage/migrations/postgres`, shaped like the
  existing per-user token tables:
  `idempotency_keys(id PK, user_id FK users, key TEXT, post_id FK posts, created_at, UNIQUE(user_id, key))`.
- `CreatePostError` / `PerformCreationError` gain an `IdempotencyConflict`
  variant.
- `post_id_for_idempotency_key` on the `PostStorage` trait + generic impl + both
  dialects.
- Keys are **kept indefinitely** (no sweeper exists). A GC/TTL follow-on issue
  is filed as the plan's first task.

### Semantics / scope

- **Same key + different content → return the original** (no request fingerprint
  stored). Our client never does this (the ephemeral key is fixed across one
  call's identical retries); it only guards a misbehaving client, for which
  returning the original is acceptable in v1.
- Header name **`Idempotency-Key`** (the de-facto standard; the
  `Slug`/`If-Match` HTTP-header precedent, not the server→client `j:` wire
  namespace).

## Acceptance criteria

**Server**

- **AC-S1 — dedup:** two POST creates with the same `Idempotency-Key` (same
  user) create **one** post; the second returns `200` with the first post's
  `Location` + `ETag` + body.
- **AC-S2 — fresh key:** a POST with a new `Idempotency-Key` returns `201` and
  creates the post (unchanged create behavior).
- **AC-S3 — no key:** a POST without the header creates as today (`201`), no key
  row.
- **AC-S4 — per-user scoping:** the same key string from two different users
  creates two independent posts (dedup is keyed on `(user_id, key)`).
- **AC-S5 — atomic/no duplicate:** the dedup is enforced by the DB unique
  constraint in the create transaction (a conflicting second create commits no
  post row) — asserted at least at the storage layer (a second
  `perform_post_creation` with a used key returns `IdempotencyConflict` and does
  not insert a post).
- **AC-S6 — backend parity:** all of the above run against SQLite and Postgres
  (`#[apply(backends)]`); the migration exists in both dialect dirs.

**Client**

- **AC-C1 — key sent:** a create POST carries an `Idempotency-Key` header; the
  value is a non-empty token; **no** `JAUNDER_IDEMPOTENCY_KEY` buffer property
  is written.
- **AC-C2 — same key across retries:** within one `jaunder-publish` call, every
  retry sends the **same** key value.
- **AC-C3 — retry on transient only:** a transport error or a `5xx` status
  triggers a retry (up to 3 attempts total); a `4xx` does **not** retry (errors
  immediately); a `2xx` returns without retrying.
- **AC-C4 — success after retry:** a first attempt that fails transiently
  followed by a `2xx`/dedup response completes the publish (write-back happens)
  without error.
- **AC-C5 — exhaustion:** after the max attempts all fail transiently, the
  publish errors (as today) and leaves the on-disk file pristine.
- **AC-C6 — fresh key per call:** two separate `jaunder-publish` create calls
  use **different** keys.

## Non-goals

- No buffer-persisted / cross-session / content-bound key (rejected above); a
  whole-call failure after the server committed the post, followed by a _manual_
  re-publish, can still duplicate — acceptable, and an edit then means a new
  version anyway.
- No retry or idempotency key on **update** (PUT is already idempotent via
  `If-Match`).
- No `Idempotency-Key` request-fingerprint / `409` on key reuse with different
  content.
- No key GC/TTL in this cycle (follow-on issue).

## Testing

- **Storage (`storage/src/post_service.rs` `mod tests`, `#[apply(backends)]`):**
  a second `perform_post_creation` with a used key → `IdempotencyConflict` and
  no new post row (AC-S5); `post_id_for_idempotency_key` returns the mapping;
  per-user scoping (AC-S4).
- **Handler integration (`server/tests/atompub/atompub_posts.rs`,
  `#[apply(backends)]`):** same-key second POST → `200` + original `Location`
  (AC-S1); fresh key → `201` (AC-S2); no header → `201` (AC-S3). Mirror
  `create_post_returns_201_and_is_retrievable`.
- **Client (ERT, `elisp/test/jaunder-test.el`):** stub `jaunder--http-request`
  to (a) return a `5xx` then a `201` and assert one retry with the same captured
  `Idempotency-Key` + success (AC-C2/C3/C4); (b) return `4xx` and assert no
  retry (AC-C3); (c) exhaust with transient failures and assert the publish
  errors (AC-C5). Assert no `JAUNDER_IDEMPOTENCY_KEY` property is written
  (AC-C1) and two calls differ (AC-C6). Stub `sleep-for` so tests don't actually
  wait.

Follow `CONTRIBUTING.md` (backend parity, dual-backend storage tests, dialect
files, coverage policy); gate is `cargo xtask check`, `validate` at ship. Client
tests run under the elisp `ert` step.
