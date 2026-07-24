# Spec — `ETag` `StrNewtype` for the quoted-format invariant (#634)

**Issue:** jaunder-org/jaunder#634 · **Milestone:** #13 (Domain-value type
safety) · **Follows:** #584 (typed `FeedCacheRow.content_type`, which deferred
this) · **Family:** #495 (`ContentType`), ADR-0063.

## Problem

An HTTP `ETag` carries a `"…"`-quoted-format invariant enforced _nowhere_. The
value is minted as a bare `String` at five independent producers, each
hand-rolling its own quoting (`format!("\"…\"")` or manual `push('"')`). Only
one copy is ever stored (`FeedCacheRow.etag`); #584 typed the sibling
`content_type` field but deliberately left `etag` a `String`, because typing the
stored field alone would enforce nothing while every producer still emits
`String`. Meaningful enforcement needs an `ETag` newtype threaded through the
producers, the stored field, and the comparison sites.

## Decisions (from the design interview)

1. **`ETag` owns the whole construction pipeline.** Every ETag in this codebase
   is SHA-256-based, so `ETag` owns
   `digest → 64-lowercase-hex → "sha256-" prefix → surrounding quotes`
   end-to-end. Producers hand it **content bytes**, a **precomputed digest**, or
   a **`ContentHash`** — never a pre-formatted string; no producer spells hex,
   the `sha256-` prefix, or the quotes. Three doors (see the type section):
   `sha256_of(&[u8])` (computes), `from_sha256([u8; 32])` (precomputed digest),
   `from_content_hash(&ContentHash)` (precomputed hex). This extends the
   `ContentHash::from_digest` house pattern (the newtype owns the format detail)
   one step: the newtype owns the _hashing_ too, where the producer has the raw
   bytes.
2. **The body format is unified to `"sha256-<64hex>"`.** All five producers
   converge on the full-digest, `sha256-`-prefixed form. Today three prefix with
   `sha256-` and two don't, and `feed_etag` truncates to a 16-byte (32-hex)
   digest — divergence with no reason but history. There are **no production
   cache clients**, so the value changes are free: `site.rs` and `media.rs` gain
   the `sha256-` prefix; `feed_etag` goes 32→64 hex (drops its truncation — no
   test asserts hex length). Owning the pipeline is what makes this unification
   fall out for free rather than being a separate behavioral change.
3. **Strong validators only.** `FromStr` rejects the weak form `W/"…"`. Nothing
   produces or consumes weak ETags today; the invariant stays minimal and
   honest, revisitable if a weak producer ever appears. `FromStr` validates the
   _general_ HTTP quoted strong-tag format (not the specific `sha256-` body
   scheme) — it models an HTTP ETag, and the minting _policy_ (always `sha256-`)
   lives in the typed doors, not the parse door.
4. **Comparison stays a string compare.** The `If-None-Match` / `If-Match` /
   `304` / `412` logic keeps string-equality semantics, comparing the request
   header `&str` against the produced/stored `ETag` through its `str` view
   (`Deref` / `PartialEq<str>`). Header _values_ change (the unified body), but
   the comparison _mechanism_ is unchanged.
