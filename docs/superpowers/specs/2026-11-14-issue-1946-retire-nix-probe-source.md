# Issue #1946 — retire the Nix source-invalidation matrix probe

## Outcome

`just source-diagnostics nix-probe-source` and its 30-arm expectation matrix are
removed. Docs stop promising its guarantees. `source-diagnostics` keeps
`cache-safety-probe`. #1946 is resolved by retirement, not reconciliation: the
human chose retirement on 2026-11-14 after reviewing the full-run evidence
below, which also confirms this manual-capability loss as #1859 requires.

## Why retire instead of reconcile

The probe evaluates the `.drv` identity of ten targets (`static-docs`,
`static-code`, `static-elisp`, `static-e2e-census`, `.#site`, `wasm-tests`,
`.#host-proof`, `.#devtool`, its tooling dependencies, `csrDebugBundle`) before
and after each of 30 one-file perturbations, and compares the changed set with a
hand-written table.

A full run on `main` (e0382b474, ~23 min, serial `nix eval`s) found 4 mismatch
kinds across 20 arms. Every one was a stale expectation after an intentional
refactor, not a filter defect:

| Mismatch                                        | Cause                                                             |
| ----------------------------------------------- | ----------------------------------------------------------------- |
| 19 arms expect `static-elisp` to change         | a7890c18b (#1898) made it run Just + Emacs over `elisp/` only     |
| `tooling` arm changes `static-code`             | `static-code` requires `clippy-aggregate`, which admits `tools/`  |
| `lane-catalog` no longer changes `.#host-proof` | host-proof reads the catalog only in tests; xtask compiled it in  |
| `qualification-schema` changes `static-code`    | clippy compiles `qualification`, which `include_str!`s the schema |

What the probe can uniquely detect is over-inclusion (needless rebuilds) and
changed target wiring — cost diagnostics that intentional refactors trip.
Under-admission of a needed compile/embed input already fails loudly (the file
is absent from the Nix sandbox), and several filters' inline `assert`s check
representative membership on every evaluation. A format/lint file a hermetic
filter drops is silently unchecked there, but the host `just check` formatting
and lint run still sees the whole tree; the probe only ever perturbed fixed
marker paths, so it did not cover that case generally either. Whether tests
actually run is assured by the real test/coverage/reconciliation gates, not by
admission. The probe runs only on request (ADR-0227) and went unrun through
#1898, so its table drifted unseen. Maintaining the table costs about what the
probe returns.

Alternatives considered and rejected:

- **Reconcile the table** (+ report all mismatches, + `qualification-host`
  boundary): passes today, but keeps a ~1,200-line table that breaks on the next
  intentional boundary change while finding no defects.
- **Table-free admission check** (does any filter admit each tracked file):
  cheap, but "admitted" does not mean "tested"; catching the meaningful case (a
  test file excluded from the test source) needs a rule table again. Weak signal
  for new machinery; not pursued.

## Approach

- Remove `tools/host-proof/src/nix_probe.rs` and `nix_eval.rs` (only the probe
  uses it), their `lib.rs` modules, and the `NixProbeSource` subcommand. The
  `probes_parse_as_closed_subcommands` unit test asserts `nix-probe-source` is
  now rejected (parsing precedes checkout admission, so it runs anywhere).
- `tests/checkout_admission.rs:24` switches its subcommand to
  `cache-safety-probe`, which reaches the same `checkout::prepare()` refusal.
- `Justfile` recipe comment becomes `# Eval-only Nix guard: cache-safety-probe.`
- `CONTRIBUTING.md`: retitle the "Nix invalidation-boundary guard" heading to
  describe the static-check derivation split it still documents, and drop the
  probe sentences inside it (source-drift-probe coverage, census runtime
  identity perturbations) and the following `nix-probe-source` paragraph. Reword
  plural probe phrases ("Source-invalidation probes", "existing on-demand
  identity probes") so they name only `coverage-check probe-source`.
- `docs/ARCHITECTURE.md`: remove the `nix-probe-source` paragraph and reword
  "Source-invalidation probes" / "Existing source-boundary probes" likewise.
- Record the decision as a numberless ADR draft (`jaunder-adr`) that narrowly
  supersedes ADR-0227's "keep both manual commands available" for the Nix probe
  only, projected into ARCHITECTURE.md (including the ADR-0227 citation near the
  CI lanes) in the same commit. ADR-0227 stays accepted for
  `coverage-check probe-source`.

## Acceptance

- `just source-diagnostics nix-probe-source` is rejected as an unknown
  subcommand (also locked by the unit test above);
  `just source-diagnostics cache-safety-probe` still runs.
- `rg 'nix-probe-source|nix_probe|NixProbeSource|SourceProbeDrvPaths'` matches
  only this spec, the ADR draft, and the source-diagnostics rejection test.
- `host-proof` tests pass, including the checkout-admission refusal test.
- `just check` passes (formatting, docs links, ADR format/view parity, lints).
- Issue #1946 carries a comment recording the human's retirement decision and
  the full-run evidence (e0382b474, the 4 mismatch kinds above).

## Boundaries

- No change to Nix filters, derivations, or their inline asserts.
- `coverage-check probe-source` and `cache-safety-probe` are untouched; whether
  the coverage probe deserves the same review is a separate question.
- No replacement check is built.
