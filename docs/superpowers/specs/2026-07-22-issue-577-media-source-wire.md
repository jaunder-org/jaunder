# Spec — #577: `MediaSource` on the wire + dialect boundary

- Issue: [#577](https://github.com/jaunder-org/jaunder/issues/577)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md),
  [ADR-0065](../../adr/0065-typed-wire-args.md) (typed wire args),
  [ADR-0074](../../adr/0074-str-enum-trailer.md) (`StrEnum` trailer)
- Related: [#562](https://github.com/jaunder-org/jaunder/issues/562) (`StrEnum`),
  [#576](https://github.com/jaunder-org/jaunder/issues/576) (`RegistrationPolicy` — the
  direct precedent for the enum-to-`common` move)
- Date: 2026-07-22

## Problem

`storage::MediaSource` is a proper enum on the `MediaStorage` surface, but degrades to
strings at four sites (issue #577): `MediaItem.source: String`, the `list_my_media` /
`delete_media` wire args (re-parsed in-body via `str::parse::<MediaSource>`), and the
internal `MediaDialect::delete_media_row(source: &str)` dialect hop. As with #576, the
enum lives in `storage`, which the wasm client cannot import — hence the stringly wire
hop.

## Decision

Move `MediaSource` to `common::media` (its natural home — `ContentHash`, `ContentType`,
`Filename` already live there) carrying the `StrEnum` trailer, and thread the typed enum
through the DTO, both wire args, and the dialect method.

### The enum — `common::media::MediaSource`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
#[str_enum(serde, error = "media source must be \"upload\" or \"cached\"")]
pub enum MediaSource {
    Upload,
    Cached,
}
```

- Tokens `"upload"` / `"cached"` (both single-word, so the snake_case default (#576)
  yields them unchanged — no rename).
- `#[str_enum(serde)]` — the type is a `#[server]` DTO field and wire arg; the serde
  bridge (serialize `as_str`; deserialize an owned `String` via `FromStr`) is what makes
  it work through the `serde_qs` `ActionForm` transport (ADR-0074).
- `#[str_enum(error = "…")]` preserves the exact user-facing message the hand-written
  `InvalidMediaSource` carried (a host error message + a storage test assert it).
- `Copy` added (fieldless 2-variant enum; simplifies call sites).
- The derive generates the named error `InvalidMediaSource` — same name as today.

### The error conversion moves to host's `validation_from!` macro

Today `storage/src/media.rs` hand-writes `From<InvalidMediaSource> for
host::error::InternalError`. Once `InvalidMediaSource` moves to `common`, that impl would
violate the orphan rule in `storage` (both types foreign). `host/src/error.rs` already
has a `validation_from!` macro that generates exactly this conversion
(`validation_source(error.to_string(), error)` — kind `Validation`, class `Client`,
public message = `Display`) for each `common` value-object error (`InvalidSlug`,
`InvalidUsername`, `InvalidPostFormat`, …). So:

- **Delete** storage's hand-written `From<InvalidMediaSource> for InternalError`.
- **Add** `common::media::InvalidMediaSource` to the `validation_from!` list in
  `host/src/error.rs`. Behavior is byte-for-byte preserved (same constructor, same
  `Display` message via `#[str_enum(error)]`).

### `storage` uses it directly — no re-export

`MediaSource` now lives in `common`, so consumers import it from its true home rather
than laundering it through `storage::MediaSource` (a Middle Man for a type storage no
longer owns). `storage/src/media.rs`: delete the hand-rolled `enum` + `as_str` +
`Display` + `FromStr` + `InvalidMediaSource` + the `From` impl, and **do not re-export**;
add `use common::media::MediaSource;` for storage's own uses (`MediaRecord.source`, the
dialect method). `helpers::build_media_record` still `.parse()`s the DB column (via the
`common` `FromStr`).

**Compiler-guided sweep of the former `storage::MediaSource` importers** — each drops
`MediaSource` from its `use storage::{…}` list and adds `use common::media::MediaSource;`
(these crates already depend on `common`): `server/src/{atompub/media,media_manager,media}.rs`,
`storage/src/helpers.rs` (`use crate::… → use common::media::MediaSource`), and the
server tests (`server/tests/{storage/mod,web/web_media,misc/backup_fixture}.rs`). The
`MediaSource::Upload` / `::Cached` value expressions (and, in `server/src/media.rs`, the
`SoftPath<MediaSource>` extractor and its `parse::<MediaSource>`) are unchanged once the
import resolves — only the import line moves.

(This deliberately diverges from #576, which re-exported `RegistrationPolicy` from
`storage`; direct import is the cleaner pattern and the one to prefer going forward.)

### The dialect method typed

- `MediaDialect::delete_media_row(…, source: &MediaSource)` (was `&str`); the sqlite +
  postgres impls (`storage/src/{sqlite,postgres}/media.rs`) bind `source.as_str()` at the
  sqlx `.bind(...)` (the existing AsRef/Deref bind convention).
- The `MediaStorage::delete_media` caller passes `source` (`&MediaSource`) instead of
  `source.as_str()`.

### `web` — DTO + wire args typed, in-body parses deleted

- `web/src/media/api.rs`:
  - `MediaItem.source: MediaSource` (add `MediaSource` to the existing
    `use common::media::{…}` import); build it as `source: r.source` (drop
    `.to_string()`). `media_url(r.source.as_str(), …)` unchanged (the URL builder stays
    `&str` — deliberate flattening, out of scope).
  - Drop `MediaSource` from the server-gated `use storage::{…}` list (avoid an E0252
    collision with the ungated `common` import).
  - `list_my_media(source: Option<MediaSource>, …)` — delete
    `source.as_deref().map(str::parse::<MediaSource>).transpose()?`; pass
    `source.as_ref()` to `list_media`.
  - `delete_media(source: MediaSource, …)` — delete `source.parse::<MediaSource>()?`;
    use `source` directly; `media_url(source.as_str(), …)`.
- `web/src/media/component.rs`: `MediaItem.source` is now `MediaSource`. Line 344
  `let source = item.source.clone();` → `item.source.to_string()` (the hidden
  `<input name="source">` and the display `<td>` both want an owned string; the
  `DeleteMedia` action arg `source: MediaSource` is reconstructed by deserializing that
  form string through the serde bridge — no programmatic typed pass needed). The
  `list_my_media(None, …)` caller is unchanged (`None` fits `Option<MediaSource>`).

## Out of scope

- `common::media::media_url(source: &str, …)` stays `&str` (deliberate `enum → str`
  flattening for URL composition; prior precedent in #459).
- `server/src/media.rs`'s `SoftPath<MediaSource>` extractor and its `parse::<MediaSource>`
  are server-side path parsing, not a `web/` wire hop — the *logic* is unchanged (its
  import line still moves to `common::media::MediaSource` as part of the sweep above,
  since `storage::MediaSource` no longer resolves).

## Tests

- `common::media`: `MediaSource` `FromStr` accept both tokens, reject unknown (with the
  custom message), `as_str`/`Display` round-trip, serde round-trip. Relocated from the
  `storage/src/media.rs` type-behavior tests.
- `host::error`: the existing `from_common_validation_sources_preserve_display_as_public`
  test gains `common::media::InvalidMediaSource` in its `check!` list (message-preserved,
  client-classified).
- `storage`: dual-backend `delete_media` / `list_media` / `find_by_hash` tests unchanged
  (now exercising the re-exported type and the `&MediaSource` dialect arg).
- `web`: existing media tests updated for the typed DTO/args (behavior unchanged — an
  invalid `source` now fails at deserialization rather than the in-body parse, an
  earlier, ADR-0065-aligned rejection).

## Acceptance

- `MediaSource` defined once in `common::media` with the `StrEnum` trailer
  (`#[str_enum(serde)]` + preserved error message); `storage` neither defines nor
  re-exports it — every consumer imports `common::media::MediaSource` directly, and
  `storage::MediaSource` no longer resolves.
- `MediaItem.source`, both `#[server]` wire args, and `MediaDialect::delete_media_row`
  are typed `MediaSource`; **no `parse::<MediaSource>` remains in `web/` source**.
- `InvalidMediaSource → InternalError` is preserved via host's `validation_from!` macro
  (same message + classification).
- The `"upload"` / `"cached"` DB/wire tokens are unchanged.
- `cargo xtask validate --no-e2e` clean.
