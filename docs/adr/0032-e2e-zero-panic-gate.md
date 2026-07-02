# ADR-0032: The e2e suite fails on any server Rust panic, and its log is visible by default

- Status: accepted
- Deciders: mdorman, Claude Opus
- Date: 2026-06-27

## Context and Problem Statement

A Rust panic inside the running server during an e2e run was invisible. Server
panics are isolated to spawned tasks: the HTTP response still returns 200 and
Playwright assertions still pass, so the e2e check **passed despite the panic**.
Worse, once a panicking-but-passing e2e derivation was cached (cachix),
subsequent CI runs were cache-hits that emit no `nix build -L` machine output,
so even `.xtask/diagnostics/<check>/build.log` was empty — the panic left no
trace anywhere a reviewer would look.

This is exactly how #89 hid: a per-render `expect_context` panic on every
authenticated SSR page, isolated and cached green for many releases, only ever
surfacing as an occasional flaky hydration timeout. The diagnostic gap — not the
bug — is what let it persist.

## Decision

**A server panic must fail the e2e suite, and the server log must be
discoverable without a rebuild.**

1. **In-sandbox zero-panic assertion.** Each e2e nixos `testScript` dumps the
   `jaunder.service` journal, copies it into the check's `$out`, and asserts the
   journal contains no `panicked at` line. A panic therefore fails the
   _derivation_ — so it cannot be cached "green" and cannot silently reappear.
   The check is **default-deny**: an explicit, commented `ALLOWED_PANICS` list
   is the only seam for a proven-benign exception, so the bar never moves
   silently.

2. **Visible-by-default journal.** The journal lives in `$out` (surviving
   cache-hits), and the `xtask` e2e step copies it from the realized output into
   the single canonical location `.xtask/diagnostics/e2e/`, which CI already
   uploads with `if: always()`. The server log is thus an artifact on every run
   — fresh or cached, pass or fail — in one unambiguous place.

## Consequences

- Good: an isolated server panic can no longer pass e2e or hide behind a
  cache-hit; the class of defect that produced #89 is now caught at the gate.
- Good: the e2e server log is discoverable by default, removing the
  rebuild-to-diagnose step that made #89 hard to find.
- Neutral: enabling the gate may surface _other_ previously-cached-green panics;
  each is addressed when it appears — fixed by preference,
  `ALLOWED_PANICS`-listed only with a written justification.
- Cost: a small shared Python snippet in each backend `testScript`, and a copy
  step in the xtask e2e gate.
