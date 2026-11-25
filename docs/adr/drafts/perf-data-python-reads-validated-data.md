# ADR-DRAFT: Performance host work is Python that reads already-validated data

- Status: proposed
- Date: 2026-11-25
- Issue: [#2068](https://github.com/jaunder-org/jaunder/issues/2068)

## Context

[ADR-0261](../0261-tool-language-by-maintainability.md) moved the host side of
the performance workflow, the Rust `perf-data` CLI, to Python. It required the
Rust writer to publish a JSON Schema generated from its types, and every reader
in another language to validate against it.

Each measurement VM already validates its own output in Rust. The seeder
(`test-support`) checks the dataset manifest before writing it. `perf-producer`
checks each fragment before writing an ok status: it reproduces the dataset
plan, recomputes every summary from its samples, checks each cursor position,
and requires the complete workload set for the backend and browser the fragment
declares. `perf-data` then ran the same checks again on the host. A schema
validation in Python or TypeScript would have been a third pass over data that
was already checked twice.

## Decision

**Performance host work is Python in `dev`, and it does not re-validate.**
`dev perf`, `dev perf-compare` and `dev perf-import-baseline` keep their
arguments, files and verdicts. `dev perf` reads each producer's status and
fragment in-process. It checks only what no single producer can:

- that the producer succeeded;
- that its fragment fills the backend/browser slot Nix selected (nothing in the
  VM binds the fragment's declaration to that slot);
- that every fragment describes the same dataset.

It then writes the result with provenance, freshness evidence and the canonical
flag. Comparison and baseline import keep their rules: authenticated admission
before download, provenance bound to the run's head SHA and attempt,
`medium`-only canonical baselines, an atomic write, and advisory-only timing
deltas. Their inputs can come from another commit, so they still check
`schema_version`.

**A reader in another language parses data its writer has already validated.**
No JSON Schema is generated for it, and no reader re-validates it. If the format
drifts, the reader fails loudly when it reads a missing field. This amends
ADR-0261's cross-language format rule.

**Rust keeps the writer side only.** It keeps the dataset plan, the manifest and
fragment types, statistics, `validate_manifest` and `validate_fragment`, all of
which the producers use. Run-level types, run assembly and validation,
comparison and the `perf-data` binary are deleted. The browser benchmark's
TypeScript stops checking the manifest the seeder wrote.

**The selection's count rules move into the Nix selector.**
`selectPerformanceProducers` checks the planner's count rules against each
profile's canonical counts, so an invalid count fails at `nix eval`, before any
VM is built. The rules are restated from the Rust planner. Drift between the two
fails loudly either way.

## Consequences

- This amends [ADR-0241](../0241-performance-workflow-ownership.md). No host
  Rust command owns performance-domain decisions any more: `dev` owns host
  assembly, comparison and import, and the VM-side Rust owns validation.
- It amends [ADR-0251](../0251-dev-runner-replaces-just.md): `perf-producer` no
  longer needs to stay apart from a `perf-data` binary, which no longer exists.
- It amends [ADR-0258](../0258-dev-runs-nix-built-host-tools.md): `perf-data`
  leaves its list of Nix-built host tools.
- It amends ADR-0261's cross-language rule, as above.
- `invocation.json` and each producer's `metadata.json` existed only to pass
  data to the Rust CLI, and they are gone. `selection.json`, the result file and
  the retained producer trees are unchanged.
- Accepted losses: the host no longer re-checks fragment contents, browser
  diagnostic file existence and confinement, or a result edited by hand before
  comparison or import. Import also relies on the assembling run for a result's
  completeness: the first baseline has no earlier one to match. Import still
  admits only an authenticated `main` run of the project's own workflow.