5. **No new ADR, no new xtask gate.** This stays within ADR-0063 and the
   `ContentType` precedent (#584/#495), which shipped no bespoke gate.
   Enforcement is: the type itself (a bare `String` can no longer reach these
   fields/args), the derived validating sqlx `Decode` on read-back, the existing
   `sqlx-newtype-bind` gate on storage binds, and a pinned test that each
   producer's output re-parses through `FromStr`. ETag is a _format_ invariant,
   not a trust/secrecy boundary like `RenderedHtml`, so it needs neither the
   `from_trusted` XSS gate nor a secret surface. (The pipeline-ownership
   rationale is recorded here in the spec; an ADR/addendum can be added on
   request.)

## Design

### The type — `common/src/etag.rs`

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct ETag(String);
```

- The `#[derive(StrNewtype)]` default trailer supplies `Display`, `AsRef<str>`,
  `Borrow<str>`, `Deref<Target = str>`, owned-`String` conversions,
  `PartialEq<str>` / `PartialEq<&str>`, the validating serde bridge, and the
  on-by-default (feature-gated) sqlx `Type`/`Encode`/`Decode` bridge — mirroring
  `ContentType`.
- Hand-written `impl FromStr` is the one validating chokepoint. Invariant (RFC
  7232 strong `opaque-tag`): the string is `DQUOTE etagc+ DQUOTE` — first and
  last byte `"`, length ≥ 2, and every interior byte an `etagc` (`0x21`,
  `0x23–0x7E`, `0x80–0xFF`; excludes `"`, control bytes, and space). A leading
  `W/` (weak) is rejected. Error type `InvalidETag` (derives `thiserror::Error`)
  with a message naming the rule.
- **Three `pub` producer doors**, all yielding `"sha256-<64hex>"`, funnelling
  into one private `fn sha256_tag(hex) -> ETag` (prefix + quote — the single
  place a `"` and the `sha256-` prefix are written):
  - `pub fn sha256_of(bytes: impl AsRef<[u8]>) -> ETag` — computes
    `Sha256::digest(bytes)` internally, hex-encodes, prefixes, quotes. For
    producers holding the raw content.
  - `pub fn from_sha256(digest: [u8; 32]) -> ETag` — hex-encodes a _precomputed_
    digest. For producers that already have a digest (a compile-time embed hash,
    or an incremental `Sha256` they finalize themselves).
  - `pub fn from_content_hash(hash: &ContentHash) -> ETag` — prefixes + quotes
    an existing validated 64-hex `common::media::ContentHash`. For a producer
    that holds only the hex (media, whose hash is a URL segment and whose file
    is streamed, so re-hashing is not an option).

  All three are trusted producer doors (no `FromStr` round-trip) — structurally
  valid by construction (a 32-byte digest is always 64 lowercase hex; a
  `ContentHash` is already validated hex), the `ContentHash::from_digest` house
  pattern extended to own hashing. Correctness is pinned by tests that each
  door's output re-parses via `FromStr`. `sha2` (`Sha256`) and `ContentHash` are
  **already** in `common` (`feed_etag` uses both), so this adds no dependency.
  None of the doors is named `from_trusted`, so the #398
  `rendered-html-from-trusted` gate is untouched and needs no
  `EXEMPT_QUALIFIERS` entry. No public string/opaque-tag door is exposed — every
  ETag value is `sha256-` by construction; a non-SHA-256 door can be added if
  one is ever needed.

- Lives in `common` (already depended on by `server` and `storage`); wasm-clean
  (no server-only deps; the sqlx bridge is feature-gated off on wasm).
  Registered via `pub mod etag;` in `common/src/lib.rs`.

### Producers — hand `ETag` bytes/digest/hash, spell no hex or quotes (5 sites)

| Site                                                            | Now                                                              | After                                                                                                                      | Value change            |
| --------------------------------------------------------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| `server/src/site.rs` `etag_for(&[u8])` + caller                 | helper hex-encodes + `push('"')` a 32-byte embed hash            | **delete `etag_for`**; caller does `ETag::from_sha256(file.metadata.sha256_hash())` (a `[u8; 32]`; drop the `.as_ref()`)   | gains `sha256-` prefix  |
| `server/src/projector/mod.rs` (inline in `cacheable`)           | `format!("\"sha256-{:x}\"", Sha256::digest(body.as_bytes()))`    | `ETag::sha256_of(body.as_bytes())`                                                                                         | none (identical)        |
| `server/src/media.rs` (inline in `serve_response`)              | `format!("\"{hash}\"")` on a `ContentHash`                       | `ETag::from_content_hash(&hash)` (no re-hash; file is streamed)                                                            | gains `sha256-` prefix  |
| `common/src/feed/metadata.rs` `feed_etag(...) -> String`        | incremental `Sha256`, `.take(16)`, `format!("\"sha256-{hex}\"")` | `-> ETag`; keep the incremental `hasher.update(...)`, then `ETag::from_sha256(hasher.finalize().into())`; drop `.take(16)` | 32→64 hex (full digest) |
| `server/src/atompub/posts.rs` `etag_for(&PostRecord) -> String` | `format!("\"sha256-{:x}\"", Sha256::digest(&bytes))`             | `-> ETag`; `ETag::sha256_of(&bytes)`                                                                                       | none (identical)        |

Each producer keeps its own _domain_ logic (which bytes/fields identify the
resource version) and hands `ETag` the content, the digest, or the hash. The
`digest → hex → "sha256-" → quotes` tail lives entirely in `ETag`. `feed_etag`'s
`sha256_hash`/`Digest` imports may become unused once it delegates — clean those
up.

### Storage — `storage/src/feed_cache.rs`

- `FeedCacheRow.etag: String → ETag` (drop the "#634 tracks this" carve-out
  comment).
- `CacheTuple`'s 3rd element `String → ETag`; `row_from_tuple` unchanged (moves
  the field; the derived `Decode` already validated at the query boundary).
