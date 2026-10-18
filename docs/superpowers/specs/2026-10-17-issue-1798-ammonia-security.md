# #1798 — remediate the ammonia CSS URL advisory

## Outcome

Remove the shared dependency-policy denial for ammonia 4.2.3 / RUSTSEC-2026-0373
while preserving Jaunder's existing rendering, sanitization, privacy, and
persisted-byte contracts. Deliver one shared-main repair, not parallel fixes in
dependent feature branches.

## Load-bearing decisions

- Raise the root `Cargo.toml` workspace ammonia floor to `4.2.4` and target
  ammonia 4.2.4 independently in `Cargo.lock`, `tools/Cargo.lock`, and
  `xtask/Cargo.lock`. Keep common/host workspace inheritance; explain only
  necessary resolution closure changes, without unrelated refreshes or lock
  transplantation. Change Nix fixed hashes only if the actual resolution
  requires.
- 4.2.4 is the nearest patched release on the current 4.2 line. Its published
  Rust 1.85 / edition 2021 / parser dependency lines match current metadata;
  project compatibility remains untested until implementation proof executes.
- Preserve dependency features, parser consumers, host-only `common/sanitize`,
  and ammonia's exclusion from the exact CSR target closure (ADR-0159).
- Preserve body style stripping, closed markup/classes/footnote IDs, URL schemes
  and relative-URL policy, Media reference classification and ownership, and
  typed provider-embed boundaries (ADRs 0079, 0179, 0202).
- Preserve the attribute-free inline Rendered Title grammar, plain-text title
  projection, persisted-title byte validation, authored-title rules, historical
  snapshots and no-op behavior. Do not sanitize body rows on read or backfill
  persisted content (ADRs 0123, 0204, 0214).
- Existing builders do not enable CSS-property filtering. This narrows relevant
  compatibility proof; it is neither a cargo-deny exemption nor an application
  exploitability conclusion. RustSec describes tracking / limited exfiltration,
  not XSS; no numeric severity or CWE is asserted here.

## Acceptance

- Each independent locked graph resolves ammonia 4.2.4 with a reviewed minimal
  closure delta, unchanged relevant features/target policy, and meaningful
  locked-consumer compatibility proof. The delivered ZIP 9.0.4 fix is unchanged.
- The actual host advisory-capable cargo-deny check passes on the final tree
  using current advisory data, without new exceptions or stale-database
  bypasses. Retain the supplied failed CI denial as historical RED; sandbox
  bans/licenses/ sources-only success is not advisory proof (ADR-0145). Label
  any separate auxiliary-graph advisory audits with their actual graph and
  result.
- Run existing focused common/host rendering tests covering active markup/style
  rejection, safe markup and Media URLs, closed attributes/classes/footnotes,
  authored HTML/Markdown/Org rendering, and Rendered Title projections.
- Run mapped stored-body sanitization and persisted Rendered Title rejection /
  lifecycle / no-op / prior-snapshot tests with applicable SQLite/PostgreSQL
  parity through the supported `cargo xtask test-local -- ...` route. Record
  actual executed identities/counts; zero selected tests is not success.
- Verify the existing isolated common/host target-closure policy. Assess
  behavior compatibility, not just version numbers; any needed regression
  strengthens the actual style-rejection boundary rather than enabling unused
  CSS filtering.
- Close independent security-sensitive, Standards, and spec-conformance review
  findings; pass normal enforced pre-commit/prepush hooks and current PR and
  merge-group CI. Renew per-PR exact-head merge approval separately.

## Boundaries

No downgrade, sanitizer replacement, new CSS-filter seam or exploit framework,
allowlist/URL/privacy expansion, parser consolidation, schema/API change,
advisory ignore, lint/coverage waiver, runtime repair, or dependent-branch
edits. Do not reopen ZIP #1795, bundle diagnostic PR #1794, or claim to fix
#1791's unresolved cause. No blanket full local matrix or foreign-runtime
requirement. A genuine contract expansion requires a new scoped decision; a
patch alone requires no outline. Commit the reviewed, human-approved spec before
implementation; retire working paperwork after conformance. Keep proof logs in
ignored storage, not committed reports. Preserve managed branch/checkout;
deployment, cleanup, physical removal, and manual Done are outside this scope.
