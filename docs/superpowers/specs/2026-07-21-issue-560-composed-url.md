# Spec — #560: type composed feed/post URLs (require-base + root-relative newtype)

**Issue:** jaunder-org/jaunder#560 · **Milestone:** #13 Domain-value type safety
(newtypes) · **Follows:** #448 (`AbsoluteUrl` + `compose` seam).

## Goal

Type the composed feed/post URLs that #448 left as `String`. Per the design
interview, `base_url` is now **required** for every composed feed/atompub URL
(all `atom:id` surfaces must be absolute, RFC-4287), which **dissolves the
absolute-or-relative sum type** the issue originally envisioned: every composed
URL is simply an `AbsoluteUrl`. A new `RootRelativeUrl` newtype types the
genuinely always-relative fields — the web post permalinks the browser resolves
against the current origin.

## Decisions (from the interview)

- **D1 — require `base_url` for all composed feed/atompub URLs.** Every value
  produced by the `compose` seam becomes `AbsoluteUrl`; the feed and atompub
  endpoints **and the feed-regeneration worker** **error when `base_url` is
  unset** rather than silently emitting root-relative URLs. This makes every
  *composed* URL absolute — the feed/collection `<id>`, `self`, `first`, `next`,
  the atompub `edit`/media URLs, **and the per-item feed URLs** (atom
  `Entry.id`/`<link>`, RSS `<link>`/`guid`, JSON item `url`). The last are now
  composed from `base` during feed regeneration (`FeedItem.permalink` →
  `AbsoluteUrl`) instead of emitted raw-relative, so **no feed/atompub surface
  emits a relative URL.** (Per the interview: the pre-existing relative
  `Entry.id`s were never valid stable IRIs, so making them absolute carries no
  migration burden.)
- **D2 — no sum type.** Because every composed URL is now absolute, the
  `ComposedUrl`/absolute-or-relative enum is **not built**. This is a deliberate
  divergence from the issue's original "sum type" direction, made possible by D1.
- **D3 — `RootRelativeUrl` newtype** for the always-root-relative fields — the
  **web** post `permalink`/`preview_url`/`edit_url` on the WASM-wire post DTOs,
  which the browser resolves against the current origin and are never composed.
  (The *feed* `FeedItem.permalink` is **not** one of these — under D1 it is
  composed to `AbsoluteUrl`.) Per ADR-0063 this is a distinct grammar from both
  `AbsoluteUrl` and `FeedPath`; **do not fold onto `FeedPath`** (that is a closed
  feed-endpoint identity newtype and an *input* to `compose`, not a URL output).
- **D4 — include the web post fields.** The always-relative post
  `permalink`/`preview_url`/`edit_url` on the WASM-wire post DTOs are typed
  `RootRelativeUrl` in this issue (not split to a follow-up), with the serde +
  `Deref<str>`/`Display` surface their Leptos `href=`/`window.location` consumers
  need.
- **D5 — query params via `url`, not `format!`.** The `FeedMeta.next` cursor URL
  is built with a new `AbsoluteUrl::with_query_pairs` method backed by
  `url::Url::query_pairs_mut()` (correct percent-encoding), replacing the current
  `format!("{collection_url}?updated_before=…&id_before=…")` string concatenation.

## Context / as-built (see #560 recon)

- **The seam:** `common/src/absolute_url.rs:76` —
  `compose(base: Option<&AbsoluteUrl>, path: &str) -> String`: absolute
  (`base.join(path)`) when `Some`, the untouched root-relative `path` when
  `None`. Every composed-URL producer calls it; producers do not branch. `join`
  (`:47`) re-validates through `FromStr`.
- **`AbsoluteUrl`** (`absolute_url.rs:22`, `#[derive(StrNewtype)]`) validates
  `http`/`https` via the `url` crate and canonicalizes; supplies the
  `Display`/`Deref<str>`/`AsRef`/`TryFrom<String>`/serde trailer.
- **`site.base_url`** is already `Option<AbsoluteUrl>` (`common/src/site.rs:6`);
  unset ⇒ `compose` returns root-relative today.
- **Fields, all currently `String`:**
  - Syndication `FeedMetadata.self_url`/`canonical_url`
    (`common/src/feed/metadata.rs:11`), produced by `compose` in
    `server/src/feed/regenerate.rs:69,79`.
  - AtomPub `FeedMeta.id`/`self_url`/`first`/`next`/`previous`
    (`common/src/atompub/entry.rs:524`), produced in
    `server/src/atompub/posts.rs:167-199`. `next` is the query-cursor (D5).
    `previous` is always `None` here.
  - AtomPub `MediaLinkEntry.id`/`edit_uri`/`edit_media_uri`/`content_src`
    (`entry.rs:588`), produced in `server/src/atompub/media.rs:31-47`.
  - Post `permalink`/`preview_url`/`edit_url` on `CreatePostResult`,
    `UpdatePostResult`, `DraftSummary`, `PublishPostResult`, `PostResponse`,
    `PostSummary` (`web/src/posts/api.rs`, `api/listing.rs`); produced by
    `PostRecord::permalink()` (`storage/src/posts.rs:80`) and
    `format!("/draft/{id}/preview")`.
  - `FeedItem.permalink` (`metadata.rs:26`), produced by `p.permalink()`
    (always relative).

