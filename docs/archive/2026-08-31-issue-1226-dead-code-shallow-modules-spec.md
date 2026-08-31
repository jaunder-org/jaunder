# Issue #1226: Dead code and shallow modules audit

## Outcome

Jaunder has a read-only, evidence-backed audit of dead code, unused facilities, pass-through wrappers, and shallow modules across all authored repository source. Confirmed findings become focused remediation issues in milestone 17; raw analyzer output remains ephemeral.

## Load-bearing decisions

- Freeze the audited commit and publish and read back the complete source, dependency, query, grouping, threshold, capability, and fallback manifest before semantic candidate inspection. Any amendment requires publication and a complete rerun of every affected query.
- Enumerate every authored Rust, TypeScript, Emacs Lisp, test-support, developer-tooling, script, and build-support source file, plus every declared Cargo, npm, Nix, and other build dependency; exclude generated, vendored, build-output, migration, and fixture-data trees explicitly.
- Publish finite source, declared-dependency, definition, symbol, module, occurrence, and candidate-group populations with exact methods, counts, exclusions, capability results, and omission checks.
- Use semantic references where available and reader- or syntax-aware analysis elsewhere. A manifested finite fallback may replace an unavailable analyzer only when it enumerates the same declared population and records its limitations.
- Confirm code dead only when the complete declared reachability suite or its finite fallback succeeds and every applicable macro, registration, feature, target, test, interactive, script, external, tooling, and cross-language path has a terminal resolution. Any unresolved or failed required path remains a low-confidence candidate.
- Classify reachability as confirmed live, confirmed dead, or low-confidence candidate. Only confirmed dead code may produce a deletion issue; low-confidence candidates remain named audit evidence.
- Inspect low-fan-in functions, one-line wrappers, module facades, re-exports, traits, and interfaces whose callers retain nearly all implementation knowledge.
- Typed contracts and their policy invariants remain protected; the concrete wrappers, modules, DI/storage implementations, and protocol machinery around them remain auditable. Dependency-injection, protocol, authorization, storage-dialect, observability, lifecycle, test-support, and tooling boundaries receive no exemption merely for naming a boundary.
- A shallow abstraction earns its weight only by hiding caller knowledge, participating in an already valuable recurring convention, stabilizing variable implementations, localizing change or failure interpretation, or shielding unstable mechanics.
- For an outlier abstraction, compare removal with normalization. Normalize only when the established convention already earns its weight and owns the same policy; never deepen a seam for symmetry alone.
- Apply the deletion test to every proposed removal or merger: prefer the alternative with the best net maintenance effect after accounting for lost policy locality and convention leverage.
- Preserve intentional assembly surfaces, generated registrations, target/feature variants, and external entry points when deletion would spread knowledge or break reachability.
- Search for duplicate issues before filing concrete incidental or primary findings; route unresolved leads to the audit that owns them.
- Record no new architectural decision: the audit evaluates existing seams without changing domain vocabulary or accepted policy.

## Acceptance

- Durable issue evidence identifies the audited commit and reconciles exhaustive grouped source, declared-dependency, definition, module, occurrence, declared-query, and candidate-group populations so omissions are detectable.
- Every analyzer capability is reported as successful, unavailable, or failed; no unavailable or failed cell is interpreted as clean.
- Every manifested occurrence and candidate group receives exactly one terminal disposition: evidence-backed rejected, exact prior-covered, low-confidence, or promoted and then accepted/rejected. Disposition counts reconcile to the complete population.
- Every promoted candidate has exact declaration/reference/registration evidence, reachability confidence, callers, current seam, deletion test, removal-versus-normalization assessment, maintenance risk, locality, and relevant ADR constraints.
- High-signal rejected candidates state the concrete cognitive leverage or dynamic reachability that justifies retention.
- Accepted remediations are separate, duplicate-checked, one-concern milestone-17 issues using the finding format in `docs/codebase-audits.md`.
- Issue #1226 receives a concise completion comment covering audit coverage, confirmed findings, low-confidence candidates, rejected high-signal candidates, capability limits, and routed incidental evidence.
- No production or test code changes, and no generated audit report is committed.

## Boundaries

- This issue discovers and records work; remediation belongs to follow-up issues.
- Repetition and low line count are candidate signals, not findings.
- A group classified as covered by #1223, #1224, or #1225 cites the exact prior disposition and audited-commit identity. Reopening a prior rejection requires a named commit-specific evidence delta; prior records are never blanket exclusions.
