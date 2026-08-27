# Codebase Audits

This guide describes how to find and remove accumulated design and maintenance
cruft without turning code quality into a metric-chasing exercise. It covers
read-only discovery, evidence-backed findings, and the transition from a finding
to a focused remediation issue.

The goal is deeper modules: useful behaviour behind small interfaces at clean
seams. This gives callers leverage and keeps change, knowledge, and verification
local to the implementation that owns the behaviour.

## Approach

Use a hybrid audit:

1. Generate a machine-assisted repository census.
2. Rank candidate hotspots using several independent signals.
3. Review complete modules and behavioural slices, not isolated files.
4. Record a small number of evidence-backed findings.
5. Remediate accepted findings as clean cutovers, one focused issue at a time.

A file-by-file walk is useful for local repetition and dead helpers, but it
misses problems distributed across callers and crates. A dependency or call
graph provides an index into the codebase, but graph shape alone does not say
whether a design is good. Automated analysis generates candidates; semantic
review determines whether anything should change.

## Repository census

Build a compact, reproducible inventory of signals such as:

- crate, package, and module dependencies;
- exported symbols and their references;
- approximate fan-in and fan-out;
- production and test function counts;
- large or frequently changed modules and functions;
- normalized syntax clones and repeated test-body shapes;
- common conversion and error-mapping sequences;
- unused dependencies and unreferenced symbols;
- files that frequently change together;
- corresponding SQLite and PostgreSQL adapter paths;
- duplicated constants, validation rules, projections, and state transitions.

Use the most semantic tool available. Rust Analyzer and language servers are
better than syntax trees for definitions and references. `cargo metadata` is
better for crate dependencies. Syntax-aware search is useful for structural
clones, wrapper shapes, and repeated conversion pipelines. Git history supplies
churn and co-change evidence.

The census is not itself a backlog and should not initially gate changes. Every
signal can have a legitimate explanation.

## Ranking hotspots

Prefer candidates supported by several independent signals. A useful mental
model is:

```text
priority = duplication × churn × fan-in × semantic risk
```

The formula is intentionally informal. It prevents a stable ten-line duplicate
from outranking drift between two heavily used storage adapters.

Useful indicators include:

- equivalent implementations that have begun to differ;
- repeated caller-side orchestration of the same operations;
- long chains of one-line wrappers;
- broad interfaces that hide little behaviour;
- repeated argument bundles;
- values converted immediately before and after a call;
- repeated translation between equivalent error representations;
- alternating calls across the same seam;
- several nearly identical callers or tests;
- low-fan-in abstractions whose removal would simplify the code;
- high-fan-in behaviour implemented separately in multiple places.

Large implementations are not inherently problematic. A cohesive implementation
behind a small interface may be exactly the deep module the project wants.

## Audit behavioural slices

A review should follow one behaviour through its complete path, including its
callers, domain representation, interfaces, adapters, results, errors, and
tests. Review relevant domain vocabulary and ADRs before deciding that a
structure is accidental.

At each step, ask:

- What must the caller know to use this module correctly?
- Where does the interface live, and is that the right seam?
- Does the module hide meaningful behaviour?
- Are conversions expressing real ownership changes or compensating for
  incompatible interfaces?
- Is an invariant repeated because no module owns it?
- Do callers repeatedly coordinate lower-level functions in the same order?
- Must tests reach past the interface to exercise observable behaviour?
- Do separate modules represent the same domain concept differently?
- If the module were deleted, would its complexity vanish or reappear across its
  callers?

The last question is the deletion test. Remove pass-through complexity. Keep or
deepen modules whose removal would spread knowledge and behaviour across their
callers.

Account for deliberate Jaunder constraints while reviewing:

- SQLite and PostgreSQL parity can require similar adapter implementations.
- Dependency injection defines intentional seams.
- `CONTEXT.md` defines domain terminology and distinctions.
- ADRs can explain otherwise surprising duplication or layering.
- Representations may differ deliberately across storage, protocol, and UI
  seams.

The task is to distinguish necessary adapter variation from accidental
reimplementation and drift.

## Audit tests through the module interface

Cluster similar tests by setup, operation, assertion shape, expected transition
or error, and adapter dimension.

Use a table-driven test when every row expresses the same observable contract
with different data. Do not combine tests when the table would:

- hide substantially different behaviour;
- require flags that control the test body;
- produce poor failure diagnostics;
- become a miniature interpreter;
- merge tests that should change for different reasons.

Repeated tests often indicate a production design issue. The deeper correction
may be to move repeated behaviour behind one module interface, test that
interface across a case table, and retain thin conformance tests for each real
adapter.

## Record findings

A finding must identify a concrete maintenance or correctness risk, not merely a
code smell. Record:

- **Evidence:** exact modules, symbols, callers, tests, and repeated structures.
- **Problem:** the risk created by the current design.
- **Current seam:** where the relevant interface lives now.
- **Depth assessment:** knowledge or coordination carried by callers.
- **Proposed module:** the smaller interface and behaviour it would hide.
- **Migration:** every caller and adapter affected.
- **Deletion:** superseded functions, conversions, helpers, and tests.
- **Verification:** observable contracts and backend combinations to exercise.
- **Confidence:** confirmed defect, strong design finding, or investigation
  candidate.

Prefer findings about ownership and locality over file-shaped tasks. For
example, “centralize a state-transition invariant behind the repository
interface” is actionable; “refactor `storage/src/example.rs`” is not.

Track accepted remediation work in GitHub issues. Do not preserve a speculative
repository-wide cleanup list as an ad hoc Markdown backlog.

## Agent-assisted audits

Keep discovery read-only. Give an agent a bounded behavioural slice and ask for
evidence-backed findings without edits. For example:

> Audit this behaviour across `common`, `storage`, and `server`. Look for
> duplicate facilities, representation churn, shallow modules, adapter drift,
> dead code, and repetitive tests. Report only evidence-backed findings, ranked
> by leverage and risk. Do not edit.

For a larger area, parallelize by independent audit lens:

- duplication and drift;
- interface friction and conversion churn;
- test structure;
- dead code and shallow wrappers;
- adapter parity and domain invariants.

Alternatively, audit genuinely independent subsystems in parallel. Avoid
splitting reviewers by arbitrary file ranges because that obscures cross-file
relationships. Reconcile and deduplicate the findings before creating issues.

Discovery and remediation should normally be separate. Once a finding is
accepted, request a clean cutover that migrates every caller, removes the
superseded path, and verifies the affected observable contracts.

When an interface or seam has several plausible shapes, design it more than
once. Compare alternatives by seam placement, depth, caller knowledge, adapter
burden, migration cost, and test surface before editing. This is useful for
cross-crate representations and storage interfaces, but unnecessary for a
straightforward consolidation.

## Cadence

Maintain two tracks:

1. Regenerate the repository census periodically and after large milestones.
2. Complete one bounded behavioural-slice audit during a maintenance cycle.

For each slice: investigate, agree on findings, create focused issues, remediate
one issue at a time, verify all affected callers and adapters, delete superseded
paths, and then regenerate the relevant census signals.

A strong first slice is one storage-backed domain operation with both SQLite and
PostgreSQL implementations and callers in another crate. It exposes adapter
parity, conversion churn, interface shape, error mapping, repeated tests,
cross-crate ownership, domain vocabulary, and dependency injection in a bounded
area.