## Design

### New type: `RootRelativeUrl`

A `StrNewtype`-derived newtype (mirroring `AbsoluteUrl`/`FeedPath`) over a
validated host-less root-relative reference.

- **Invariant:** starts with `/`, no scheme/authority, optional query; parses as
  a relative reference against a dummy base (via the `url` crate) and re-emits
  canonical form. Rejects absolute URLs (has scheme/host), protocol-relative
  `//host`, and anything not `/`-rooted.
- **Trailer (via `#[derive(StrNewtype)]`):** `Display`, `Deref<str>`/`AsRef`/
  `Borrow`, `TryFrom<String>`, `From<Self> for String`, `PartialEq<str>`,
  validating serde bridge — everything the feed renderers (`&str` out) and the
  Leptos consumers (`href=`, `escape_html`, `window.location.replace`) need.
- Lives in `common/src/root_relative_url.rs`, re-exported like `AbsoluteUrl`.

### `AbsoluteUrl` additions

- `with_query_pairs(&self, pairs: &[(&str, &str)]) -> AbsoluteUrl` — parses self
  (always a valid `url` by construction), `query_pairs_mut().extend_pairs(pairs)`,
  re-emits canonical. Used for the `next` cursor (D5).

### `compose` requires a base — the type, not a runtime check

`compose` must not take an `Option` and fail at runtime when it's `None` — that
is the exact "validate at every use site" anti-pattern this milestone exists to
kill. Make the base a **type precondition**: `compose` takes a non-optional
`&AbsoluteUrl` and is **infallible**.

```rust
pub fn compose(base: &AbsoluteUrl, path: &str) -> AbsoluteUrl;
```

- `base.join(path)` for a valid `AbsoluteUrl` base and a server-built `/…` path
  cannot fail; a join failure would be a genuine bug, treated as unreachable
  (the current `compose` already marks its error arm `cov:ignore unreachable`).
- Keeps `&str` for `path` (call sites hold server-built `/…` strings, a
  `&FeedPath`, or a `format!` result — never a `RootRelativeUrl`).

The `Option<AbsoluteUrl> → &AbsoluteUrl` narrowing happens **once**, at each
feed/atompub handler and the regeneration worker's entry, *before* any
composition:

```rust
let base = identity.base_url.as_ref().ok_or(BaseUrlRequired)?;   // single guard
// …every downstream `compose(base, path)` is infallible…
```

So "you cannot compose a URL without a base" is enforced by the signature, and
the missing-base error is raised exactly once per request/regeneration at the
boundary — **not** threaded as a `Result` through the ~12 compose call sites
(`regenerate.rs:69,79`, `atompub/posts.rs:167,452`, `atompub/media.rs:31,36`,
`atompub/mapping.rs:142,157`, `atompub/service.rs:45,51`, `rsd.rs:35,37`,
`worker.rs:206`, `invites/mod.rs:68`, `commands.rs:309`). The invite/CLI sites
already hold a non-optional `base_url`, so they pass `&base_url` directly with no
guard.

### Error surface when `base_url` is unset (the single guard)

The guard (`ok_or(BaseUrlRequired)`) fires at two kinds of entry:

- **Endpoints** (atompub collection/member/media handlers) map it to a clear,
  uniform error — **proposed** an `InternalError`-class failure with a
  "site.base_url must be configured to serve feeds/atompub" message (HTTP 500).
- **Feed regeneration** (`regenerate.rs`, returns `RegenerateError`, driven by
  the **background worker**): add a `RegenerateError::BaseUrlRequired` variant
  raised by the entry guard; the worker logs/propagates it like any other
  regeneration failure (no feed row is produced). The WebSub ping
  (`worker.rs:206`) sits behind the same guard.

The existing unit test
`regenerate_site_feed_falls_back_to_relative_urls_without_base`
(`regenerate.rs:222-271`) asserts the *old* relative-fallback and is **inverted**
to assert the new `BaseUrlRequired` error.

### Test & e2e migration (required by D1)

`base_url` defaults to `None`, and the existing `end2end/tests/feeds.spec.ts`
and `atompub.spec.ts` currently run **without** it and assert success — under D1
they would fail. This issue must:

- **Seed `site.base_url`** (a valid absolute URL) in the feed/atompub e2e setup
  so those suites exercise the now-required-base path and stay green.
- Add **negative coverage** (base unset → feed/atompub error) per AC5.
- Invert the `regenerate` relative-fallback unit test (above) and audit
  server/integration tests that build feeds/atompub without base for the same
  migration.

