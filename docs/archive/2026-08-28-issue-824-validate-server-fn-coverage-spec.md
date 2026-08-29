# Issue #824: Validate server-function coverage parity

## Outcome

`cargo xtask validate` runs the same authoritative SQLite/Chromium
server-function coverage verification as CI's per-combination E2E command. A
stale committed coverage snapshot fails locally instead of surviving until a
pull request.

## Load-bearing decisions

- ADR-0081's sole authoritative evidence remains the SQLite/Chromium E2E
  capture; `validate` does not union or compare all four captures.
- `validate` still realizes all four E2E combinations through the aggregate.
  After that E2E portion succeeds, it resolves the already-realized
  SQLite/Chromium derivation output directly.
- Resolution uses
  `nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.outPath`; it never
  invokes another `nix build`, so verification cannot trigger a second VM run.
- Coverage reads the uncollided `<outPath>/capture-sqlite.tar.gz`, not the
  aggregate symlink join.
- E2E success is computed from the steps appended by the E2E portion, not from
  global `CommandResult::ok`. An earlier unrelated validate failure must not
  hide an otherwise-valid coverage result.
- If any E2E-specific step fails, coverage verification is explicitly skipped as
  secondary to an untrustworthy/incomplete run. If E2E succeeds, outPath,
  capture, extraction, parsing, and snapshot drift failures are fail-closed.
- Standalone `cargo xtask e2e sqlite chromium` keeps its existing verification
  path and result semantics.
- `flaky::collect` remains informational on explicit per-combination commands;
  this issue does not add it to `validate`.

## Acceptance

- A successful `validate` E2E aggregate appends a `server-fn-coverage-verify`
  step sourced from the authoritative individual derivation output.
- A deliberately stale `docs/coverage/server-fns.json` makes that step and
  `validate` fail without running another E2E VM.
- An earlier non-E2E validate failure does not suppress coverage verification
  when all E2E-specific steps pass.
- Any E2E-specific failure preserves the primary failure and appends an explicit
  skipped coverage step.
- Missing, empty, malformed, or drifted authoritative capture/snapshot data
  fails closed after a successful E2E aggregate.
- Existing standalone per-combination coverage verification remains green and
  unchanged.
- `CONTRIBUTING.md` accurately states that local `validate` covers this CI
  correctness gate; no other per-combination-only correctness gate is omitted.

## Boundaries

- No E2E matrix, authoritative combination, snapshot format, flow-coverage rule,
  VM configuration, or capture naming change.
- No second VM realization and no reliance on collided aggregate artifacts.
- No change to informational flaky-report collection.
