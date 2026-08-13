# #269 — enforce the zero-panic gate in the host e2e loop

Issue: [#269](https://github.com/jaunder-org/jaunder/issues/269). Milestone:
Test infrastructure & E2E. Blocked-by #249 is complete.

## Problem

`cargo xtask e2e-local` owns a real `jaunder serve` process and enables the same
`JAUNDER_CAPTURE_DIR` contract as the NixOS-VM e2e checks, but it judges success
only from Playwright. A Rust panic isolated in a server task can therefore pass
the host loop even though the VM's ADR-0032 zero-panic gate rejects it.

The VM currently implements panic verification as inline Python in `flake.nix`.
It scans the union of the app-written `capture/diag.log` and the
`jaunder.service` journal for the raw substring `panicked at`, excludes an
explicit default-empty allowlist, deduplicates reports by panic location,
prefers the scoped diagnostic record, and fails on any remaining report. Copying
that logic into `xtask` would create two implementations of a load-bearing gate.

This issue closes the panic-verification gap. It does not make ordinary WARN+
diagnostic records fatal and does not add host OpenTelemetry capture; the latter
remains #802.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | Move panic verification into one Rust implementation owned by the existing `test-support` binary. Both the VM and `e2e-local` invoke it; neither keeps a second parser or decision path.                                                                                                                                                                                                                                                                                                                                                            |
| **D2** | Preserve the ADR-0032/ADR-0049 contract exactly: scan bytes for raw `panicked at` substrings from both sources rather than parsing JSON or requiring UTF-8, apply one default-empty source-controlled allowlist, deduplicate by the location token after `panicked at ` with trailing `:` removed, and prefer the scoped diagnostic record when both sources report the same location. A marker-bearing line with no location token still fails and uses the whole line as its fallback deduplication key. Ordinary WARN+ records are not failures. |
| **D3** | The verifier accepts the capture directory and a journal-equivalent server-log path. It resolves the diagnostic stream through `host::capture::Stream::Diag`, so the filename remains defined only by the ADR-0057 capture contract. A missing scoped diagnostic log is treated as empty, preserving the VM's pre-hook/journal-only fallback. Failure to read the required server log is an infrastructure error, not a clean result.                                                                                                               |
| **D4** | The VM continues to materialize `journalctl -u jaunder.service` output, then invokes the shared verifier with the capture directory and that journal file. The inline Python verifier and its local allowlist are removed. The existing journal and capture artifact behavior remains unchanged.                                                                                                                                                                                                                                                    |
| **D5** | On the host, `jaunder serve` stderr remains visible live and unchanged in the terminal while also being streamed to a file inside the run's temporary directory. That file is the VM-journal equivalent and catches panics emitted before the app panic hook is installed. The implementation does not buffer the whole server log in memory.                                                                                                                                                                                                       |
| **D6** | After Playwright finishes, including when Playwright fails, `e2e-local` stops and reaps its server, drains the stderr mirror, and runs the shared verifier. Playwright failure and panic-verifier failure are recorded as independent failed steps; neither masks the other.                                                                                                                                                                                                                                                                        |
| **D7** | Host capture files retain the existing per-run lifetime and disappear with the temporary directory. A panic failure prints the offending records through the verifier, while the full server stderr was already visible live. No shared persistent host artifact path is introduced, preserving concurrent-run isolation.                                                                                                                                                                                                                           |
| **D8** | The allowlist has no command-line or environment bypass. A future exception requires a documented source change in the one shared verifier and therefore applies identically to host and VM.                                                                                                                                                                                                                                                                                                                                                        |
| **D9** | No ADR is needed. This applies the existing ADR-0032, ADR-0049, and ADR-0057 contracts to the host loop without changing their architecture or policy. `docs/ARCHITECTURE.md` is updated to state that both e2e surfaces invoke the shared verifier.                                                                                                                                                                                                                                                                                                |

## Observable flow

### NixOS-VM e2e

1. Run Playwright and capture its exit status.
2. Preserve the existing journal and capture artifacts.
3. Invoke `test-support`'s panic verifier with the capture directory and the
   materialized `jaunder.service` journal.
4. Fail for a verifier error and independently fail for a non-zero Playwright
   status.

### Host `e2e-local`

1. Spawn `jaunder serve` with stderr mirrored live to the terminal and to a
   per-run file.
2. Seed and run Playwright as today.
3. Preserve Playwright's success or failure instead of returning immediately on
   failure.
4. Stop/reap the server and finish draining the mirrored stderr.
5. Invoke the same `test-support` verifier with the per-run capture directory
   and server-stderr file.
6. Return failure if Playwright failed, the verifier found a panic, or the
   verifier itself could not inspect its required input.

## Acceptance criteria

- **AC1 — one verifier.** The panic detection, allowlist, location extraction,
  deduplication, and scoped-record preference exist in one Rust implementation
  reachable through `test-support`. `flake.nix` contains no inline panic parser
  or second allowlist, and `xtask` contains no duplicate parser.

- **AC2 — exact clean-case semantics.** Given clean inputs, an absent diagnostic
  stream, arbitrary non-JSON or invalid-UTF-8 bytes without `panicked at`, or
  ordinary WARN+ JSONL records without that substring, the shared verifier
  succeeds when the required server log is readable and contains no panic.

- **AC3 — raw union and preference semantics.** A raw `panicked at` byte
  sequence in either source fails verification even inside torn, non-JSON, or
  otherwise invalid-UTF-8 input, including a line ending at the marker with no
  location token. When both sources contain the same panic location, the failure
  reports it once and uses the scoped diagnostic line. Distinct locations are
  all reported.

- **AC4 — default-deny allowlist.** The shared source-controlled allowlist is
  empty. No CLI flag or environment variable can bypass or extend it.

- **AC5 — VM uses the shared verifier.** Every
  `{sqlite,postgres}×{chromium,firefox}` VM e2e check invokes the verifier after
  diagnostic capture and before the Playwright-status assertion, while
  continuing to publish its existing journal and capture tarball artifacts.

- **AC6 — host stderr is both live and complete.** `e2e-local` shows server
  stderr during the run, captures the same bytes without whole-log memory
  buffering, and drains the capture before verification. The server is still
  killed and reaped on every exit path.

- **AC7 — Playwright cannot mask a panic.** If Playwright and panic verification
  both fail, the `CommandResult` records distinct failed steps for both.
  Verification runs after a Playwright failure rather than being skipped by an
  early return.

- **AC8 — host clean run.** A real targeted `cargo xtask e2e-local <spec>` run
  with no server panic passes its new panic-verification step and tears down its
  temporary server and files.

- **AC9 — regression proof.** Tests exercise clean inputs; an absent optional
  diagnostic stream; arbitrary non-JSON and invalid-UTF-8 input with and without
  the raw panic substring; a marker with no following location; server-log-only
  panic; diagnostic-only panic; same-location deduplication with scoped-record
  preference; distinct panics; a nonexistent or unreadable required server log
  producing an infrastructure error; and host result aggregation when Playwright
  and verification both fail.

- **AC10 — documentation and gates.** `docs/ARCHITECTURE.md` describes the
  shared VM/host verifier. A real targeted `cargo xtask e2e sqlite chromium` run
  proves the migrated VM invocation, the shared helper structurally covers all
  four backend/browser combinations, and the full `cargo xtask validate` gate
  passes.

## Non-goals

- Failing on all WARN+ diagnostic records.
- Adding or analyzing host OTel traces (#802).
- Persisting host capture artifacts outside the per-run temporary directory.
- Changing Playwright specs, browser coverage, retry policy, VM artifact names,
  or the `JAUNDER_CAPTURE_DIR` filename contract.
- Adding a runtime-configurable panic exception.

## Risks

- **Stderr mirroring can regress teardown.** The server owner must retain RAII
  cleanup for every early return, and normal completion must join the mirror
  only after the child is stopped so the pipe reaches EOF.
- **Shell invocation in the VM.** The verifier must receive the
  capture-directory and journal paths as arguments through the existing NixOS
  test script without embedding log contents in shell text or restating the
  diagnostic filename.
- **Failure masking.** `e2e-local` must delay its Playwright return until
  verification finishes and retain both step results; replacing one error with
  the other would violate AC7.
