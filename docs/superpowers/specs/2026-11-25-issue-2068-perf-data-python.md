# Issue #2068: move `perf-data` into `dev` as Python, without re-validation

## Outcome

`dev perf`, `dev perf-compare` and `dev perf-import-baseline` keep their
arguments, files and verdicts, but the host side runs as Python inside `dev`.
The Rust `perf-data` binary and its Nix package are deleted. Rust keeps only the
writer side that runs in the measurement VMs. Data that Rust already validated
is read, not validated a third time, and no JSON Schema is generated.

## Approach and tradeoffs

**Who checks what.** The seeder validates the dataset manifest before writing it
(`test-support`). `perf-producer` validates each fragment (plan reproduction,
summaries against samples, cursor positions, a complete workload set for the
backend/browser the fragment declares) before it writes the fragment and an ok
status. Nothing in the VM binds that declaration to the selected slot. Today the
host `perf-data` repeats all of that. After this change the host keeps only the
checks that no single producer can make, plus the admission policy:

| Step                   | Host (Python) does                                                                                                                                                                                                                                                                                                                                                                                          |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| selection              | nothing: Nix `selectPerformanceProducers` now also asserts the count rules (below)                                                                                                                                                                                                                                                                                                                          |
| after each producer    | read `producer-status-v1.json` (`schema_version` 1); on `ok: false` stop with its `detail`, keeping earlier evidence; parse that slot's fragment (by file name) now, so a missing or unparsable fragment stops the run before the next VM; require its producer kind, `setup.backend` and workload browser to equal the slot (the one binding nothing else makes)                                           |
| assembly               | require identical manifests across fragments; write the result: `selection` copied verbatim from Nix's `selection.selection`, `setup`/`workloads` concatenated in selection order, provenance (`GITHUB_*` or `git rev-parse HEAD`), evidence (time, nonce, name-sorted producer identities) and `canonical_selection`; publish atomically to both current paths                                             |
| `perf-compare`         | require `schema_version` 1 on both files, equal dataset plans, equal compatibility-key sets, and every candidate workload present in the baseline; print, per workload, the same fields (workload/backend/browser, baseline and candidate median, p95, min..max) and classification (≥20% by integer `ceil(baseline/5)` threshold; zero-baseline increase), not byte-identical lines; exit 0 on regressions |
| `perf-import-baseline` | run ID parsed as an integer ≥ 1 by click; unchanged admission: run/jobs/artifacts metadata checked before download; then `schema_version` 1, `medium`, `canonical_selection`, GitHub provenance bound to the run ID, head SHA and attempt, key-set compatibility with an existing baseline; atomic write                                                                                                    |

`schema_version` is still checked for compare and import inputs, because those
files can come from another commit. That is a version gate, not re-validation.

**Removed host re-checks, accepted.** These host checks go away: fragment
semantics, browser diagnostic file existence and confinement, and binding the
caller's metadata to the Nix selection. The Nix selection binding is obsolete,
because `dev` now holds that data in-process. Two more go away because they hold
by construction, since Nix passes the same profile and counts to the seeder:
fragment profile equals the requested profile, and count overrides match the
manifest plan. `dev` still refuses a build output that differs from the selected
identity. A schema-valid but hand-edited result would be compared or imported
unchecked. Import accepts only an authenticated `main` run of our own workflow,
so the risk is the cooperative case the policy allows.

**Selection preflight moves to Nix.** `perf-data validate-selection` is deleted.
`selectPerformanceProducers` gains the canonical counts table (small 100/10/500,
medium 5000/100/25000, large 50000/1000/250000) and asserts that posts are
divisible by authors, that revisions ≥ posts, and that revisions / authors > 50
when any override is present. These rules are duplicated with the Rust planner.
Drift is loud either way: Nix rejects a valid run, or the in-VM seeder rejects
an invalid one. `performance-contract.nix` asserts the rejected cases.

**Python placement.** The code goes in `tools/dev/dev/perf.py`, with traced
`steps.phase`s. The nonce comes from `secrets.token_hex(32)`. `invocation.json`
and per-producer `metadata.json` existed only to feed the Rust CLI, and they are
dropped. `selection.json` and the result file layout stay. No new Python
dependency. `PERF_DATA` and `packages.perf-data` are removed, with every
consumer: `nix/platform-contract.nix`'s `portable` list, the `perfDataBin`
package and source filters in `nix/packages.nix`, and the fake `perf-data`
entries in `tools/dev/tests/{conftest.py,test_workflows.py}`.

