# Plan — #577: `MediaSource` on the wire + dialect boundary

Spec: [2026-07-22-issue-577-media-source-wire.md](../specs/2026-07-22-issue-577-media-source-wire.md)
· Issue [#577](https://github.com/jaunder-org/jaunder/issues/577)

## Review header

**Goal.** Move `MediaSource` from `storage` into `common::media` (`StrEnum` trailer),
thread the typed enum through the `MediaItem` DTO, both `#[server]` wire args, and the
`delete_media_row` dialect method, and re-home its error conversion to host's
`validation_from!` macro — deleting the four stringly sites.

**Scope.**
- **In:** `common::media::MediaSource` (StrEnum + serde + preserved error message);
  host `validation_from!` registration; delete storage's duplicate enum/error/`From`;
  type `MediaRecord`/dialect/`MediaStorage`/web DTO+args+component; the compiler-guided
  `storage::MediaSource → common::media::MediaSource` import sweep; delete the in-body
  `web` parses.
- **Out:** `media_url(&str)` (deliberate flattening); the `server/src/media.rs` extractor
  *logic* (only its import moves).

**Tasks.**
1. Introduce `common::media::MediaSource` + register its error in host's
   `validation_from!` (storage's duplicate untouched — both coexist).
2. Atomic switchover: delete storage's duplicate, retype every surface to the `common`
   type, sweep all importers, delete the `web` in-body parses; update all tests.

**Key risks / decisions.**
- The type swap is **atomic by necessity**: `storage::MediaSource` and
  `common::media::MediaSource` are distinct types, so the `MediaStorage` trait, its impls,
  all callers, and the `web` DTO/args must move together in Task 2 — it won't compile
  half-migrated. Task 2 is one large but cohesive commit.
- **No re-export** (per the spec): `storage::MediaSource` stops resolving; every importer
  moves to `common::media::MediaSource` (compiler-guided).
- Error conversion re-homes to host's `validation_from!` (orphan rule); message preserved
  via `#[str_enum(error = …)]`; the host test's `check!` list covers the new `From`.
- Tokens `"upload"`/`"cached"` are single-word → snake_case default yields them unchanged.

**For agentic workers:** execute with **jaunder-iterate**; Task 2 is a good candidate to
delegate via **jaunder-dispatch** (broad, mechanical, compiler-guided).

## Global constraints

- No `Co-Authored-By` trailer.
- Each task: `cargo xtask check` clean before commit (pre-commit hook enforces it). Task 2
  closes with `cargo xtask validate --no-e2e`.
- Follow `CONTRIBUTING.md` (backend parity — the dialect method change touches both
  sqlite + postgres impls; coverage policy; import discipline).
- Preserve the `"upload"` / `"cached"` DB/wire tokens and the exact
  `"media source must be \"upload\" or \"cached\""` error message.

---

## Task 1 — introduce `common::media::MediaSource` + host error registration

**Files**
- `common/src/media.rs`: add the enum (alongside `ContentHash`/`ContentType`/`Filename`):

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
  #[str_enum(serde, error = "media source must be \"upload\" or \"cached\"")]
  pub enum MediaSource {
      /// File uploaded directly by a local user.
      Upload,
      /// Remote file cached locally by the system.
      Cached,
  }
  ```
  (Import `macros::StrEnum` if `common::media` doesn't already. The derive generates
  `as_str`/`Display`/`FromStr`/`TryFrom` + the named `InvalidMediaSource` + the serde
  bridge.)
- `host/src/error.rs`: add `common::media::InvalidMediaSource,` to the `validation_from!(…)`
  list; add `check!(common::media::InvalidMediaSource);` to the
  `from_common_validation_sources_preserve_display_as_public_and_are_client` test.

**Test** — in-file `#[cfg(test)] mod tests` in `common/src/media.rs` (relocated from the
`storage/src/media.rs` type-behavior tests + a serde assertion):

```rust
#[test]
fn tokens_parse_and_round_trip() {
    assert_eq!("upload".parse::<MediaSource>().unwrap(), MediaSource::Upload);
    assert_eq!("cached".parse::<MediaSource>().unwrap(), MediaSource::Cached);
    assert_eq!(MediaSource::Upload.as_str(), "upload");
    assert_eq!(MediaSource::Cached.to_string(), "cached");
}

#[test]
fn unknown_token_is_rejected_with_message() {
    let err = "bogus".parse::<MediaSource>().unwrap_err();
    assert_eq!(err.to_string(), "media source must be \"upload\" or \"cached\"");
}

#[test]
fn serde_round_trips_the_token() {
    assert_eq!(serde_json::to_string(&MediaSource::Cached).unwrap(), "\"cached\"");
    assert_eq!(serde_json::from_str::<MediaSource>("\"upload\"").unwrap(), MediaSource::Upload);
}
```

**Run**
- `cargo nextest run -p common media::` and `cargo nextest run -p host error::` — PASS
  (both `MediaSource` still coexists with storage's untouched duplicate).
- `cargo xtask check` — clean.

**Commit:** `feat(common): MediaSource StrEnum in common::media (#577)`

---

## Task 2 — switch every surface to the `common` type; delete the duplicate

One atomic commit (the type-identity swap). Organized by area:

**`storage/src/media.rs`**
- Delete the `enum MediaSource`, `impl MediaSource`/`Display`/`FromStr`, `InvalidMediaSource`,
  and `From<InvalidMediaSource> for InternalError` (and the now-unused `thiserror::Error`
  import if nothing else needs it). **No re-export.**
- `use common::media::MediaSource;` (join the existing `use common::media::{…}` line).
- `MediaDialect::delete_media_row(…, source: &MediaSource)` (was `&str`).
- `MediaStorage::delete_media` caller: `DB::delete_media_row(&self.pool, user_id, sha256,
  filename, source)` (drop `.as_str()`).
- Delete the relocated type-behavior tests (`as_str`/`Display`/`FromStr`/the
  `InvalidMediaSource → InternalError` test at ~line 645); keep the dual-backend
  `delete_media`/`list_media`/`find_by_hash` tests.

**`storage/src/{sqlite,postgres}/media.rs`** (both — backend parity)
- `delete_media_row(…, source: &MediaSource)`; bind `.bind(source.as_str())` (was
  `.bind(source)`).
- Add `use common::media::MediaSource;`.

**`storage/src/helpers.rs`**
- Swap `MediaSource` in the `use crate::{…}` list for `use common::media::MediaSource;`.

**`server/src/{atompub/media,media_manager,media}.rs` + `server/tests/{storage/mod,web/web_media,misc/backup_fixture}.rs`**
- Each: drop `MediaSource` from `use storage::{…}`, add `use common::media::MediaSource;`.
  `MediaSource::Upload`/`::Cached` and (in `server/src/media.rs`) `SoftPath<MediaSource>` /
  `parse::<MediaSource>` are unchanged.

**`web/src/media/api.rs`**
- `use common::media::{…, MediaSource}` (add to the existing import); drop `MediaSource`
  from the server-gated `use storage::{…}`.
- `MediaItem.source: MediaSource` (was `String`); build `source: r.source` (drop
  `.to_string()`); `media_url(r.source.as_str(), …)` unchanged.
- `list_my_media(source: Option<MediaSource>, …)`: delete
  `source.as_deref().map(str::parse::<MediaSource>).transpose()?`; pass `source.as_ref()`.
- `delete_media(source: MediaSource, …)`: delete `source.parse::<MediaSource>()?`; use
  `source` directly; `media_url(source.as_str(), …)`.

**`web/src/media/component.rs`**
- Line 344: `let source = item.source.to_string();` (was `.clone()`) — `MediaSource` impls
  neither `IntoView` nor `IntoAttributeValue`, so the `<td>` display and hidden
  `<input name="source">` need the owned string. The `DeleteMedia` action arg
  `source: MediaSource` is reconstructed by deserializing that form string.

**Tests**
- `web`: update media tests for the typed DTO/args (an invalid `source` now fails at
  deserialization). Behavior unchanged.
- `server`/`storage`: value expressions unchanged post-import-swap; confirm the
  dual-backend delete/list tests still pass (they now pass `&MediaSource` to the dialect).

**Run / final gate**
- `cargo check -p storage`, `-p host`, `-p server`, `-p web` (+ `--features server` for web)
  all compile.
- `cargo nextest run -p storage media::` — PASS (dual-backend).
- `cargo xtask validate --no-e2e` — green. Confirm `rg -n "parse::<MediaSource>" web/src`
  returns nothing, and `storage::MediaSource` appears nowhere.

**Commit:** `refactor(common,storage,web,server): type MediaSource end-to-end, drop stringly hops (#577)`

## Self-review

- Task 1 compiles with both `MediaSource` types coexisting (storage's untouched); Task 2
  is the atomic swap that deletes the duplicate. No partial-migration state is ever
  committed.
- Every spec acceptance criterion maps to a task: enum-once + host registration → Task 1;
  typed DTO/args/dialect + no `web` parse + no `storage::MediaSource` + token preservation
  → Task 2.
