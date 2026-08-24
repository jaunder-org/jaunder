# Server-function flow-coverage evidence

## Outcome

Server-function flow coverage has one durable generated artifact: the
deterministic, byte-compared snapshot of browser-driven server functions and
orphan reasons. The committed, uncompared per-test-title evidence file is
removed, eliminating unrelated generated diffs and stale attribution claims.

## Load-bearing decisions

- `docs/coverage/server-fns.json` remains the sole committed server-function
  flow-coverage artifact and the sole generated artifact read by the static
  verdict. The source-derived server-function inventory remains its other input.
- Per-test trace attribution remains part of coverage extraction because it
  distinguishes requests driven inside a test from unattributed orphan traffic.
  It is not persisted as a separate artifact.
- `docs/coverage/server-fns-evidence.json` and all behavior whose only purpose
  is reading, writing, rendering, or cross-checking that artifact are removed in
  a clean cutover.
- Regeneration writes only the deterministic snapshot. Verification reads and
  compares only that snapshot.
- No replacement title list, spec-file list, gitignored evidence file, or
  attribution-reporting CLI is introduced. A developer needing request-to-test
  attribution inspects a fresh trace capture.
- ADR-0081 is amended in place: trace-derived coverage remains the decision,
  while the later two-file compromise from #745 is retired by #757.

## Acceptance

- The committed per-test-title evidence file no longer exists, and repository
  guidance presents `docs/coverage/server-fns.json` as the only generated
  server-function flow-coverage artifact.
- Adding, renaming, or deleting an e2e test title cannot alter the committed
  server-function coverage artifact when the covered-function and orphan-reason
  sets are unchanged.
- Coverage regeneration succeeds from an authoritative capture and writes only
  the deterministic snapshot.
- Static verification fails closed for a missing, malformed, or drifted snapshot
  and no longer requires or reads title evidence.
- Coverage extraction still classifies test-attributed requests as covered and
  preserves the existing orphan-reason behavior.
- The focused server-function coverage tests and the repository pre-commit gate
  pass.

## Boundaries

- This work does not change the server-function inventory, trace propagation,
  request-to-test attribution algorithm, authoritative e2e combo, orphan
  classification, or coverage policy.
- This work does not add a human-readable flow-to-server-function mapping; flow
  documentation and the CSR/e2e traceability matrix remain separate surfaces.
- Historical archived specs, plans, and accepted ADRs other than ADR-0081 remain
  historical records and are not rewritten solely to remove old evidence-file
  references.
