# #1811 Host temporary-file routing Implementation Outline

> Execute with dev-cycle-iterate. Outline needed because the approved
> shared-shell change adds a durable architecture boundary at development-shell
> startup. This is not an authentication change.

**Stable outline request ID:** `issue-1811-outline-v1`  
**Status:** Draft; not yet approved.  
**Approved scope:**
`docs/superpowers/specs/2026-10-23-issue-1811-host-temp-routing.md`
(`issue-1811-spec-v1`).

## Execution summary

First resolve the costly compatibility assumptions in a small tracer: an empty
`unix_socket_directories` value must survive `pg_ctl -o` construction, actual
PostgreSQL startup/bootstrap must still work over loopback TCP at a deeply
nested temporary path, and the generated Cargo metadata manifest must isolate
itself from the checkout workspace. Then implement the shared shell-entry TMPDIR
contract and verify inheritance, overrides, failure behavior, and binding to the
checkout at entry. Finish with focused integrated proof, two-axis conformance
review, and applicable normal hook/CI gates. Scope and guarantees remain those
of the approved spec.

## Task outline

- [x] Task 1: Keep Cargo metadata generation isolated and ephemeral PostgreSQL
      usable with a deeply nested TMPDIR.
  - Contract: generated manifest has a producer-local empty `[workspace]`;
    disable only the unused ephemeral PostgreSQL Unix socket while retaining
    PGDATA, loopback TCP, bootstrap, PgEnv, and lifecycle behavior.
  - Verification: focused Cargo metadata regression with temp storage nested
    under the checkout; verify `pg_ctl -o` quoting and actual startup/bootstrap
    plus child TCP connectivity under a deeply nested/long TMPDIR. These are
    diagnostic proofs, not broad workload or final certification.
- [ ] Task 2: Make supported development-shell entry establish the approved
      checkout-local TMPDIR contract.
  - Contract: default, CI, and mutants shells use an absolute
    `<checkout>/.xtask/tmp`, deliberately replace inherited entry TMPDIR,
    preserve later command-local overrides, remain bound to entry checkout after
    `cd`, and visibly fail without `/tmp` fallback when checkout resolution or
    directory usability fails. No shell behavior outside the approved shared
    devShell contract.
  - Verification: focused shell-entry checks for each supported shell, inherited
    child and override behavior, unusable-root/directory failure, and checkout-A
    entry followed by `cd` versus entry in checkout B; update only minimal
    relevant docs.
- [ ] Task 3: Integrate and establish approved behavior against the combined
      tree.
  - Contract: preserve unique per-run leaves and existing cleanup owners;
    routing is not cleanup and adds no collector, framework, global temp policy,
    or Nix-daemon/sandbox guarantee.
  - Verification: map every approved spec acceptance criterion to focused
    integrated proof, conduct the required Standards/Spec review, then run
    applicable normal commit hook and CI gates for the resulting implementation.
    No broad profiling or expanded matrix beyond approved acceptance.

## Risk checks

- Resolve empty PostgreSQL socket-setting argument/quoting behavior and prove
  real PostgreSQL startup/TCP before investing in shell inheritance coverage.
- Confirm Cargo's generated metadata package remains independent of ancestor
  workspace membership without changing root workspace configuration.
- Confirm checkout-root and directory failure is visible and unsuccessful, with
  no silent fallback; confirm shell-entry binding and later override behavior.
- Preserve PGDATA, bootstrap/PgEnv TCP behavior, unique per-run leaves, and
  existing cleanup ownership; do not imply routing provides cleanup.
- Keep all changes, claims, and guarantees within the approved spec; any
  material design change returns to spec approval.
