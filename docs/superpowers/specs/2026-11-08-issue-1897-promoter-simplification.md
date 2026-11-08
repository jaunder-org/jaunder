# #1897: Simplify the ADR promoter

- Status: proposed; architecture is not shipped. Implementation is approved only
  within the separately human-approved outline's recorded scope
- Approval provenance: human response via `9c324a59-7b2c-4519-925a-927c9c89816f`
  to `issue-1897-simplification-design-v2`: “Approve with this boundary.” This
  resolves qualification `ace338e3` for this exact revised design boundary only;
  it does not approve an outline, implementation, builds, pushes, watches,
  privilege, merges, or cleanup.
- Outline/implementation approval provenance: human response via
  `89ec8287-4f38-479c-bcbd-24cf540bfc7c` to
  `issue-1897-simplification-outline-v2` approves the revised outline and
  implementation for the exact outline reviewed at SHA-256
  `3bfd75b7641ae666155bc2fc9f9d4c8177b52aa24415770dc3a3d1415118758c`; scope is
  limited to that outline. The exact-head PR merge gate and post-merge
  trusted-main rollout observation remain separate gates.
- Ordering amendment provenance: human response via
  `2ee4041c-3fd5-4481-9748-b1beb36c8899` to `issue-1897-single-pr-v1`: “Use one
  cohesive PR.” This supersedes only the two-PR/observe-before-retirement
  ordering; it does not waive the separate outline approval, exact-head PR merge
  gate, or post-merge trusted-main rollout observation.
- Issue: #1897

## Outcome

Reduce the promoter's maintained responsibilities while preserving its
authority, immutable-attempt recovery, exact evidence checks, and
promotion-as-acceptance. Python remains the sole mutation owner. The existing
staged repair is separate and is not evidence of approval or implementation
under this proposal.

## Approach and tradeoffs

### SDK mechanics, custom policy

Replace the custom GitHub subprocess transport, response/error parser, and
unused response fields with pinned PyGithub 2.8.1. Use the existing installation
token with `Auth.Token` and `Github(..., retry=None)`. The exact locked source
confirms that `PullRequest.enable_automerge(expected_head_oid=...)` sends
GraphQL `expectedHeadOid`; this is an exact-head mutation, not a
read-before-write substitute. Raw GraphQL remains only for observations the SDK
does not expose adequately.

Project only consumed event/PR/head/ref/group identity, commit SHA/parents,
required-check identity/state, queue/rule evidence and mutation postconditions.
Acquire at most the first page of 100 merge-group runs; consume it explicitly,
never automatic SDK iteration. GraphQL connection paging is separate and must
retain explicit bounded `first`/`after` handling. Implicit object loading must
not broaden acquisition or mask failures. SDK supplies mechanics; custom code
keeps correlation, required-context policy, malformed-evidence rejection and
postcondition verification. Preserve operational, network, authentication,
rate-limit and GraphQL errors; retain narrow `NOT_FOUND`-as-absence only at
existing PR lookup sites and the rate-limit-reset diagnostic, using explicit
SDK-equivalent mechanics where needed. SDK is not queue policy.

### One selected-package path and admission

Keep Nix `buildPythonApplication` and ADR-0225 admission. Wrap its entry point
directly with pinned Git/Prettier; remove the extra `writeShellApplication` and
exported inner-package test interface. Workflow selects the package directly,
without the CI devShell/Just hop:
`nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller`.
This does not cure cold source-realization limits. Do not disable checks or add
a fallback. One Python admission path consumes the authoritative Nix source-
membership predicate before mutation, without duplicating membership rules.
Remove duplicate Rust promoter admission; no ambient binary, digest ledger, or
automatic staging.

### Explicit public command migration

Retire the dormant Rust controller and public `cargo xtask adr promote` and
`cargo xtask adr sync-readme` commands in the same cohesive PR #1917 as the
simplified package, after reviewing actual integrated callers. This is a public
command change, not an internal cleanup. Migrate documented recovery to:

```text
nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- sync-readme
```

Preserve the feature-author prohibition on promotion and independent Rust
read-only format, README/view parity, document-link and collision gates. At
retirement, retain shared mechanisms only where actual integrated callers
remain; verify then. Do not assume or modify cross-project consumers.

### Test responsibility — bounded to our behavior

Trust documented PyGithub, Marko and Git behavior; do not test SDK internals,
HTTP mechanics, retry/pagination engines, or provider/library matrices. Test our
bounded evidence selection and consumed policy distinctions; assert our
load-bearing SDK boundary arguments (`expected_head_oid` and `retry=None`), not
that the library implements them. Test our correlation, refusals,
postconditions/outcomes, byte-range edits and complete resulting documents,
staging, admission and runtime wiring. Real-Git tests cover only our lease and
conflict argv, exit interpretation and resulting policy—not generic Git
correctness. One selected-package smoke test covers our integration, not package
bootstrap or library internals. Avoid per-field/error permutations not consumed
by policy and do not add a fake framework; remove route × action × source-kind ×
outcome fake-Nix matrices.

