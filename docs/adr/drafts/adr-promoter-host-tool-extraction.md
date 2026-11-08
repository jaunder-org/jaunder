# ADR-DRAFT: Nix-Selected Python ADR Promoter

- Status: proposed
- Date: 2026-11-07
- Issue: [#1897](https://github.com/jaunder-org/jaunder/issues/1897)

## Context

[ADR-0152](../0152-adr-numbering-happens-after-merge.md) assigns promotion to a
trusted GitHub Actions workflow and serialized controller. This proposal changes
implementation ownership, not its authority, acceptance event or recovery
guarantees. The revised design boundary was approved by the human via
`9c324a59-7b2c-4519-925a-927c9c89816f` / `issue-1897-simplification-design-v2`:
“Approve with this boundary.” This resolves qualification `ace338e3` for the
revised design only. The architecture is not shipped; revised-outline and
implementation approval was subsequently granted with the revised outline by
human response `89ec8287-4f38-479c-bcbd-24cf540bfc7c` /
`issue-1897-simplification-outline-v2`, limited to that outline's scope. The
exact-head PR merge gate remains outstanding. The human also approved one
cohesive PR via `2ee4041c-3fd5-4481-9748-b1beb36c8899` /
`issue-1897-single-pr-v1`: “Use one cohesive PR.” This changes only rollout
ordering, not design substance or testing boundaries.

## Revised proposal — approved design, proposed and not shipped

Keep Python as the sole mutation owner. Replace custom GitHub transport,
response parsing and unused fields with locked PyGithub 2.8.1 (`Auth.Token`,
`retry=None`); `enable_automerge(expected_head_oid=...)` sends
`expectedHeadOid`. Limit the merge-group acquisition to the first 100 runs,
explicitly consumed without automatic iteration; handle GraphQL connections
separately with explicit bounded pagination. SDK object loading must not broaden
evidence or mask operational, network, authentication, rate-limit, GraphQL or
malformed-evidence failures. Preserve narrow `NOT_FOUND` absence only at current
PR-lookup sites and the rate-limit-reset diagnostic. SDK supplies mechanics, not
queue correlation or policy; retain only consumed evidence and custom exact
identity/check/parent correlation and authorization.

Keep Nix `buildPythonApplication`, one Python admission path consuming the
single authoritative Nix source-membership predicate, and direct selected-
package invocation without the CI devShell/Just hop or extra outer wrapper. This
does not resolve cold source-realization limitations. In the same cohesive PR
#1917 as the simplified package, retire the Rust controller and public
`cargo xtask adr promote` / `cargo xtask adr sync-readme` commands after
reviewing actual integrated callers; document package `sync-readme` as recovery.
The revised outline and implementation are approved within the scope recorded in
Context; one exact-head PR merge gate remains required. After merge,
trusted-main operation observation is rollout acceptance before completion; on
failure diagnose, then use scoped repair or revert, with no dormant fallback.
Preserve independent Rust read-only format, README/view parity, link and
collision gates. At retirement, retain shared mechanisms only if actual
integrated callers remain; do not assume or modify cross-project consumers.

Testing concerns our behavior, not library internals: focused checks cover
bounded evidence selection, consumed policy distinctions, correlation,
refusals/postconditions/outcomes, load-bearing SDK arguments, byte edits,
resulting documents/staging, admission/runtime wiring, and our Git
lease/conflict command interpretation. One package smoke proves integration
only. Trust published PyGithub/Marko/Git behavior; do not test HTTP, SDK
internals, retry or pagination engines, provider matrices, or unconsumed
field/error permutations. This explicitly supersedes only the older spec's
excessive testing obligations; all system guarantees remain. The prior qualified
response `ace338e3-c664-4cda-aa82-8f61cc298c53` /
`issue-1897-simplification-design-v1` was resolved by the exact revised-design
approval recorded above. That design approval covers these design and testing
choices; the later, separate outline/implementation approval is limited to its
recorded scope. Preserve all three approved Marko eligibility exclusions; no
fallback scanner, duplicate parser, framework or FFI bridge. Keep the single PR
merge gate and post-merge trusted-main rollout observation.

This numberless ADR remains proposed; it does not alter any accepted ADR
Decision or index. The approved design boundary does not ship the architecture.
Revised-outline and implementation approval were granted only within the
recorded scope; the single exact-head human PR merge gate remains outstanding.
See the
[revised spec](../../superpowers/specs/2026-11-08-issue-1897-promoter-simplification.md)
for the bounded design and approval provenance. The
[architecture projection](../../ARCHITECTURE.md#proposed-final-simplification--approved-design-not-shipped)
remains explicitly prospective and does not change the current-branch
description.

## Locked source evidence

`flake.lock` pins nixpkgs revision `d6524aaca2ff07876657ae2b323f24be4874944b`;
its PyGithub package is 2.8.1 with source hash
`sha256-36taxa95WrpQw0UUlmnWX4XFslAAuuoousxNh5O5uDA=`. That release's
`PullRequest.py` passes `expected_head_oid` as `expectedHeadOid`; its
`MainClass.py` documents `retry=None` as disabling retries.

## Historical approval

The earlier design selected a custom `gh` adapter, retained Rust mutation
delegates and specified a temporary controller command. Approval was recorded as
`cd55b65a-4f7d-426d-991a-04cc256ec206` / `issue-1897-python-design-v2`, against
baseline `82bd37273dd533026fbf07f0121865fe254583a8`. That approval did not
itself authorize the revised proposal; the separate revised-design approval is
recorded in Context. This numberless draft remains proposed; no accepted
Decision text or generated index is changed here. The outline and implementation
are approved only within the recorded scope; the single exact-head human PR
merge gate and post-merge rollout observation remain required.
