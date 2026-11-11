# Dependency-security repair (#1939)

## Outcome

Resolve the root/tools/xtask dependency-security findings without weakening
advisory, source or license checks: rustix RUSTSEC-2026-0486, auxiliary
unsafe-libyaml RUSTSEC-2024-0449, and xtask's borrow-or-share MIT-0 rejection.
All three actual host Cargo contexts must pass full cargo-deny. Preserve
application behavior and the existing workflow/collector validation contracts.

## Approach and tradeoffs

Keep the initial rustix repair: raise only its three first-party minimums from
`1.1.4` to `1.1.6`, preserving features/defaults/target conditions, and lock
exactly 1.1.6 in all three contexts. Manifest requirements remain caret ranges;
the lockfiles provide exact pins. No other existing package version is upgraded.

Replace deprecated serde_yaml in `tools/pr-observer/Cargo.toml` and
`xtask/Cargo.toml` with the maintained `yaml_serde` 0.10.7 package through
Cargo's `serde_yaml` dependency alias. This preserves existing imports, Serde
entry points and the YAML Value/Mapping interface, including non-string keys
needed for explicit rejection paths. Lock yaml_serde 0.10.7 and its maintained
libyaml-rs 0.3.0 dependency, removing serde_yaml and unsafe-libyaml from both
auxiliary graphs. Use the normal Cargo resolver and inspect every lock hunk.

This is a maintained fork, not a claim that all C-derived unsafe parsing has
been removed or independently audited. Upstream assumes maintenance and has
published fixes beyond renaming. The alternative safe-Rust parsers inspected
would change the Value/Mapping interface or introduce a young compatibility
surface; that additional consumer churn is unnecessary for this bounded fix.
Focused consumer tests must confirm actual compatibility, not just compilation.

The actual borrow-or-share 0.2.4 license grant/disclaimer matches SPDX MIT-0
(MIT No Attribution), ignoring whitespace and the copyright header. SPDX lists
it as OSI-approved and nondeprecated. Add that reviewed standard identifier to
`deny.toml`'s normal SPDX allowlist with a short rationale. MIT-0 removes MIT's
attribution paragraph; its permissive grant imposes no conflicting obligation on
Jaunder's GPL-3.0-or-later grant. Existing policy already accepts MIT and 0BSD.
Do not falsify declared licensing, add a per-crate exception or change
first-party licensing; retain actual license/source checks for the full graphs.
Replacing jsonschema's unrelated dependency chain solely to avoid this license
would create unnecessary dependency/API churn.

Expanded implementation surfaces beyond the original rustix changes are the two
YAML dependency declarations, necessary auxiliary lock additions/removals,
`deny.toml`, and a focused assertion that the actual GitHub workflow's `on` key
remains a string mapping. No parser adapter, gate framework or public API is
introduced. Preserve unrelated versions, features, source patches and lock
edges; changes outside the necessary replacement closure require explanation,
not a broad update. Source inspection and existing package mechanisms do the
work.

## Acceptance

1. Root/tools/xtask locks each select exactly rustix 1.1.6. Root lock changes
   remain rustix version/checksum only. Both auxiliary graphs contain yaml_serde
   0.10.7 and libyaml-rs 0.3.0, with no serde_yaml or unsafe-libyaml. No
   unrelated package versions/features/edges change. Keep all source patches.
2. Run the actual product host security leaf:
   `cargo run --locked --quiet --manifest-path tools/Cargo.toml -p devtool --bin devtool -- check cargo-deny`.
   Run
   `cargo deny --manifest-path tools/Cargo.toml --config deny.toml --locked check`
   and the equivalent xtask manifest command. All three must pass advisories,
   bans, licenses and sources, without new ignores, warnings overrides or skips.
   Sandboxed mode omitting advisories is not equivalent proof.
3. Run focused actual YAML consumers: the pr-observer package tests, xtask's
   collector-config library tests, and `e2e_topology`, `performance_workflow`
   and `just_workflows` integration targets. Preserve workflow ancestry/matrix
   classification and fail-closed malformed/unsupported input behavior,
   collector privacy-policy digest and strict shape validation, plus the
   workflow `on` key. Require nonempty test selections. No real merge/queue
   operation is a test.
4. Keep valid existing rustix consumer proof when its inputs are unchanged: host
   theme-repository, jaunder shutdown, traced-run library, xtask invocation
   timing and Unix platform tests. Re-run affected auxiliary proof after the
   YAML dependency change; do not duplicate broad validation merely for
   confidence. All normal hook/PR CI requirements remain authoritative for final
   delivery.
5. Run commands through `traced-run --`. Copy complete emitted stdout/stderr and
   honest runner status promptly into ignored review storage, before further
   lifecycle work can prune it. Preserve failures/retry reasons. Supported
   test-local adapters do not emit successful inner per-test output; distinguish
   retained adapter output and non-executing listings from per-test reports.
6. Return one immutable complete diff/spec/proof packet with exact source/tree
   identity and dirty/staged/untracked inventory for Lead-owned independent
   Standards/Spec review. Keep this spec through conformance; evidence remains
   ignored, not a checked-in archive. Lead accepts the integrated reviewed tree
   before the normal hook-backed implementation commit/push/PR CI sequence.

## Authority, historical deviation and boundaries

The initial human decision `rustix-security-intake-v1`, “Approve bounded
repair”, was relayed through Manager background
`b0c1713e-1ecf-4649-b0f2-4522aadbbc3a` and acknowledged by Lead. After explicit
discussion of both auxiliary findings, the human expanded authority via Manager
with “Make the fixes, stat!”. Lead acknowledges this supersedes the pending
deferral-amendment request: fix the findings, do not defer them or repeat a
routine scope gate. Consumer testing seams are confirmed for this approved
replacement. No outline is needed because existing interfaces and architecture
remain unchanged.

Historical deviation: initial resolver/hook and some proof raw logs expired
before review. Original receipts are not logs and cannot recreate that history;
current successful proof was restored separately, without pretending to recover
original operations. Preserve this disclosed deviation and the prior immutable
packets. New execution must retain real emitted output through review. The gap
is not retroactively erased or used to weaken present dependency acceptance.

Preserve assigned branch `fix/rustix-rustsec-2026-0486` and checkout. Commit
this approved expanded working scope before expanded implementation, preserving
the initial rustix patch. Stop with the completed immutable packet before final
implementation commit only for Lead's integrated review/acceptance, not a new
routine approval request. Exact-head human merge approval remains separate.

Issue #1898 is independent in another checkout: coordinate actual shared
pr-observer manifest/auxiliary lock hunks in addition to the known rustix and
traced-run minimum hunks, never replace a whole lockfile or hold its independent
work. Issue #1933 and PR #1937 stay untouched. No merge, reenqueue, cleanup,
resource override, broad upgrade or suppression is authorized by this scope.
