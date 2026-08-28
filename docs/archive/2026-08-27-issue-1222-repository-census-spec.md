# Repository Census

## Outcome

Jaunder provides a reproducible, host-side `cargo xtask census` command that
emits a compact repository-census report for maintenance audits. The same
command supports `--json` for machine-readable comparison without turning census
candidates into findings, backlog items, or check failures.

## Load-bearing decisions

- The census runs in `xtask`, consistent with ADR-0028's host-analyzer boundary.
- The command's stable interface is `cargo xtask census [--json]`:
  human-readable output by default and the same report data in xtask's
  structured result envelope and sidecar for JSON mode.
- Reports use repository-relative identities, deterministic ordering, and a
  stable section for every signal family named by this specification so runs can
  be compared across maintenance cycles.
- The source snapshot is the current content of Git-tracked files under the root
  Rust workspace, `xtask`, `tools`, `end2end`, and `elisp`. Untracked, ignored,
  vendored, generated, and build-output paths are excluded.
- Churn and co-change use the full non-merge history reachable from `HEAD`. Git
  rename detection attributes history to current tracked paths; working-tree
  changes affect source analysis but do not rewrite commit history.
- Every required language/signal cell has exactly one collection state: `clean`,
  `candidates`, `unavailable`, or `failed`. A signal section deterministically
  summarizes its cells. Unavailable and failed cells must never appear clean.
- Any failed cell makes the command fail, while the report preserves results
  from collectors that completed. An unavailable optional analyzer does not make
  the command fail.
- Every collector records its identity, version when externally supplied,
  evidence method, and material limitation. Evidence methods are `semantic`,
  `structural`, or `heuristic`; the report does not invent numeric confidence
  scores.
- Semantic definitions and references come only from a semantic analyzer. A
  missing semantic analyzer produces `unavailable`; syntax or text matching must
  not masquerade as semantic evidence.
- The report always accounts for these signal families across Jaunder's owned
  Rust, TypeScript, and Elisp trees where applicable:
  - dependency structure and exported-symbol references;
  - normalized syntax clones and repeated test shapes;
  - common conversion and error-mapping sequences;
  - unused dependencies and unreferenced symbols;
  - churn and co-change;
  - corresponding SQLite and PostgreSQL adapter paths.
- The minimum implemented collector matrix is:
  - dependency structure for Rust, TypeScript, and Elisp;
  - semantic exported-symbol references and unreferenced symbols for Rust and
    TypeScript;
  - structural clone and repeated-test-shape detection for Rust, TypeScript, and
    Elisp;
  - structural conversion and error-mapping detection for Rust and TypeScript;
  - heuristic churn and co-change for every tracked source tree;
  - heuristic SQLite/PostgreSQL adapter-path correspondence.
- Unused-dependency analysis and any language/signal cell outside that minimum
  matrix remain present but may report `unavailable` until a sound collector
  exists. Missing runtime tooling may also make an implemented collector
  unavailable; it does not excuse omitting the collector or its conformance
  fixtures.
- SQLite/PostgreSQL analysis inventories paired and unpaired adapter paths and
  may emit structural review candidates. It never claims behavioral equivalence
  or treats legitimate dialect differences as defects.
- Generated census output is ephemeral. The repository owns the command and
  documentation, not a committed generated backlog or baseline.
- The census is manual audit input and is not added to normal check, pre-commit,
  pre-push, or validation gates.

## Acceptance

- `cargo xtask census` succeeds in the declared development environment and
  prints a compact, deterministic human report covering every required signal
  family.
- `cargo xtask census --json` succeeds over the same repository state and
  exposes the same section states, candidates, evidence methods, collector
  metadata, and limitations in structured form.
- Repeated runs over an unchanged checkout produce equivalent ordered report
  data apart from explicitly non-comparable runtime metadata.
- A signal with no candidates reports `clean` rather than disappearing.
- A missing optional analyzer reports `unavailable`, names the missing
  capability, and cannot be mistaken for a clean result.
- A malformed or partial collector result reports `failed`, preserves other
  completed sections, and causes a non-zero command result.
- Lower-confidence structural and heuristic candidates are visibly distinct from
  semantic evidence.
- The adapter-path section identifies corresponding and unmatched
  SQLite/PostgreSQL paths without asserting behavioral parity.
- Focused tests cover command parsing, deterministic report rendering and
  serialization, clean and candidate cells, unavailable tooling, collector
  failure, and preservation of partial results. Every minimum-matrix collector
  has positive and negative fixtures proving it detects its intended candidate
  shape without flagging the clean counterpart.
- User documentation gives the reproducible command, explains each signal and
  method class, states tool and language limitations, and states that the report
  is neither a finding list nor a gate.

## Boundaries

- This issue discovers and reports candidates; it does not judge them, rank
  remediation work, create issues, or refactor production code.
- This issue does not add census thresholds to CI or any local verification
  ladder.
- This issue does not commit generated census output or establish a historical
  baseline file.
- This issue does not promise semantic reference analysis where the declared
  environment lacks a sound analyzer.
- This issue does not prove SQLite/PostgreSQL behavioral parity; existing
  backend tests remain the correctness authority.
- This issue does not introduce new domain vocabulary or alter Jaunder's product
  behavior.
