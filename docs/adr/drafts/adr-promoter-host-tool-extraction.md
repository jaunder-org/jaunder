# ADR-DRAFT: Nix-Selected Python ADR Promoter

- Status: proposed
- Date: 2026-11-07
- Issue: [#1897](https://github.com/jaunder-org/jaunder/issues/1897)

## Context

[ADR-0152](../0152-adr-numbering-happens-after-merge.md) assigns promotion to a
trusted workflow and serialized controller;
[ADR-0088](../0088-promotion-is-the-acceptance-event.md) makes promotion the
acceptance event. The promoter's custom transport and dormant Rust controller
increased maintained responsibilities without adding authority or safety.

This branch implements the approved Python simplification in one cohesive PR
#1917, but the implementation is not shipped or observed on trusted `main`. This
numberless ADR remains proposed. The exact-head human merge gate and post-merge
rollout acceptance remain pending.

## Proposed decision

### SDK mechanics, application policy

Python is the sole mutation owner. Locked PyGithub 2.8.1 supplies transport,
authentication and errors, using the workflow installation token with
`Auth.Token` and `Github(..., retry=None)`. Auto-merge uses
`enable_automerge(expected_head_oid=...)`, which sends `expectedHeadOid`;
read-before-write is not a substitute for that exact-head mutation.

Project only consumed event/PR/head/ref/group identity, commit SHA/parents,
required-check identity/state, queue/rule evidence and mutation postconditions.
Acquire explicitly the first page of at most 100 merge-group runs, never
automatic SDK iteration. Raw GraphQL remains only for observations SDK methods
do not expose adequately, with explicit bounded `first`/`after` paging and
stable observation identity. Effective branch rules use the SDK's authenticated
requester. Implicit object loads must not broaden candidates or hide failures.

Application code owns correlation, required-context policy, pending-rerun
precedence, malformed-evidence rejection and postcondition verification.
Preserve operational, network, authentication, rate-limit and GraphQL errors.
Only the existing PR/snapshot/ref lookup sites interpret typed `NOT_FOUND` as
absence; retain the one best-effort rate-limit-reset diagnostic, without
retrying the failed request. The SDK supplies mechanics, not queue policy.

### One selected package and admission path

Use one standard Nix `buildPythonApplication`, wrapping its entry point directly
with pinned Git and Prettier. The workflow selects the package without a CI
devShell/Just hop, custom outer wrapper or exported inner-package test
interface:

```text
nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller
```

One Python admission path consumes the authoritative Nix source-membership
predicate before mutation, as required by
[ADR-0225](../0225-nix-cached-host-tooling.md). Refuse relevant nonignored
untracked inputs that Git flakes omit; tracked dirty and intent-to-add inputs
participate. Do not duplicate membership rules, stage inputs automatically, add
a digest/RPC framework, or fall back to an ambient or older binary. Generation
separately requires a clean checkout and fresh `main`.

Normal locked realization retains configured Nix substitution. This design does
not guarantee cold source bootstrap or repair upstream bootstrap source
failures; failed realization stays visible, without disabling checks or changing
pins as a fallback.

### Deterministic, source-preserving mutation

Use pinned Marko 2.2.4 GFM structure and link/image destination spans, including
table cells. Translate normalized character boundaries back to original UTF-8
byte offsets and patch the original bytes, never serialize an AST. The approved
eligibility exclusions are orphan textual link substrings, indented code and
multiline code spans. Reference definitions and parser-recognized code remain
unchanged; there is no fallback scanner, duplicate parser or FFI bridge.

Sort draft slugs, allocate every destination, and validate all headings and
whole-set tracked admission before the first move. Preserve complete tracked
renames and staging. Replace only the initial `# ADR-DRAFT: ` heading prefix,
requiring a nonempty first-line title. Inspect the first status line under the
existing trimmed-prefix rule; replace its status only when the trimmed remainder
is exactly `proposed`. Preserve other statuses, malformed multi-token
remainders, prose, indentation and line terminators.

For eligible relative destinations, remove one leading `../` only when a
nonempty remainder follows. Replace assigned draft path stems literally across
tracked-file bytes, not just Markdown. README synchronization preserves curated
titles by number, seeds new titles from headings and retains the existing
marker-delimited table projection/splice behavior. Promotion skips a missing or
markerless README; standalone synchronization retains its read/error behavior.
Inspect promoted files after staging and report link problems as warnings. This
ordered mutation is not an all-I/O transactional rollback.

The controller retains promotion, staged-diff/required-queue checks, staged
Markdown Prettier formatting and restaging, then commit/publication ordering.
Formatting failures remain operational errors.

### Public command retirement and read-only consumers

Retire public Rust `adr promote`, `adr sync-readme` and `adr promoter`, their
delegates, the dormant controller/armer and exclusive helpers in the same PR as
the simplified package. Migrate documented table-only recovery to:

```text
nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- sync-readme
```

Feature authors and shipping flows must not promote. Preserve independent Rust
ADR format, README/view parity, document-link and collision gates. Retain shared
mechanisms only where actual integrated consumers remain. After integration with
the base's performance extraction, xtask has no remaining consumer of its old PR
exports/transport facade or observer dependency; remove those adapters without
changing the independent observer or performance implementation.

### Safety, outcomes and rollout acceptance

Preserve trusted-main authority and the existing repository-scoped App
permissions: Actions read, Contents and pull requests read/write, checks and
commit statuses read, and mandatory Metadata read. Add no Actions-write,
Administration, direct-main or branch-protection bypass authority. Generation
coalescing and per-PR dequeue recovery remain distinct; no active run is
canceled.

Keep open attempt head/diff immutable and fail closed on changed refs or
ambiguous identity. Generate-only recovery permits the two existing interrupted
states: an exact promoter PR without its ref, or a promoter ref without an open
PR. Conflict retirement requires both GitHub conflict evidence and exact local
merge proof against fresh `main`. Use the observed expected-SHA Git lease,
verify ref absence, then verify closure of the same PR. Never force-update an
attempt or use manual promotion as recovery.

Dequeue re-arm requires exact event/PR/head/ref/group/check and commit-parent
correlation, one unambiguous group, and complete green required contexts on both
the unchanged PR head and group SHA. Pending reruns take precedence. Verify
exact-head auto-merge or live queue membership after arming. Accept a failed
write command only after observing its exact successful postcondition.

Safe policy refusals and successful outcomes exit zero. Operational failures,
unsafe inconsistent state and unverified writes exit nonzero. Preserve useful
outcome details and visible workflow/PR/ref failure evidence. Promotion remains
acceptance; only the merge queue writes to `main`.

The cohesive PR requires ordinary required checks and separate human merge
approval for its exact head. After merge, observe trusted-main operation as
rollout acceptance before completing #1897. If observation fails, diagnose and
use a scoped repair or revert, never retain or revive a dormant fallback. Source
conformance, implementation approval and focused proof do not waive these
pending gates or authorize privilege expansion, live mutation as unit proof, or
cleanup.

### Testing responsibility

Remove all tests redundant with the consumed dependencies' own testing, even
when the test invokes our adapter. Trust documented PyGithub, Marko, Git and
standard-library behavior; do not test SDK internals, HTTP mechanics,
retry/pagination engines, parser recognition or provider/library matrices. Test
our bounded evidence selection and consumed policy distinctions; assert our
load-bearing SDK boundary arguments (`expected_head_oid` and `retry=None`), not
that the library implements them. Test our correlation, refusals,
postconditions/outcomes, byte-range edits and complete resulting documents,
staging, admission and runtime wiring. Byte tests cover our original-byte
mapping/patch output, not Marko's AST or normalization implementation. Git
boundary tests cover our lease/conflict argv, exit interpretation and resulting
policy with supplied results, not Git correctness. One selected-package smoke
uses nonprivileged temporary Git to cover our integration, not package bootstrap
or library internals. Avoid per-field/error permutations not consumed by policy
and do not add a fake framework; remove route × action × source-kind × outcome
fake-Nix matrices.

## Consequences

The maintained implementation owns policy and source-preserving mutation, not a
parallel GitHub transport or dormant fallback. Public Rust mutation commands are
removed deliberately; package synchronization is the recovery interface. The
[branch architecture projection](../../ARCHITECTURE.md#host-side-adr-promoter-extraction--branch-implementation-not-shipped)
describes the implemented but unshipped state. Numbering and acceptance remain
post-merge automation, not an authoring or planning-document retirement action.
