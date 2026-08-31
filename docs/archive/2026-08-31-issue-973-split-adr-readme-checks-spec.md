# Split ADR README checks by concern

## Outcome

The xtask ADR documentation gates are organized into cohesive private leaf
modules while retaining their existing crate paths, CLI behavior, gate output,
and repository-document contracts. ADR file validation, README table projection,
and architecture-view parity each live with the tests that prove them.

## Load-bearing decisions

- Replace `xtask/src/adr_readme.rs` with a wiring-only
  `xtask/src/adr_readme/mod.rs` containing module documentation, private
  declarations, and explicit re-exports only, per ADR-0128.
- Preserve every existing `crate::adr_readme::*` and `pub(crate)` path used by
  `xtask/src/adr.rs`, `xtask/src/steps/adr_check.rs`, and CLI dispatch; do not
  add a compatibility layer or expose new module paths.
- Split implementation into private `files.rs`, `readme.rs`, and `view.rs`
  leaves, matching the issue's three independently named concerns.
- Keep the single non-recursive numbered-ADR population, `AdrEntry`, heading and
  status parsing, status rewrite contract, and file-format validation together
  in `files.rs`. Preserve the exact `StatusLine` span/remainder/canonical
  semantics and the `PROPOSED`/`ACCEPTED` coupling used by merge-time promotion.
- Keep `TableRow`, marker parsing, table parsing/rendering/splicing,
  synchronization, and README parity together in `readme.rs`. Preserve
  ADR-0036's mechanical number/link/status cells, hand-owned titles, ordering,
  duplicate/error behavior, and recovery command.
- Keep accepted-ADR `docs/ARCHITECTURE.md` citation parity in `view.rs`.
  Preserve ADR-0127's accepted-only population, Markdown-link and bare-token
  citation recognition, dangling-citation behavior, and missing-view error.
- Move each existing unit test beside the implementation or contract it proves,
  retaining names, fixtures, assertions, and fail-closed coverage. Shared
  inventory helpers remain owned by `files.rs`; leaves may use private sibling
  contracts without facade exports.
- Update current `docs/ARCHITECTURE.md` source-symbol citations whose exact
  paths move. Leave historical ADR and archive references unchanged.
- Treat completed issue #742 as a compatibility constraint, not a prerequisite:
  preserve promotion/renumber status rewriting and README synchronization; do
  not revisit ADR promotion design or #1169.

## Acceptance

- `mod.rs` only documents and assembles the existing module surface; each of
  `files.rs`, `readme.rs`, and `view.rs` has one named responsibility.
- Existing xtask callers compile without import-path migration, aliases, or new
  exports, and `adr-format`, `adr-readme-parity`, `adr-view-parity`, and
  `adr-sync-readme` retain their names, ordering, details, and recovery text.
- Numbered ADR discovery/format validation, README table rendering/sync/parity,
  architecture-view parity, and promotion status rewriting remain observably
  unchanged, including malformed and missing-file cases.
- All existing `adr_readme` tests move with their owning contracts and retain
  their behavior; existing `adr.rs` promotion/renumber tests continue to pass.
- Current architecture documentation cites the new source locations accurately.
- The test-enabled repository gate (`cargo xtask check`) passes on the complete
  split.

## Boundaries

- No ADR format, status vocabulary, promotion workflow, README table schema,
  architecture-view policy, CLI grammar, gate registration, diagnostic text,
  domain vocabulary, or filesystem population change.
- No new public helper, interface rename, deprecation, compatibility shim,
  allowlist, recursive discovery, or unrelated documentation cleanup.
- No new ADR: this refactor implements ADR-0036, ADR-0127, and ADR-0128 without
  changing an architectural decision.