- `upsert`: `.bind(row.etag.as_str())` → `.bind(&row.etag)` (typed bind via the
  derived `Encode`; the `sqlx-newtype-bind` gate forbids an `.as_ref()`/`&*`
  strip).
- No schema/migration change — the column stays TEXT.

### Consumers — string-compare _semantics_ unchanged, but the edits are compile-forced (5 sites)

The 304/412 logic keeps byte-for-byte string-equality semantics, but typing the
value `ETag` forces mechanical rewrites — the derive gives
`AsRef`/`Deref`/`Borrow`/ `PartialEq<str>`, **not** an inherent `as_str()` nor
the reverse `str: PartialEq<ETag>`. Concretely:

- `server/src/site.rs::not_modified(if_none_match, etag)` — compares
  `tag.trim() == etag`; the produced-value arg becomes `&ETag` (callers pass
  `&etag`), compared via `PartialEq<&str>`. Its unit tests need
  `Some(etag.as_ref())` (Deref does **not** reach through `Option`, so
  `Some(&etag)` yields `Option<&ETag>`) and drop `etag.as_str()`.
- `server/src/projector/mod.rs` — `inm.to_str().ok() == Some(etag.as_ref())`
  (was `Some(etag.as_str())` on a `String`).
- `server/src/media.rs` — flip the comparison to
  `etag_value == if_none_match.to_str()…` (LHS `ETag`, RHS `&str`) since only
  `ETag: PartialEq<str>` exists, not the reverse.
- `server/src/feed/handlers.rs` — `Some(row.etag.as_str())` →
  `Some(row.etag.as_ref())` (no inherent `as_str` on `ETag`).
- `server/src/atompub/posts.rs::if_match_satisfied(headers, etag: &ETag)` — `*`
  wildcard path untouched; the equality arm compares via the `str` view.

**Response-header construction converts `ETag` to a header value at the
boundary** — the one boundary conversion. Its shape follows each site's existing
code: `site.rs` / `feed/handlers.rs` use `HeaderValue::from_str(etag.as_ref())`;
`atompub/posts.rs` builds `(header::ETAG, …)` tuples in an array alongside
sibling `String` entries, so the ETag entry becomes
`etag_for(&post).to_string()` (a heterogeneous `[(_, String), (_, ETag)]` array
will not compile).

### Test seams

- Add `common::test_support::parse_etag(&str) -> ETag` (routes through the
  validating `FromStr`, per the newtype test-helper convention).
