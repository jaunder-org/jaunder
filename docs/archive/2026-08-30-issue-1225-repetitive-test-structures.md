# Issue #1225: Repetitive test structures audit

## Outcome

Jaunder has a read-only, evidence-backed audit of repetitive structures in authored product-contract tests across Rust, TypeScript, and Emacs Lisp. Accepted findings become focused remediation issues in milestone 17; generated census data remains ephemeral.

## Load-bearing decisions

- Inventory every authored product-contract test before ranking candidates; use machine signals to find structural clusters, not to sample or score by names and line counts.
- Audit product-contract Rust tests in every workspace crate, including inline tests under `common/src`, `host/src`, `storage/src`, `server/src`, and `web/src`, plus the single integration target under `server/tests`; audit Playwright tests under `end2end/tests` and pure and live ERT tests under `elisp/test`.
- Tooling, gate, coverage-tool, generated, and harness self-tests are excluded except when their helpers are necessary to understand product-test setup.
- Cluster by setup, operation, assertion shape, expected transition or error, and adapter or representation dimension.
- A data-driven test is an opportunity when setup, control flow, operation, transition, and assertion shape are invariant; differing inputs and expected values belong in rows.
- Keep tests separate when consolidation would require control flags, merge different failure stages or side effects, weaken diagnostics, or join contracts that change for different reasons.
- Preserve required SQLite and PostgreSQL evidence while treating an invariant behavior as one shared, backend-parameterized contract; keep genuinely dialect-specific behavior separate. Treat integration plus end-to-end coverage, pure plus live ERT coverage, and separate protocol representations as distinct evidence layers unless duplicated orchestration adds maintenance or drift risk beyond the contract each layer proves.
- Compare languages through shared observable behavior and domain ownership, never through syntax alone.
- Repetition that exposes production knowledge without an owner may justify a deeper production module rather than test-only consolidation.
- File concrete incidental findings after duplicate checking; route unresolved investigation leads to the milestone audit that owns them instead of creating duplicate remediation issues.
- Record no new architectural decision: this audit applies the existing test, backend-parity, dependency-injection, protocol, and harness decisions.

## Acceptance

- The completion evidence provides a reproducible manifest of every included authored test identifier, grouped by root with counts and explicit exclusions, so omitted files, inline modules, or registered tests are detectable.
- Each ranked finding cites exact tests, helpers, operations, assertions, contracts, maintenance risk, locality, and relevant ADR constraints.
- Each candidate states why it is a legitimate data-driven opportunity, a deeper-module finding, or deliberate repetition that must remain separate.
- Accepted remediations are separate one-concern milestone-17 issues using the finding format in `docs/codebase-audits.md`, after searching for duplicates.
- Issue #1225 receives a concise completion comment covering audit coverage, accepted findings, rejected high-signal candidates, and routed incidental leads.
- No tests or production code change, and no generated census report is committed.

## Boundaries

- This issue discovers and records work; remediation belongs to follow-up issues.
- Findings require concrete maintenance or correctness risk, not repetition alone.
- The audit does not redesign intentional backend, protocol, test-layer, or live-harness boundaries.