The prior qualified response `ace338e3-c664-4cda-aa82-8f61cc298c53` /
`issue-1897-simplification-design-v1` was resolved by the exact revised-design
approval recorded above. That approval covers the bounded testing responsibility
and other listed design choices, but not implementation or the separately
required revised-outline approval.

#### Normative unchanged contract

The earlier approved contract at
[`2026-11-07-issue-1897-promoter-extraction.md`](2026-11-07-issue-1897-promoter-extraction.md),
sections “Existing behavior and explicit parser-eligibility differences,”
“Ownership and command compatibility,” “Nix-selected invocation,” and “Preserved
controller and staged delivery,” remains normative except for the stated SDK,
runtime/invocation, public-command, testing, and rollout/delivery-order choices.
The rollout/delivery-order exception is the human-approved amendment
`2ee4041c-3fd5-4481-9748-b1beb36c8899` / `issue-1897-single-pr-v1`: one cohesive
PR, one exact-head merge gate, then trusted-main operation observation as
rollout acceptance before completion. All other controller and promotion
requirements, byte editing, staging, Prettier, independent read-only gates, and
safety guarantees remain unchanged. The older acceptance section's detailed
testing requirements are explicitly superseded by the bounded project-owned test
responsibility above; no other system behavior or acceptance guarantee is
waived.

Safe policy refusals and all successful outcomes exit zero. Operational errors,
unsafe inconsistent state and unverified writes exit nonzero. A failed write
command is accepted only after observing its exact successful postcondition.

### Safety summary

Preserve ADR-0152 trusted-main/App scope; distinct generation/per-PR coalescing;
no active cancellation; immutable attempts; exact event/PR/head/ref/ group/check
correlation; pending-rerun precedence; fail-closed ambiguity; leased
deletion/verified absence; verified close/arm; constrained recovery;
promotion-as-acceptance; and queue-only writes to `main`. Preserve all three
approved Marko exclusions (orphan textual, indented-code, and
multiline-code-span links). No fallback scanner, framework, duplicate parser or
FFI bridge.

Deliver the simplified package, actual caller migration, and obsolete Rust
command/controller retirement together in one cohesive PR #1917, with one exact-
head human merge gate. After merge, observe the operation on trusted `main` as
rollout acceptance before completion. If observation fails, diagnose and use a
scoped repair or revert; do not retain or revive a dormant fallback.

## Acceptance

- Test our bounded evidence selection and consumed policy distinctions,
  correlation/refusals/postconditions/outcomes, expected-head/retry boundary
  arguments, byte edits and whole outputs/staging, admission and runtime wiring.
  Real-Git checks prove our lease/conflict command interpretation and policy;
  one selected-package smoke proves our integration only. Do not test library
  internals, upstream mechanics, or unconsumed field/error permutations.
- Adapter uses locked PyGithub and one admission path consumes the Nix
  predicate; direct package route, recovery and independent read-only gates are
  preserved.
- The cohesive PR #1917 passes ordinary checks and its one exact-head human
  merge gate. After merge, observe trusted-main operation as rollout acceptance
  before completion; diagnose failure with scoped repair or revert, never a
  dormant fallback. Preserve promotion-as-acceptance.

Accounting: raw +4249/-1389; conservative dormant Rust removal 2326 lines. Not
semantic totals or a promised LOC target: describe reduced responsibilities, not
invented net counts.

## Boundaries

The revised design and outline/implementation are approved only within the scope
and provenance recorded above. This does not authorize work beyond the outline;
the exact-head PR merge gate and post-merge trusted-main rollout observation
remain separate requirements. The prior spec/outline remain historical contracts
and are not deleted. Do not modify implementation/tests, packages,
pins/lockfiles, workflow, staged repair/index or accepted ADR/index. No builds,
tests, installs, pushes, watches, staging, unstaging or cleanup.

## Evidence and unresolved choices

Locked nixpkgs revision is `d6524aaca2ff07876657ae2b323f24be4874944b`; its
PyGithub 2.8.1 source hash is
`sha256-36taxa95WrpQw0UUlmnWX4XFslAAuuoousxNh5O5uDA=`. Source confirms
`expected_head_oid` becomes `expectedHeadOid` and `retry=None` disables retries;
no raw mutation fallback is required.

The human approved (1) PyGithub replacing the approved GitHub adapter and direct
package invocation replacing the CI-shell/Just route; (2) retiring Rust commands
and migrating recovery; and (3) focused behavior/package proof replacing route
matrices, under the design boundary in the approval provenance above. The
ordering amendment is recorded above; the single exact-head PR merge gate and
post-merge trusted-main rollout observation remain required. Implementation
approval is limited to the separately approved outline and does not authorize
merging or work beyond its scope.
