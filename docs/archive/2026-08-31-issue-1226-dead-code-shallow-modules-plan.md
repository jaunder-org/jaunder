# Issue #1226 Dead Code and Shallow Modules Audit Outline

> Execute with `jaunder-iterate` and delegate bounded slices with `jaunder-dispatch`. This outline exists because exhaustive cross-language and dependency analysis requires stable multi-agent contracts.

## Scope

In:

- Frozen authored-source and declared-dependency populations across Rust, TypeScript, Emacs Lisp, test support, tooling, scripts, and build support.
- Reachability, low-fan-in wrapper, shallow-module, deletion-test, and removal-versus-normalization analysis.
- Durable manifest, reconciled dispositions, duplicate-checked remediation issues, and completion evidence.

Out:

- Production or test remediation in this issue.
- Generated, vendored, build-output, migration, and fixture-data trees.
- Reopening prior audit dispositions without exact commit-specific evidence.

## Task outline

- [x] Task 1: Freeze and publish the audit manifest
  - Contract: one read-back issue comment names the audited SHA; exact roots/suffixes/exclusions; source and dependency populations; query IDs; grouping and low-fan-in thresholds; analyzer capabilities; finite fallbacks; artifact schema; and amendment/rerun rule.
  - Verification: comment readback matches the local canonical manifest byte-for-byte; every tracked eligible root and dependency manifest is classified.

- [x] Task 2: Audit declared dependencies
  - Contract: `.xtask/audits/issue-1226/dependencies.json` implements the shared artifact schema and covers every declared Cargo, npm, Nix, and other build dependency from Task 1.
  - Verification: declaration totals reconcile by manifest/ecosystem; each dependency group has one terminal disposition and explicit analyzer/fallback status.

- [x] Task 3: Audit Rust reachability and module depth
  - Contract: `.xtask/audits/issue-1226/rust.json` implements the shared artifact schema across root, `xtask`, and `tools` workspaces, including targets, features, macros, registrations, tests, support binaries, and prior-audit mappings.
  - Verification: every Rust occurrence/group from the frozen queries has one terminal disposition; confirmed-dead classifications resolve every applicable reachability path.

- [x] Task 4: Audit TypeScript reachability and module depth
  - Contract: `.xtask/audits/issue-1226/typescript.json` implements the shared artifact schema across `end2end`, including Playwright config/reporters, fixture registration, project matrices, init scripts, subprocess/tool paths, and prior-audit mappings.
  - Verification: every TypeScript occurrence/group from the frozen queries has one terminal disposition; dynamic registrations and cross-language strings are resolved.

- [x] Task 5: Audit Emacs Lisp reachability and module depth
  - Contract: `.xtask/audits/issue-1226/elisp.json` implements the shared artifact schema across production modules, tests, runners, and tooling, including reader forms, generated struct accessors, interactive/autoload entry points, filename registration, dynamic symbols, and prior-audit mappings.
  - Verification: every Elisp occurrence/group from the frozen queries has one terminal disposition; reader and dynamic-entry limitations are explicit.

- [x] Task 6: Audit scripts and non-language build support
  - Contract: `.xtask/audits/issue-1226/support.json` implements the shared artifact schema across authored shell, Nix, configuration, and other non-Rust/TypeScript/Elisp source from Task 1, including operational entry points and external tool invocations.
  - Verification: every support-source occurrence/group from the frozen queries has one terminal disposition; script and build entry paths are resolved.

- [x] Task 7: Reconcile findings and file remediation issues
  - Contract: `.xtask/audits/issue-1226/reconciled.json` deduplicates cross-slice groups, preserves every initial and terminal disposition, applies the deletion and removal-versus-normalization rubric, and contains complete `docs/codebase-audits.md` issue drafts. Each accepted finding records its duplicate-search evidence, one-concern issue number/URL, milestone 17, type, labels, priority, and body readback.
  - Verification: source artifacts, reconciled groups, prior-covered mappings, low-confidence candidates, rejections, and accepted findings reconcile exactly; promoted groups equal accepted plus rejected with zero unresolved terminals; tracker readback proves each created issue's body and metadata.

- [x] Task 8: Publish completion evidence and archive audit documents
  - Contract: issue #1226 receives durable population/disposition totals, capability results, accepted and rejected findings, low-confidence candidates, prior mappings, incidental routing, and links to remediation issues; raw artifacts remain gitignored. Archive the approved spec and this completed outline.
  - Verification: completion comment readback satisfies every spec acceptance clause; branch diff contains only archived audit documents.

## Shared artifact schema

Each slice artifact records `audited_sha`, `manifest_version`, `slice`, `populations`, `capabilities`, `queries`, `occurrences`, `groups`, `prior_coverage`, `findings`, `rejected_candidates`, `low_confidence`, and `incidental`.

- `populations` is a finite set of named buckets. Every declared query records its `query_id`, population bucket, executed status, raw count, occurrence IDs, and either zero-result evidence, analyzer result, or finite-fallback/error detail.
- Every occurrence names its query IDs and belongs to exactly one group, so omitted queries or cross-query overlap remain detectable.
- Every group records one initial disposition: `rejected`, `prior-covered`, `low-confidence`, or `promoted`. A promoted group separately records one terminal disposition, `accepted` or `rejected`; before Task 8, `promoted = accepted + rejected` and no promoted terminal is unset.
- Every promoted group records exact declarations, references and registrations, confidence and resolved reachability paths, callers, current seam, deletion test, removal-versus-normalization comparison, maintenance risk, locality, and relevant ADRs.
- Every high-signal rejection records the concrete convention leverage, caller knowledge, policy locality, or dynamic reachability that justifies retention.
- Counts and canonical SHA-256 values reconcile populations, declared queries, occurrences, groups, initial dispositions, and terminal dispositions without publishing raw analyzer output.
- A confirmed-dead finding records all applicable reachability paths as resolved; any unresolved or failed required path forces `low-confidence`.

## Risk checks

- Do not infer clean from unavailable/failed Rust Analyzer, TypeScript language server, Emacs reader, dependency analyzer, target, feature, registration, or cross-language evidence.
- Preserve typed contracts while auditing their concrete implementation weight.
- Treat DI, protocol, storage dialect, authorization, observability, lifecycle, test-support, and tooling seams as evidence questions, not exemptions.
- Credit consistency only when the convention already reduces caller knowledge or change locality.
- Do not let capped census output, lexical reference counts, line count, or fan-in thresholds define the backlog.
- Prior-covered dispositions cite the exact prior record and audited SHA; reopening names the commit-specific delta.
