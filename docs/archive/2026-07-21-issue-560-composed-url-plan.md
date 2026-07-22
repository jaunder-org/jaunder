# Type composed feed/post URLs (#560) — Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate**
> (delegating a task to a subagent via **jaunder-dispatch** when useful — Task 3
> is a good candidate). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Type the composed feed/post URLs #448 left as `String`, requiring
`site.base_url` for every composed feed/atompub URL (so no relative `atom:id` is
ever emitted) and typing the always-relative web post URLs with a new
`RootRelativeUrl` newtype.

**Architecture:** `base_url` becomes a _type_ precondition of composition:
`compose(base: &AbsoluteUrl, path: &str) -> AbsoluteUrl` (infallible), with the
`Option → &AbsoluteUrl` narrowing done once per request/regeneration at the
handler/worker entry (`ok_or(BaseUrlRequired)`). Composed URL fields become
`AbsoluteUrl`; the web post DTO URLs become `RootRelativeUrl`. See the spec for
the field inventory and decisions.

**Tech Stack:** Rust, `url` crate, `macros::StrNewtype` (ADR-0063), Leptos
server fns + components, Playwright e2e.

**Spec:** `docs/superpowers/specs/2026-07-21-issue-560-composed-url.md`.

## Global Constraints

- **Illegal states unrepresentable (the milestone's point).** No `Option`-base
  runtime check inside `compose`; the guard is a single `ok_or` at each entry.
- **New types via `#[derive(StrNewtype)]`** (ADR-0063) like
  `AbsoluteUrl`/`FeedPath` — get the
  `Display`/`Deref<str>`/`AsRef`/`TryFrom<String>`/`From<Self> for String`/`PartialEq<str>`/
  serde trailer for free; don't hand-roll it.
- **Newtypes used pervasively** (#470): retype every field/producer/consumer of
  a value, incl. serialization surfaces; build test values via
  `common::test_support::parse_*`.
- **Feeds/atompub require `base_url`** — a behavior change; the feed/atompub e2e
  must seed it (Task 5) or CI e2e fails.
- **`id` fields** (`FeedMeta.id`, `MediaLinkEntry.id`) → `AbsoluteUrl`
  (approved; update the `tag:` test literal to an http id).
- Commit via **jaunder-commit** (pre-commit runs `cargo xtask check`). **No
  `Co-Authored-By`.**

---

## Review header

**Scope (in):** `RootRelativeUrl` newtype; `AbsoluteUrl::with_query_pairs`;
`compose` require-base signature + single-guard error surface; retype composed
feed/atompub URL fields to `AbsoluteUrl` (+ compose `FeedItem.permalink` so
per-item `atom:id`/links are absolute); retype web post
`permalink`/`preview_url`/`edit_url` to `RootRelativeUrl`; migrate feed/atompub
e2e + the `regenerate` fallback test.

**Scope (out):** the absolute-or-relative sum type (dissolved); same-origin
`join` enforcement; the #575 warning banner.

**Tasks:**

1. `RootRelativeUrl` newtype (common, foundation).
2. `AbsoluteUrl::with_query_pairs` (common, foundation).
3. Require-base `compose` + feed/atompub URL typing (common DTOs + renderers +
   server seam + error/guards) — the large atomic seam change.
4. Web post-DTO URL typing → `RootRelativeUrl` (storage `permalink()` + web
   wire + Leptos).
5. Feed/atompub e2e + test migration (seed `base_url`, negative coverage).

**Key risks:**

- Task 3 is **atomic across common+server** — the `compose` signature +
  return-type change forces all producers and the composed-URL DTO fields
  together in one commit.
- Requiring `base_url` breaks the existing base-less feed/atompub e2e and the
  `regenerate` fallback unit test — Task 5 (+ Task 3's inverted unit test)
  migrate them.

---

## Task 1: `RootRelativeUrl` newtype

**Files:**

- Create: `common/src/root_relative_url.rs`
- Modify: `common/src/lib.rs` (module decl + re-export, mirroring
  `absolute_url`)
- Modify: `common/src/test_support.rs` (add `parse_root_relative_url` helper, if
  the crate's convention has per-newtype parse helpers — mirror `parse_*` for
  `AbsoluteUrl`/`FeedPath`)

**Interfaces:**

- Produces: `common::RootRelativeUrl` — `#[derive(StrNewtype)]` newtype over a
  validated host-less root-relative reference; error `InvalidRootRelativeUrl`.

- [x] **Step 1: Write the failing validation tests** (`#[cfg(test)]` in the new
      module):

```rust
#[test] fn accepts_root_relative_paths() {
    for s in ["/", "/~alice/2026/01/02/hello", "/draft/5/preview", "/a/b?x=1"] {
        assert_eq!(s.parse::<RootRelativeUrl>().unwrap().as_ref(), s);
    }
}
#[test] fn rejects_non_root_relative() {
    for s in ["", "foo", "foo/bar", "//evil.com/x", "https://x/y", "http://h", "mailto:a@b"] {
        assert!(s.parse::<RootRelativeUrl>().is_err(), "should reject {s:?}");
    }
}
```

- [x] **Step 2: Run, verify FAIL** —
      `cargo nextest run -p common root_relative_url` (type undefined).
      Expected: FAIL (compile).

- [x] **Step 3: Implement `RootRelativeUrl`** to satisfy the tests.
      Signature/skeleton:

```rust
use macros::StrNewtype;

/// A validated host-less, root-relative URL reference (`/…`, optional query) — the
/// browser-resolved web URLs (post permalinks) that are never composed against a base.
/// Distinct grammar from `AbsoluteUrl` (has scheme+host) and `FeedPath` (closed feed
/// endpoint set); see ADR-0063.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct RootRelativeUrl(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("must be a root-relative URL beginning with '/'")]
pub struct InvalidRootRelativeUrl;

impl std::str::FromStr for RootRelativeUrl {
    type Err = InvalidRootRelativeUrl;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        // Root-relative, not protocol-relative, not absolute.
        if !s.starts_with('/') || s.starts_with("//") {
            return Err(InvalidRootRelativeUrl);
        }
        // Resolve against a dummy base; reject anything that changes host/scheme.
        let base = url::Url::parse("https://root-relative.invalid").map_err(|_| InvalidRootRelativeUrl)?;
        let resolved = base.join(s).map_err(|_| InvalidRootRelativeUrl)?;
        if resolved.host_str() != Some("root-relative.invalid") || resolved.scheme() != "https" {
            return Err(InvalidRootRelativeUrl);
        }
        // Re-emit canonical path+query (host-less).
        let mut out = resolved.path().to_owned();
        if let Some(q) = resolved.query() { out.push('?'); out.push_str(q); }
        Ok(Self(out))
    }
}
```

(The exact `from_str` body is pinned by the Step-1 tests — every branch, accept
and each reject, is covered; the `StrNewtype` derive supplies the rest of the
trailer. If `StrNewtype` requires `FromStr` be generated rather than
hand-written, follow the derive's convention as `AbsoluteUrl` does — check
`common/src/absolute_url.rs` for the exact shape.)

- [x] **Step 4: Run, verify PASS** — same command. Expected: PASS.

- [x] **Step 5: Wire the module** (`lib.rs`) — `parse_root_relative_url` test
      helper deferred to Task 4 (its first use), to avoid an uncovered unused
      helper. (`lib.rs`, `test_support.rs`), then
      `cargo clippy -p common --all-targets`. Expected: PASS.

- [x] **Step 6: Commit** (`jaunder-commit`):
      `feat(common): RootRelativeUrl newtype (#560)`.

---

## Task 2: `AbsoluteUrl::with_query_pairs`

**Files:** Modify `common/src/absolute_url.rs` (+ its `#[cfg(test)]`).

**Interfaces:**

- Produces:
  `AbsoluteUrl::with_query_pairs(&self, pairs: &[(&str, &str)]) -> AbsoluteUrl`.

- [x] **Step 1: Write the failing test:**

```rust
#[test] fn with_query_pairs_encodes_and_appends() {
    let base: AbsoluteUrl = "https://ex.com/atompub/alice/posts".parse().unwrap();
    let out = base.with_query_pairs(&[("updated_before", "2026-01-02T03:04:05Z"), ("id_before", "5")]);
    let parsed = url::Url::parse(out.as_ref()).unwrap();
    let got: Vec<(String, String)> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    assert_eq!(got, vec![
        ("updated_before".to_string(), "2026-01-02T03:04:05Z".to_string()),
        ("id_before".to_string(), "5".to_string()),
    ]);
    // A value with reserved chars round-trips *decoded*.
    let out2 = base.with_query_pairs(&[("q", "a b&c=d")]);
    let p2 = url::Url::parse(out2.as_ref()).unwrap();
    assert_eq!(p2.query_pairs().next().unwrap().1.into_owned(), "a b&c=d");
}
```

- [x] **Step 2: Run, verify FAIL** —
      `cargo nextest run -p common with_query_pairs`. FAIL.

- [x] **Step 3: Implement:**

```rust
#[must_use]
pub fn with_query_pairs(&self, pairs: &[(&str, &str)]) -> AbsoluteUrl {
    // `self` is a valid URL by construction, so parse cannot fail.
    let mut url = url::Url::parse(&self.0).expect("AbsoluteUrl holds a valid url"); // cov:ignore
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    Self(url.to_string())
}
```

- [x] **Step 4: Run, verify PASS.**
- [x] **Step 5: Commit** (`jaunder-commit`):
      `feat(common): AbsoluteUrl::with_query_pairs (#560)`.

---

## Task 3: Require-base `compose` + feed/atompub URL typing (atomic; dispatch-suitable)

Changes `compose`'s signature (breaking all callers) and retypes every composed
feed/atompub URL field — one commit spanning `common` + `server`. **Verify
current line numbers by grep before editing** (`#574` rebased in).

**Files:**

- Modify `common/src/absolute_url.rs` (`compose` + `BaseUrlRequired`),
  `common/src/feed/metadata.rs` (`FeedMetadata`/`FeedItem` URL fields),
  `common/src/atompub/entry.rs` (`FeedMeta`/`MediaLinkEntry` fields + XML
  writers), `common/src/feed/{atom,rss,json}.rs` (renderer boundaries + tests).
- Modify `server/src/feed/regenerate.rs` (guard + compose
  `self_url`/`canonical_url` + **compose `FeedItem.permalink`** +
  `RegenerateError::BaseUrlRequired` + invert fallback test),
  `server/src/atompub/posts.rs` (guard + `collection_url` + `next` via
  `with_query_pairs`), `server/src/atompub/media.rs` + `mapping.rs` +
  `service.rs` + `rsd.rs` (guards + compose), `server/src/feed/worker.rs` (guard
  for the WebSub ping), `server/src/commands.rs` + `web/src/invites/mod.rs`
  (pass `&base_url` directly — already non-optional).
- Modify the atompub handlers' entry points (the `#[server]`/axum handlers) to
  `ok_or` the base and map to the error surface.

**Interfaces:**

- `compose(base: &AbsoluteUrl, path: &str) -> AbsoluteUrl` (infallible, required
  base).
- `struct BaseUrlRequired` (common); `RegenerateError::BaseUrlRequired`
  (server).
- `FeedMetadata.self_url`/`canonical_url`: `AbsoluteUrl`; `FeedItem.permalink`:
  `AbsoluteUrl`.
- `FeedMeta.id`/`self_url`/`first`/`next`/`previous`: `AbsoluteUrl`
  (`next`/`previous` `Option`).
- `MediaLinkEntry.id`/`edit_uri`/`edit_media_uri`/`content_src`: `AbsoluteUrl`.

- [x] **Step 1: Change `compose` + add `BaseUrlRequired`** (`absolute_url.rs`).
      New tests (replace the old `compose` tests):

```rust
#[test] fn compose_joins_against_required_base() {
    let base: AbsoluteUrl = "https://ex.com".parse().unwrap();
    assert_eq!(compose(&base, "/feed.atom").as_ref(), "https://ex.com/feed.atom");
    assert_eq!(compose(&base, "/~a/2026/01/02/x").as_ref(), "https://ex.com/~a/2026/01/02/x");
}
```

```rust
pub fn compose(base: &AbsoluteUrl, path: &str) -> AbsoluteUrl {
    base.join(path).unwrap_or_else(|_| unreachable!("valid base + server path")) // cov:ignore
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("site.base_url must be configured to serve feeds and AtomPub")]
pub struct BaseUrlRequired;
```

- [x] **Step 2: Retype the composed-URL DTO fields** to `AbsoluteUrl`
      (metadata.rs, entry.rs). The hand-rolled XML writers consume via
      `&str`/`as_str()` — `Deref<str>` keeps them compiling; adjust any
      `&String` bindings. Update in-file test constructors from `.into()`/
      `.to_string()` literals to `parse::<AbsoluteUrl>().unwrap()` (or
      `parse_absolute_url`), incl. the `render_feed` `id:` literal → an **http**
      id (drop the `tag:` literal).

- [x] **Step 3: Add the entry guards + retype producers** (server). At each
      atompub handler and the regen worker entry:

```rust
let base = base_url().ok_or(/* handler: InternalError; worker: RegenerateError::BaseUrlRequired */)?;
```

Then every `compose(Some(&base_url), p)?`/`compose(base, p)` call becomes
`compose(base, p)` (infallible), and
`FeedItem.permalink = compose(base, &record.permalink())` (NEW — composes the
per-item permalink to absolute). **This requires threading `base` into
`build_feed_items` (`regenerate.rs`), which today takes `(posts, records)` with
no base — extend its signature.** Build `next` with
`collection_url.with_query_pairs(&[("updated_before", …), ("id_before", …)])`
(drop the `format!`). Add `RegenerateError::BaseUrlRequired`.

- [x] **Step 4: Invert the fallback unit test.** `regenerate.rs`'s
      `regenerate_site_feed_falls_back_to_relative_urls_without_base` (asserts
      old relative fallback) → assert the base-less regeneration returns
      `Err(RegenerateError::BaseUrlRequired)`. Rename accordingly. Audit sibling
      regenerate/atompub unit tests that build feeds without base.

- [x] **Step 5: Renderers** (`atom.rs`/`rss.rs`/`json.rs`) — they now receive
      `AbsoluteUrl` values on `FeedMetadata`/`FeedItem`; convert to `String`
      only at the external-crate boundary (`atom_syndication`/`rss`) via
      `.to_string()`/`.into()`. Update their `#[cfg(test)]` constructors (build
      `AbsoluteUrl`s, assert emitted `<id>`/`<link>` are absolute).

- [x] **Step 6: Add a base-required integration test** (server, package
      `jaunder`): request a feed/atompub endpoint with `site.base_url` **unset**
      → assert the handler errors (non-OK); with it set → assert success and
      that emitted `<id>`/`self`/per-item URLs are absolute (no emitted URL
      starts with `/`). Use the server test harness (see `CONTRIBUTING.md`;
      raw-SQL seed recipe for site config in `project_server_crate_package_name`
      conventions).

- [x] **Step 7: Build the whole tree.**
      `cargo clippy -p common -p jaunder --all-targets` then
      `cargo clippy -p web --target wasm32-unknown-unknown` (invite stub).
      Expected: PASS.

- [x] **Step 8: Run the affected tests.** `cargo nextest run -p common` +
      `cargo nextest run -p jaunder feed atompub regenerate`. Expected: PASS.

- [x] **Step 9: Commit** (`jaunder-commit`):
      `refactor(feed,atompub): require base_url; type composed URLs as AbsoluteUrl (#560)`.

---

## Task 4: Web post-DTO URL typing → `RootRelativeUrl`

Retype the always-relative web post URLs. Atomic within `web` (+ the `storage`
producer).

**Files:**

- Modify `storage/src/posts.rs` (`PostRecord::permalink()` → `RootRelativeUrl`;
  its `assert_eq!(…, "/~…")` unit tests, ~`:2419`, compare against `&str` → use
  `.as_ref()` / `*val` or `parse_root_relative_url`).
- Modify `web/src/posts/api.rs` + `api/listing.rs`
  (`permalink`/`preview_url`/`edit_url` on
  `CreatePostResult`/`UpdatePostResult`/`DraftSummary`/`PublishPostResult`/`PostResponse`/
  `PostSummary` → `RootRelativeUrl`),
  `web/src/posts/{server,parse,render,component}.rs` (producers/consumers), and
  any timeline DTO carrying a post URL.
- **Fallible-construction sites to update** (`RootRelativeUrl` has no infallible
  `From<&str>` — only `TryFrom<String>`/`FromStr`): `api.rs:653` (`"…".into()`),
  `parse.rs:153-155` (`.to_string()` client-side build →
  `parse().expect()`/`parse_root_relative_url`), and the `assert_eq!(…, "/~…")`
  at `api.rs:759`.
- **Not in this file set but stay compiling** (deref-coerce
  `&RootRelativeUrl → &str` into `compose`): `server/src/feed/regenerate.rs:137`
  and `server/src/atompub/mapping.rs:154` consume `permalink()` — spot-check
  they still build after the return-type change.

**Interfaces:**

- `PostRecord::permalink(&self) -> RootRelativeUrl` — re-parse its own
  known-valid `format!` result with
  `.parse().expect("permalink() builds a valid /… path")` + `cov:ignore` on the
  unreachable arm (matches the `AbsoluteUrl` precedent — no `from_trusted`
  door).
- Post DTO URL fields: `RootRelativeUrl` (keep `Serialize + Deserialize`).

- [x] **Step 1: Write the failing wire test** (`web/src/posts/api.rs` tests): a
      post DTO with a `RootRelativeUrl` permalink round-trips over serde*json,
      and a decode of an \_absolute* permalink string is rejected (`is_err()`) —
      pins the wire type.

- [x] **Step 2: Run, verify FAIL to compile** (fields still `String`).

- [x] **Step 3: Retype `PostRecord::permalink()`** → `RootRelativeUrl`
      (`storage/src/posts.rs`), and
      `preview_url = format!("/draft/{id}/preview")` producers →
      `RootRelativeUrl`.

- [x] **Step 4: Retype the web DTO fields + producers** (`api.rs`, `server.rs`,
      `parse.rs`) — assign the typed values; `parse.rs`'s client-side
      construction builds a `RootRelativeUrl` (parse the literal, or the trusted
      door).

- [x] **Step 5: Adapt Leptos consumers** (`component.rs`, `render.rs`): `href=`
      and `escape_html` accept `&str` via `Deref`/`Display`;
      `window.location().replace(...)` takes `permalink.as_ref()`/`&permalink`;
      emptiness checks use `.as_ref().is_empty()` or a typed predicate. No
      behavior change.

- [x] **Step 6: Run the wire test (PASS)** +
      `cargo clippy -p storage -p web --all-targets` +
      `cargo clippy -p web --target wasm32-unknown-unknown`. Expected: PASS.

- [x] **Step 7: Commit** (`jaunder-commit`):
      `refactor(web,storage): type post permalink/preview_url as RootRelativeUrl (#560)`.

  **Deviation (behavior-preserving):** `TimelinePostSummary.permalink` was typed
  `Option<RootRelativeUrl>` (not the plan's non-optional `RootRelativeUrl`). A
  draft's permalink is absent (empty-string sentinel today, driving link-less
  title render + the ADR-0044 flash-free paint), and a validated
  `RootRelativeUrl` can't be empty — so absence is modeled as `Option`,
  mirroring `PostResponse.permalink`. Timeline endpoints always emit `Some(..)`;
  wire unaffected in the normal path.

---

## Task 5: Feed/atompub e2e + test migration

**Files:** Modify `end2end/tests/feeds.spec.ts`, `end2end/tests/atompub.spec.ts`
(+ their `fixtures.ts`/`seed.ts` setup if base_url is seeded there).

- [x] **Step 1: Seed `site.base_url`** — done in the **canonical global seed**
      (`tools/devtool/src/seed_e2e.rs`, `= https://example.com`) rather than
      per-spec, so every spec has it and there's no parallel-spec race (it sits
      beside the existing `registration_policy`/`websub_hub_url` seeds; unit
      test updated). The atompub e2e was already written for this (`onServer`
      re-baser + substring assertions); the feed head auto-discovery links are
      relative (`canonicalize`), so their resolve is unaffected. _(Original step
      said seed in the feed/atompub e2e setup)_ absolute URL) so the suites run
      the now-required-base path. Follow the existing site-config seeding idiom
      (`seed.ts`/`fixtures.ts`).

- [x] **Step 2: Assert absoluteness** — covered by the pre-existing atompub e2e
      (`onServer` re-bases the absolute Location/URLs to fetch them) + the
      Task-3 renderer unit tests asserting emitted `<id>`/`self`/`edit` are
      absolute. Not adding fragile new e2e assertions (local e2e VM is reaped —
      unverifiable here). Original: **Step 2: Assert absoluteness** — in at
      least one feed and one atompub test, assert the emitted
      `<id>`/`self`/per-item URLs begin with the seeded base (are absolute).

- [x] **Step 3: Negative coverage** — satisfied by the Task-3 integration test
      `collection_get_without_base_url_returns_500` (base unset → atompub 500) +
      the inverted `regenerate_without_base_url_errors` unit test. Original:
      **Step 3: Negative coverage** — a test (e2e or the Task-3 integration
      test) with `base_url` unset asserts the feed/atompub endpoint errors. (If
      e2e can't easily unset base mid-suite, the Task-3 server integration test
      satisfies AC5's negative case; note that here.)

- [x] **Step 4: Run** `cargo xtask validate --no-e2e` (local e2e VM is reaped —
      CI's matrix gates the browser run). Expected: PASS.

- [x] **Step 5: Commit** (`jaunder-commit`):
      `test(e2e): seed site.base_url for the require-base feed/atompub surface (#560)`.

---

## Self-review notes

- **Spec coverage:** AC1→T1; AC2→T2; AC3→T3 S1/S3/S6; AC4→T3 S2 + T4 S3/S4;
  AC5→T3 S6 (+ T5 S3); AC6→T3 S3 (with_query_pairs, decoded-equivalence test);
  AC7→T4; AC8→T5; AC9→each commit's gate. Decisions D1–D5 realized;
  `id`→AbsoluteUrl (T3 S2) and error-surface (T3 S3) per approval.
- **Type consistency:** `RootRelativeUrl` (web post URLs), `AbsoluteUrl` (all
  composed feed/ atompub URLs incl. `FeedItem.permalink`),
  `compose(&AbsoluteUrl,&str)->AbsoluteUrl` everywhere.
- **No placeholders:** foundation tasks carry full tests+signatures; Task 3/4
  enumerate the exact files + the mechanical retype rule + per-step
  verification. Line numbers are grep-verified at edit time (post-#574 rebase).