- Fix fixtures/call sites currently holding an **unquoted** etag (would now fail
  `FromStr`/`Decode` _and_ be a type error against the `ETag` field):
  - `server/tests/feed/feed_worker.rs` (`etag: "etag"`).
  - `server/tests/storage/mod.rs` (`etag: "etag"`).
  - `server/tests/feed/feed_handlers.rs` — **three** sites
    (`etag: "known-etag"`, `etag: "test-etag-123"`, `etag: "test-etag"`). In
    `handler_if_none_match_returns_304` the stored etag literal is **also sent
    as the `If-None-Match` request header**; both the stored value and the
    header must be quoted to the same value, or the 304 comparison silently
    mismatches.
  - The in-crate `storage/src/feed_cache.rs` unit fixture already uses a valid
    `"\"sha256-deadbeef\""`; `server/tests/projector/mod.rs` already sends a
    valid quoted `"\"sha256-stale\""`.

  These fixture values are arbitrary — they need only be _valid quoted strong
  tags_ (the `sha256-` prefix is a producer policy, not a `FromStr`
  requirement), so `"\"known-etag\""` etc. parse fine. Build them via
  `common::test_support::parse_etag()` rather than raw `String` literals.

## Acceptance criteria

1. **Type & validation.** `common::etag::ETag` exists with a private field,
   `StrNewtype` derive, and a validating `FromStr`. Tests: accepts canonical
   `"…"`; rejects each of empty, unquoted, single `"`, missing-close-quote,
   interior `"`, control/space interior, and weak `W/"…"`, each surfacing the
   `InvalidETag` message. serde serializes as the raw quoted string and rejects
   an invalid value on deserialize. The FromStr/serde and all three doors are
   covered (common is coverage-measured).
2. **Doors own the pipeline.** The three doors — `sha256_of(bytes)`,
   `from_sha256([u8;32])`, `from_content_hash(&ContentHash)` — each produce
   `"sha256-<64hex>"`, and each has a unit test asserting its output re-parses
   through `FromStr` and equals the expected `"sha256-<hex>"` for a known input
   (e.g. `from_sha256([0u8;32])` and `sha256_of(b"")`/`from_content_hash` agree
   for the empty-input digest). The `sha256-` prefix and the `"` appear in
   exactly one private helper (grep-checkable: no `format!("\"` or `"sha256-"`
   literal outside `common/src/etag.rs`).
3. **Producers hand raw inputs.** All five producers mint via a door; no
   producer contains `format!("\"…\"")`, a manual quote `push`, a `sha256-`
   literal, or an `{:x}` hex format on an ETag path (grep-checkable).
   `site::etag_for` is deleted (caller uses `from_sha256` directly). `feed_etag`
   returns `ETag`. Produced values: projector + atompub identical; site + media
   gain the `sha256-` prefix; feed is full 64-hex — asserted by the producers'
   own tests (updated for the new values).
4. **Storage typed.** `FeedCacheRow.etag` and the `CacheTuple` slot are `ETag`;
   `upsert` binds the typed value with no `.as_str()`/`.as_ref()`/`&*` strip; a
   tampered unquoted `etag` column read-back is rejected as a `ColumnDecode`
   error (mirrors the existing `content_type` decode-error test).
5. **Comparison mechanism unchanged.** All
   `If-None-Match`/`If-Match`/`304`/`412` unit, integration, and e2e tests still
   pass — feed 304, media 304, projector 304 + stale-200, atompub If-Match
   412/200, site 304 — after the **compile-forced mechanical edits** to test
   call sites (the `as_ref()`/`Some(etag.as_ref())`/comparison-flip changes
   mirroring the consumer edits, and valid-quoted fixture/header literals). A
   conditional request that echoes a produced ETag still yields `304`/`412` (the
   round-trip is self-consistent even though the emitted value now differs for
   site/media/feed); no 304/412 _outcome_ changes.
6. **Gates green.** `cargo xtask validate --no-e2e` clean (fmt, clippy,
   coverage, `sqlx-newtype-bind`, and the unchanged `rendered-html-from-trusted`
   gate).

## Out of scope

- Weak-ETag representation (`W/"…"`).
- A non-SHA-256 or opaque-string producer door — every ETag is SHA-256-based;
  add one only if a non-hash ETag ever appears.
- Changing _which_ bytes/fields each producer hashes (its resource-version
  identity) — that domain logic stays put; only the format tail moves into
  `ETag`.
- The `#[server]`/wire boundary — no ETag crosses it; `FeedCacheRow` is
  storage-internal.
- #417 and the other open milestone-#13 issues.
