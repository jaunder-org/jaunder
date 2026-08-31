# Split storage test support implementation outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because an atomic public module-surface cutover must preserve cross-crate
> macro, template, visibility, and cfg contracts across ten Rust files.

## Scope

In:

- Atomically replace `storage/src/test_support.rs` with the approved wiring-only
  facade and nine cohesive implementation leaves.
- Move every existing unit test beside the implementation or contract it proves.
- Preserve every existing `storage::test_support` import, exported macro, rstest
  template, backend path, and observable behavior.

Out:

- The `Backend::setup` redesign tracked by #841.
- The `PostWriteLock` behavior fix tracked by #874.
- Production behavior, schema, protocol, domain vocabulary, or unrelated
  cleanup.

## Task outline

- [x] Task 1: Land the atomic module-surface cutover and prove the unchanged
      test-support contract.
  - Contract: `storage/src/test_support/mod.rs` explicitly re-exports the
    unchanged facade from `backend`, `postgres`, `users`, `posts`, `media`,
    `post_service`, `mail`, `feeds`, and `invites`; `with_closeable_pool!`
    continues to resolve `$crate::test_support::CloseablePool`; `backends`,
    `backends_matrix`, `sqlite_only`, and `postgres_only` remain usable by
    bare-name `#[apply(...)]` imports across crates.
  - Ownership: one integration owner assembles `mod.rs`, removes the old source
    file, reconciles visibility/imports, and verifies the complete tree;
    delegated leaf extraction may proceed in parallel only under that facade
    contract.
  - Verification: run the focused host-native storage test-support contract
    first, then the test-enabled repository gate `cargo xtask check`;
    both-backend coverage and unchanged server imports are required evidence.

## Risk checks

- `storage/src/lib.rs` retains
  `#[cfg(any(test, feature = "test-support"))] pub mod test_support;` unchanged.
- `mod.rs` contains only module documentation, attributes, declarations, and
  explicit re-exports under ADR-0128.
- Re-export coverage is exhaustive: no caller changes import paths, no aliases
  or compatibility shims appear, and `MEDIA_TEST_SHA256` remains available from
  the facade.
- Cross-leaf private items use the narrowest visibility that compiles; moving
  code does not silently widen the public facade.
- `CloseablePool`, lock guards, `TestEnv`/`TestBase`, setup, SQLite URL
  handling, and templates remain one backend/fault-injection concern; PostgreSQL
  lifecycle remains isolated in `postgres.rs`.
- Media backup/raw-file helpers remain in `media.rs`; focused mail, feed-path,
  and invite-code helpers do not form a generic grab-bag module.
- Existing tests move rather than being deleted, weakened, duplicated, or
  re-homed away from their current backend matrix.
- No lint suppression is introduced without explicit approval; commits contain
  no `Co-Authored-By` trailer.