**Rust library shrinks** to what the producers use: the plan, the manifest and
fragment types, statistics, `validate_manifest` and `validate_fragment`.
`validate_fragment` is rewritten over a fragment-local required-workload set,
with identical rules (schema, manifest, setup, diagnostics count and uniqueness,
per-workload rules, duplicates, missing and unselected workloads). The following
are deleted: run-level types (`RunEnvelope`, `RunSelection`, `RunEvidence`,
`Provenance`, `Baseline`), `assemble_run`, `validate_run`, `is_valid_selection`,
`is_canonical_selection`, `selection_matches_plan`, `comparison.rs`, the
`perf-data` binary, `tools/performance/tests/`, and the crate dependencies the
library no longer uses (`clap`, `anyhow`, `tempfile` if unused). The
fragment-rule tests in `aggregate.rs` and
`test-support/src/performance_tests.rs` are ported to call `validate_fragment`
directly. Only the true run-level cases (provenance, evidence, the setup matrix,
`validate_run`) are deleted; their host behaviour is now tested in pytest.

**TypeScript** (`end2end/tests/browser-performance.ts`): delete the hand-written
manifest checks and the cursor rule checks, keeping only the "manifest lacks
cursor for X" lookup failure. The seeder has already validated the manifest. The
`validateManifest` export goes away. Its callers in
`browser-performance.measure.spec.ts` read the parsed JSON, and its unit cases
in `browser-performance.spec.ts` are deleted.

**Decision record.** A draft ADR,
`docs/adr/drafts/perf-data-python-reads-validated-data.md`, records the
decision. A reader in another language parses data its writer already validated,
with no generated schema; format drift fails at the reader. It amends ADR-0261's
cross-language rule, ADR-0241 (the host Rust command), ADR-0251 (the
perf-producer/perf-data split) and ADR-0258 (the package list). It is projected
into `docs/ARCHITECTURE.md` and the `CONTRIBUTING.md` language rule and
performance section.

Simplest alternative rejected: a line-for-line Python port of `perf-data`. It
would keep a second copy of the planner, statistics and position rules (about
400 lines) only to re-check data that Rust already checked.

## Acceptance

- `cargo` (tools workspace and root) builds with no `perf-data` binary and no
  run-level types. No code, Nix, test or current-doc reference to `perf-data`
  remains (numbered ADRs, the draft and this spec excepted).
- `dev perf small` (storage and Chromium browser on both backends) runs end to
  end and writes `.xtask/performance/<nonce>/performance-result-v1.json`, plus
  the stable copy.
- pytest asserts the result's exact top-level layout and its serde-tagged
  shapes: provenance `{"source","details"}`, and `count_overrides` with null
  fields.
- pytest (`dev`'s 100% coverage gate) covers: a failed status, unsupported
  status schema, missing fragment, or slot mismatch stops later producers and
  keeps earlier evidence; mismatched manifests fail assembly; a
  canonical/noncanonical flag; local and GitHub provenance; atomic publish that
  never truncates on failure; compare's advisory output and refusals; import's
  admission refusals (repository, workflow, event, conclusion, branch, job,
  artifact, provenance tuple, profile, noncanonical, baseline incompatibility,
  non-positive run ID), with no write on refusal.
- The ported `validate_fragment` cases pass (`dev test-local -p test-support`
  and the `performance` crate tests).
- Docs updated: `CONTRIBUTING.md` (perf section, including `invocation.json` and
  the "malformed results" wording, plus the language rule) and
  `docs/ARCHITECTURE.md` perf and language passages.
- `nix eval` of `performance-contract.nix` passes, with new rejected cases for
  indivisible posts, revisions < posts, and ≤50 rows per author.
- The `dev check` gate is green (Rust, Python, TypeScript and docs). The
  host-proof performance workflow test is unchanged and passing.

## Boundaries

- No change to producers, the seeder, VM definitions, the workflow YAML, result
  file names or JSON shape, or the baseline path
  (`tools/performance/baseline-v1.json`).
- No JSON Schema or validator dependency is added anywhere.
- Other cross-language pairs are not audited.
