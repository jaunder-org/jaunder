# Issue #1149 — Proffered filename extractor-private implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for isolated slices
> if useful. This outline exists because the approved spec changes a cross-crate
> type boundary, Axum extractor contracts, a static gate, and accepted ADR text.

## Scope

In:

- Move the decoded filename intermediate out of the public `common::media`
  surface and confine it to server extractor implementation code.
- Preserve ADR-0084 filename semantics: decoded safe-leaf check, one
  percent-encode, encoded-length rejection, and infallible conversion to
  `Filename`.
- Keep media serve and AtomPub media member route behavior for decoded filename
  path segments.
- Delete or rewrite `proffered-filename-position` so the gate matches the new
  privacy boundary.
- Amend ADR-0063, ADR-0084, ADR-0140, and the architecture view where they
  describe the public proffered type or gate.

Out:

- Media URL layout, on-disk layout, database representation, and `Filename`'s
  canonical encoded invariant.
- New storage/sqlx/serde/display trailer for a decoded filename intermediate.
- Other milestone #13 type-safety issues.

## Task outline

- [x] Task 1: Establish the common validating conversion without public
      proffered type
  - Contract: `common::media` owns the validation/encoding logic needed to
    construct a canonical `Filename` from an Axum-decoded path segment, but
    exposes no unchecked `Filename` constructor and no public
    `ProfferedFilename` type. Existing `Filename` parsing/sanitizing behavior
    remains unchanged.
  - Verification: focused `common::media` tests/doctests prove the
    decoded-segment conversion accepts safe decoded names, rejects
    unsafe/over-budget decoded names, and does not add `Display`, `Serialize`,
    or sqlx behavior for a decoded intermediate.

- [x] Task 2: Move decoded filename extraction behind server extractor seams
  - Contract: media serve extraction and AtomPub media member extraction produce
    only `Filename` or a validated address by the time handler logic runs. Any
    server-private wrapper/intermediate is private to extractor implementation
    code and cannot appear in ordinary structs, helper parameters, returns,
    server functions, DTOs, storage, or web code.
  - Verification: media serve route tests still prove malformed address
    components are pre-handler 400 and well-formed-but-absent media is 404;
    AtomPub media tests still prove decoded names such as `my%20photo.jpg`
    find/delete the stored record and unsafe decoded names such as `a%5Cb.png`
    are pre-handler 400.

- [x] Task 3: Replace the obsolete public-type gate
  - Contract: `xtask` no longer treats `common::media::ProfferedFilename` as a
    public type that may appear in selected extractor positions. Either remove
    `proffered-filename-position` entirely because Rust privacy now enforces the
    boundary, or rewrite it as a migration detector for any remaining
    public/common exposure or decoded-intermediate use outside extractor
    implementation code.
  - Verification: `cargo xtask check --no-test` reports the new static-check
    surface green; if the gate remains, its unit tests include at least one
    failing fixture for decoded-intermediate leakage outside extractor code and
    no longer bless public cross-crate extractor positions.

- [x] Task 4: Amend durable docs for the new seam
  - Contract: ADR-0063, ADR-0084, and ADR-0140 state that the decoded filename
    intermediate is extractor-private and that `Filename` is the only type past
    extraction. `docs/ARCHITECTURE.md` names the updated gate/static-check state
    and keeps ADR citations current.
  - Verification: `cargo xtask check --no-test` covers ADR
    formatting/readme/view parity and static docs checks; targeted review
    confirms no current durable doc claims `ProfferedFilename` must be public
    for route signatures.

## Risk checks

- Do not introduce an unchecked path from arbitrary `String` to `Filename`;
  every decoded-segment construction path must run the current safe-leaf,
  encode, and budget checks.
- Do not accidentally double-encode canonical filenames; the decoded-segment
  conversion is used only at Axum-decoded route boundaries.
- Do not turn malformed decoded filename segments into lookup misses; current
  pre-handler 400 behavior remains observable on media serve and AtomPub media
  member routes.
- Do not leave stale allowlist language: gate diagnostics, ADRs, architecture
  text, and code docs must describe the same extractor-private boundary.
- Before commit, run `devtool run -- cargo xtask precommit` through
  `jaunder-commit` after focused checks for the changed route/gate/docs slices.