*(Open for spec-approval: whether a more specific status/user-facing config error
is warranted — see below.)*

## Retype summary

| Field(s) | New type |
| --- | --- |
| `FeedMetadata.self_url`, `canonical_url` | `AbsoluteUrl` |
| `FeedMeta.id`, `self_url`, `first`, `next`, `previous` | `AbsoluteUrl` (`next` via `with_query_pairs`) |
| `MediaLinkEntry.id`, `edit_uri`, `edit_media_uri`, `content_src` | `AbsoluteUrl` |
| post `permalink`, `preview_url`, `edit_url` (web DTOs) | `RootRelativeUrl` |
| `FeedItem.permalink` (feed DTO) | `AbsoluteUrl` — composed from `base` in `regenerate.rs` (`compose(base, &record.permalink())?`); the atom `Entry.id`/`<link>`, RSS `<link>`/`guid`, JSON item `url` all render this absolute value |

External-crate boundaries (`atom_syndication::Feed/Entry/Link`, `rss`) still take
`String` — convert with `.into()`/`.to_string()` at the boundary (recon §7). The
pure feed renderers (`atom.rs`/`rss.rs`/`json.rs`) stay pure — they receive the
already-composed `AbsoluteUrl` values on `FeedMetadata`/`FeedItem`, so no `base`
threads into the renderer.

## Acceptance criteria (observable)

1. `common::RootRelativeUrl` exists with the `StrNewtype` trailer; rejects
   absolute/`//host`/non-`/`-rooted strings (unit tests), accepts `/~a/b` (and an
   optional query, though the fields it types don't currently carry one).
2. `AbsoluteUrl::with_query_pairs` produces a correctly percent-encoded query
   (unit test: a value containing `&`/space/`=` round-trips encoded).
3. `compose(base: &AbsoluteUrl, path: &str) -> AbsoluteUrl` takes a **required**
   base (no `Option`, no `Result`) and is infallible; the missing-base error is
   raised by a **single `ok_or(BaseUrlRequired)` guard** at each feed/atompub
   handler and the regen worker entry (unit/integration asserts the guard errors
   when `base_url` is unset). No code path emits a root-relative composed URL.
4. Every field in the retype table has the new type in source (grep/inspection);
   no composed feed/atompub URL field is `String`.
5. **Feeds/atompub require base:** with `base_url` unset, requesting a feed
   (`/feed.atom`) or an atompub collection returns the error (integration/e2e
   asserts non-OK); with `base_url` set, the same requests succeed and **every**
   emitted URL — feed/collection `<id>`/`self`/`first`/`next`, atompub
   `edit`/media, **and every per-item atom `Entry.id`/`<link>`, RSS
   `<link>`/`guid`, JSON item `url`** — is absolute (test asserts no emitted URL
   begins with `/`).
6. The `next` cursor carries the same cursor values: **decode** `updated_before`
   and `id_before` from both the old and the new URL and assert equal values with
   pair order preserved (the *encodings* differ — `query_pairs_mut` uses
   form-urlencoding vs. today's `NON_ALPHANUMERIC` percent-encoding — so assert
   decoded equivalence, not byte-identity). No `format!` query concat remains in
   `atompub/posts.rs`.
7. Post `permalink`/`preview_url` round-trip over the server↔WASM wire as
   `RootRelativeUrl` (existing posts e2e green; Leptos `href=`/redirect
   consumers unchanged).
8. The feed/atompub e2e (`feeds.spec.ts`, `atompub.spec.ts`) seed `site.base_url`
   and stay green; the inverted `regenerate` unit test and any base-less
   integration tests are migrated.
9. `cargo xtask validate --no-e2e` green; CI e2e matrix green.

## Open for spec-approval (flagged judgment calls)

- **`id` grammar narrowing (`FeedMeta.id`, `MediaLinkEntry.id`).** An atom
  `<id>` is an IRI, so `tag:`/`urn:` are valid (a `render_feed` unit test uses
  `id: "tag:example.com,2026:…"`, `entry.rs:954`). Production composes the http
  collection/edit URL as the id, so typing `id` as `AbsoluteUrl` matches actual
  usage but **forecloses non-http ids** and invalidates that test literal.
  **Proposed:** type `id` as `AbsoluteUrl` and update the test to an http id
  (matching production). *Accept, or keep `id` a broader IRI type?*
- **Error surface/status** when `base_url` unset (500 vs. a specific status/
  config error) — confirm the proposed 500 + message.

## Related follow-ups

- **#575** — a "site.base_url not configured" admin warning banner (mirroring the
  backup-not-configured banner), the friendly surface for the require-base
  behavior this issue introduces. Ships independently.

## Out of scope

- The absolute-or-relative sum type (dissolved by D1).
- Same-origin enforcement on `join` (a documented pre-existing `AbsoluteUrl`
  limitation).
- Non-URL atompub fields (`title`, timestamps, `content_type`).
