# #1795 — Remediate ZIP dependency advisories

## Outcome

Remove the ZIP advisory blocker from Jaunder's supported dependency-policy gate
without exemptions or changes to the Theme Package contract. Deliver one shared,
main-derived prerequisite that dependent branches can deliberately adopt.

## Load-bearing decisions

- The retained RED is merge-group run 38993230572: cargo-deny rejected locked
  `zip 8.6.0` for RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369.
  Browser success does not override that verdict.
- The supplied canonical RustSec records list `>=9.0.0` as patched for all three
  advisories. Select a stable release satisfying all three ranges; 9.0.0 is the
  earliest candidate, not a uniquely mandated or proven fix.
- Candidate 9.0.0 declares Rust 1.88 / edition 2024; the mapped pinned toolchain
  is Rust 1.97.1. Removed `ZipFileData::extra_data_start` use was not found.
  These facts are not compilation or compatibility proof.
- The starting scope is `host/Cargo.toml` and independently reconciled
  `Cargo.lock`, `tools/Cargo.lock` and `xtask/Cargo.lock`. All three mapped
  graphs resolve ZIP through host; tools and xtask have no direct declaration.
- Preserve `default-features = false` and explicit `features = ["deflate"]`. Do
  not enable upstream's broad defaults or refresh unrelated dependencies. Review
  necessary changes within the selected ZIP dependency closure.
- Each lock belongs to its own workspace. Regenerate it there, never by
  transplanting another workspace's or another branch's lock.
- Include compatibility edits or Nix fixed-output/vendor hash adjustments only
  when demonstrably necessary for the selected resolution and existing behavior.
  A newly required contract change is not authorized by this allowance.
- Preserve ADR-0184's bounded, non-executable Theme Package and ADR-0197's
  common validator, canonical source and deterministic package exporter.
- Preserve closed UTF-8 forward-slash member names, path rejection, duplicate
  and encrypted-member rejection, nonregular/symlink rejection, header
  agreement, compressed/expanded/file-count/per-file limits and supported
  compression. Preserve manifest/assets validation and canonical source/revision
  identities.
- Mapped production ZIP use is bounded in-memory reading/writing, not filesystem
  extraction. Absence of observed extraction narrows the call-site surface; it
  is neither a no-exploitability claim nor an advisory-policy exception.
- Export must remain deterministic and validated. Within-run determinism is not
  evidence of byte identity between dependency versions: inspect emitted archive
  and system-artifact changes rather than silently blessing changed identities.
- This is one security-sensitive dependency-maintenance cycle, not a diagnostic
  feature change on PR #1794. One writer owns the shared repair.
- Existing maintained behavior tests are the approved compatibility seams. A
  pure dependency correction need not add a version-number assertion test. If
  source adaptation needs a missing behavioral regression, confirm the real
  maintained seam and demonstrate failure before fixing it; do not invent a
  privileged-filesystem extraction seam absent matching production behavior.

## Acceptance

All implementation proof below is prospective: intake and spec preparation did
not compile a candidate, resolve dependencies, run tests or establish
sufficiency.

1. The final manifest and all three independently owned locked graphs resolve a
   stable ZIP release satisfying the three patched ranges. Review the actual
   selected release's features, APIs, toolchain and target implications; verify
   the supported owning graph/feature/target checks and locked consuming seams.
   Record exact commands, source identity and results without claiming one
   product-graph command audited every auxiliary graph.
2. The actual host advisory-capable cargo-deny policy passes on the final
   candidate, with current advisory data and no ignored advisories or suppressed
   diagnostics. Use the supported host devtool/xtask route. ADR-0145's sandbox
   bans/licenses/sources subset does not prove the original advisory blocker
   fixed.
3. Through `devtool run -- cargo xtask test-local --`, pass these existing
   focused product filters:
   - `-p host theme_package::tests`
   - `-p host system_theme::tests`
   - `-p host theme_repository::tests`
   - `-p jaunder web::themes::` The backend-parametric integration cases retain
     SQLite/PostgreSQL parity.
4. Theme Package proof includes archive-byte/path-limit rejection,
   Unix-symlink-member rejection, duplicate/encrypted/header-disagreement
   rejection and deterministic validated round-trip with canonical source. These
   prove the application's archive boundary, not upstream extraction safety.
5. Pass the auxiliary consumer diagnostic through
   `devtool run -- cargo test --locked --manifest-path tools/Cargo.toml -p devtool csr_bundle::tests`.
   Inspect package/system-artifact determinism and identity compatibility using
   the maintained consumers; no blanket fixture or baseline regeneration.
6. Review the final diff for minimal dependency-closure churn, independent lock
   ownership, retained feature intent and unchanged archive/security contracts.
   Obtain independent security-sensitive dependency review, applicable ordinary
   hook/prepush proof and current PR/merge-group CI under CONTRIBUTING's ladder.
7. Retain command results and review evidence in ignored run storage or the PR,
   not a committed execution report. The working spec is committed only after
   approval, maintained through conformance and deleted at final lifecycle
   cleanup.

## Boundaries

- Spec approval remains required before dependency/product edits, lock
  regeneration, producers or implementation commits. A major version alone
  requires no new outline. No novel architecture decision is introduced here.
- Stop and surface any required archive grammar, compression/features, public
  schema/interface, canonical identity or filesystem-trust-boundary expansion
  for the specific renewed scope/design/outline decision before implementing it.
- No advisory waiver, lint/coverage suppression, broad dependency refresh,
  foreign-lock copy, new public interface, production extraction, queue policy,
  privacy change, deployment, native exiftool-rs change or physical cleanup.
- The historical synthetic-lock/yanked-version seeding correction is separate.
  Reuse only its smallest reviewed prerequisite if demonstrably required; do not
  import a peer feature or duplicate its completed diagnosis.
- After shared delivery, dependent heads need deliberate adaptation, current
  proof/review and renewed exact-head merge approval when changed. Do not modify
  or land those branches in this cycle; PR #1794's b43 approval does not extend.
- #1791 remains open; this repair claims neither its original pixel cause nor
  the historical trace-parent cause fixed. Preserve the assigned semantic
  branch, checkout, unrelated state and retained failure evidence. Per-PR merge
  approval and Manager-owned lifecycle retirement remain separate.
