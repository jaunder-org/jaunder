# Protocol and Local Content-Lifecycle ADRs Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record #937's four accepted architectural decisions without presenting
unimplemented behavior as current system truth.

**Architecture:** First file focused, natively blocked implementation issues.
Then write each numberless ADR draft with an accepted-target/current-deviation
projection. Promote only after rebasing: the promotion tool assigns numbers,
rewrites citations, and generates the ADR index.

**Tech Stack:** Markdown ADRs, GitHub issue tracker, `cargo xtask`, Prettier.

## Review header

**Scope — in:** Four ADRs, their `docs/ARCHITECTURE.md` and `CONTEXT.md`
projections, focused follow-up issues, ADR promotion, and generated ADR index.

**Scope — out:** Rust, SQL, migrations, routes, browser behavior, inbound WebSub
or `ajr_*` policy, restore/revert, and purge implementation.

**Tasks:**

1. File and triage focused implementation-debt issues.
2. Record publisher-side WebSub and its target/current split.
3. Record HybridWindow membership and its target/current split.
4. Record HTTP validation and its target/current split.
5. Record local Post lifecycle and its target/current split.
6. Promote, verify, and commit the accepted decision set.

**Key risks/decisions:** #937 must not canonize existing defects. ADR-0102 is
immutable; the HybridWindow ADR records a narrow later supersession only for the
two feed-window keys. Tuple ETags are deliberately retained over body hashing,
so serializer-input completeness is a maintained invariant.

## Global Constraints

- Implement exactly the approved specification at
  `docs/superpowers/specs/2026-08-14-issue-937-protocol-content-lifecycle-adrs.md`.
- File follow-ups before promotion. One concern per issue; use the task table's
  required type, milestone #12, Priority, topic labels, project #1, and native
  blocked-by #937 relation; read each issue back after creation.
- Start ADRs from `docs/adr/template.md`. In a draft, link accepted sibling ADRs
  as `NNNN-slug.md` and other drafts as `../drafts/slug.md`; outside a draft,
  cite it as `docs/adr/drafts/slug.md`. These forms survive promotion.
- Do not edit `docs/README.md`. `cargo xtask adr promote` assigns numbers,
  rewrites citations, and owns the generated index.
- Preserve accepted ADR text. The later HybridWindow ADR narrows ADR-0102; do
  not patch ADR-0102.
- `docs/ARCHITECTURE.md` labels accepted target and current implementation
  separately until every linked implementation issue lands.
- Run `devtool run -- cargo xtask check` before each commit. Do not add a
  `Co-Authored-By` trailer.

---

## File Structure

| Path                                                | Responsibility                                                                               |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| GitHub issues                                       | Durable implementation contracts and resolving links for each current deviation.             |
| `docs/adr/0137-publisher-side-websub.md`            | Outbound WebSub topic, trigger, durability, configuration, retry, and recovery policy.       |
| `docs/adr/0139-syndication-feed-hybrid-window.md`   | Feed-membership union, visibility/ranking, time, configuration, and corruption policy.       |
| `docs/adr/0138-syndication-feed-http-validation.md` | Strong ETag, Last-Modified, and RFC 9110 conditional-response policy.                        |
| `docs/adr/0136-local-post-lifecycle.md`             | Local deletion, revision, retention, media, access, and non-purge policy.                    |
| `docs/ARCHITECTURE.md`                              | Materialized current view: accepted targets and linked present deviations.                   |
| `CONTEXT.md`                                        | WebSub Publish Ping, Deleted Post, Post Revision, Collection, and active permalink identity. |
| `docs/DESIGN.md`                                    | Explicit assessment for user-facing deletion/retention language.                             |
| `docs/README.md`                                    | Generated ADR table; changed only by `cargo xtask adr promote`.                              |

## Tasks

### Task 1: File and triage implementation-debt contracts

**Files:**

- Create: six GitHub issues in `jaunder-org/jaunder`.

**Interfaces:**

- Produces one stable issue URL per concern for Tasks 2–5.
- Every issue has a native blocked-by #937 relation. It is neither a sub-issue
  nor a blocker of #937.
- Produces no code API and references #937 plus a decision topic, never a
  numberless ADR draft path that promotion cannot rewrite.

- [ ] **Step 1: Write six issue bodies.** Every body states current behavior,
      accepted target, affected backend/protocol, observable acceptance cases,
      and #937. Create exactly these one-concern contracts:

  | Title                                                                      | Type    | Milestone / Priority | Contract                                                                                                                                                                                                                                                                                               |
  | -------------------------------------------------------------------------- | ------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
  | `Fix transactional public-feed invalidation across authoring protocols`    | Bug     | #12 / P1             | AtomPub POST/PUT/DELETE and web mutations atomically enqueue the same public-projection changes; anonymous/Public eligibility precedes feed ranking.                                                                                                                                                   |
  | `Correct publisher-side WebSub configuration and retry recovery`           | Bug     | #12 / P1             | Hub set/change/unset invalidates all cached feeds; worker/regenerator use one snapshot; malformed hub is unset while configuration/site reads retry; classify 408/429/5xx/transport versus terminal 4xx; honour bounded Retry-After; separately inspect/redrive regeneration and publish dead letters. |
  | `Activate and harden Syndication Feed window settings`                     | Bug     | #12 / P1             | Valid min-items/min-days writes durably invalidate all feeds before return; checked overflow means all history; corrupt values return errors.                                                                                                                                                          |
  | `Implement complete Syndication Feed HTTP validators`                      | Bug     | #12 / P1             | Deterministic complete ETag tuple; persisted identity-change time; malformed condition cannot return 304; RFC 9110 INM list/wildcard/weak precedence; truthful GET/HEAD/304 headers and bodies.                                                                                                        |
  | `Implement full local Post revision history and retained media references` | Feature | #12 / P2             | Full prior-state revision, no-op suppression, owner list/detail history, revision-aware ordinary media reference guard, and force-delete disclosure.                                                                                                                                                   |
  | `Prevent AtomPub idempotency replay from exposing Deleted Posts`           | Bug     | #12 / P1             | A retry against an existing key whose original Post is deleted returns the deleted-resource response, never a 200 Entry.                                                                                                                                                                               |

- [ ] **Step 2: Create and triage each issue.** Use `mcp__github__issue_write`
      with `owner: "jaunder-org"`, `repo: "jaunder"`, the table's type,
      milestone #12, and existing topic labels. Add it to Jaunder Backlog
      project #1, set the table's Priority, and create its native blocked-by
      #937 dependency. Read it with `mcp__github__issue_read`; rewrite an
      angle-bracket-mangled body in prose. Record every URL.

- [ ] **Step 3: Verify tracker contracts.** Confirm six focused open issues,
      required type, milestone #12, project membership, Priority, native
      blocked-by #937 relation, and stated acceptance cases. Expected: PASS.

- [ ] **Step 4: Record completion without a repository commit.** Tick this task
      after tracker read-back. Its durable artifact is GitHub; an empty Git
      commit is forbidden.

### Task 2: Record publisher-side WebSub

**Files:**

- Create: `docs/adr/0137-publisher-side-websub.md`.
- Modify: `docs/ARCHITECTURE.md` WebSub publishing section.
- Modify: `CONTEXT.md` Syndication vocabulary.

**Interfaces:**

- Produces `docs/adr/0137-publisher-side-websub.md`, cited by the architecture
  view.
- Defines **WebSub Publish Ping** as content-free outbound notification naming
  one concrete public Syndication Feed URL as topic.

- [ ] **Step 1: Write the failing documentation contract.** Require all
      Site/User/SiteTag/UserTag × RSS/Atom/JSON topics, one optional site-wide
      hub, public-projection trigger, protocol parity, transactional outbox,
      cache-before-ping, at-least-once delivery, separate regeneration/publish
      budgets, dead letters, and redrive. Require hub set/change/unset
      invalidation, late binding to one hub/site snapshot, malformed stored hub
      purge-as-unset, read-error retry, and disabled-hub completion without
      replay. Name current AtomPub omission, non-atomic web enqueue, coarse
      trigger, stale discovery, incoherent configuration reads, NoHub error
      collapse, all-error retry, ignored Retry-After, shared budget, and missing
      redrive.

- [ ] **Step 2: Verify it fails before the ADR.** Confirm no accepted ADR
      records this outbound policy and current-deviation links are absent.
      Expected: FAIL.

- [ ] **Step 3: Write draft and projection.** Use template heading, `proposed`
      status, date, and #937. Explicitly distinguish this publisher leg from
      ADR-0010's inbound WebSub delivery; cite ADR-0015, ADR-0016, ADR-0021,
      ADR-0027, ADR-0092, ADR-0102, and Task 1 issue URLs. Replace each present
      #937-only deviation placeholder with its resolving issue. Add the
      direction-qualified glossary term without changing inbound terminology.

- [ ] **Step 4: Verify the contract.** Check Context/Decision/Consequences,
      governing constraints and alternatives/tradeoffs, target/current labels,
      and resolving links for every named deviation. Expected: PASS.

- [ ] **Step 5: Run documentation checks.** Run:
      `devtool run -- cargo xtask check --no-test` Expected: PASS.

- [ ] **Step 6: Commit.** Tick this task, run
      `devtool run -- cargo xtask check`, stage tracked projection files, and
      commit: `docs(adr): define publisher-side WebSub`.

### Task 3: Record Syndication Feed hybrid membership

**Files:**

- Create: `docs/adr/0139-syndication-feed-hybrid-window.md`.
- Modify: `docs/ARCHITECTURE.md` Syndication feeds section.

**Interfaces:**

- Produces `docs/adr/0139-syndication-feed-hybrid-window.md`, cited by the
  architecture view.
- Defines `HybridWindow` as
  `eligible_before_rank -> ordered union(prefix(min_items), since(min_days))`.

- [ ] **Step 1: Write the failing documentation contract.** Require anonymous/
      Public eligibility before ranking, `published_at DESC, post_id DESC`,
      20/30 defaults, inclusive fixed 24-hour UTC cutoff, regeneration snapshot,
      durable invalidation before successful setting return, overflow to all
      history, and corruption error. Require a narrow later supersession of
      ADR-0102 only for corrupt min-items/min-days reads; leave ADR-0102
      unchanged.

- [ ] **Step 2: Verify it fails before the ADR.** Confirm no ADR records the
      union rationale, activation boundary, or read-error policy. Expected:
      FAIL.

- [ ] **Step 3: Write draft and projection.** Cite ADR-0020, ADR-0027, ADR-0063,
      ADR-0102, Task 1's authoring-invalidation issue for
      rank-before-visibility, and its settings issue for activation, overflow,
      and corruption. State each as linked deviation, never policy.

- [ ] **Step 4: Verify the contract.** Confirm all decision branches, the narrow
      ADR-0102 relation, current/target labels, alternatives/tradeoffs, and one
      resolving issue per mismatch. Expected: PASS.

- [ ] **Step 5: Run documentation checks.** Run:
      `devtool run -- cargo xtask check --no-test` Expected: PASS.

- [ ] **Step 6: Commit.** Tick this task, run
      `devtool run -- cargo xtask check`, stage tracked projection files, and
      commit: `docs(adr): define Syndication Feed hybrid window`.

### Task 4: Record HTTP representation validation

**Files:**

- Create: `docs/adr/0138-syndication-feed-http-validation.md`.
- Modify: `docs/ARCHITECTURE.md` Syndication feeds section.

**Interfaces:**

- Produces `docs/adr/0138-syndication-feed-http-validation.md`, cited by the
  architecture view.
- Defines complete semantic ETag identity and an unnamed persisted
  representation-modification-time contract. It chooses no storage field or Rust
  API name.

- [ ] **Step 1: Write the failing documentation contract.** Require
      deterministic complete tuple identity and stability on identical semantic
      inputs/bytes, serializer revision spanning ADR-0015/ADR-0089 paths, RFC
      9110 weak INM list, wildcard, precedence, and
      malformed-condition-never-304 behavior; GET/HEAD non-match and no-body 304
      behavior; 304 validators/cache metadata; persisted whole-second time
      changing only on identity change; and downstream `max-age=300`.

- [ ] **Step 2: Verify it fails before the ADR.** Confirm no ADR defines
      complete tuple membership, representation time, or the full conditional
      matrix. Expected: FAIL.

- [ ] **Step 3: Write draft and projection.** Cite ADR-0015, ADR-0089, and the
      Task 1 HTTP-validator issue. State incomplete current ETag inputs,
      item-derived date, headerless 304, and narrow parser behavior as linked
      divergence. Reject body hashing only as a decision alternative, not a
      current-code claim.

- [ ] **Step 4: Verify the contract.** Confirm GET/HEAD body rules, 304
      metadata, tuple/time stability, alternatives/tradeoffs, and resolving
      links. Expected: PASS.

- [ ] **Step 5: Run documentation checks.** Run:
      `devtool run -- cargo xtask check --no-test` Expected: PASS.

- [ ] **Step 6: Commit.** Tick this task, run
      `devtool run -- cargo xtask check`, stage tracked projection files, and
      commit: `docs(adr): define Syndication Feed HTTP validation`.

### Task 5: Record local Post lifecycle

**Files:**

- Create: `docs/adr/0136-local-post-lifecycle.md`.
- Modify: `docs/ARCHITECTURE.md` Content model section.
- Modify: `CONTEXT.md` Publishing and AtomPub vocabulary.
- Assess: `docs/DESIGN.md` user-facing retention and deletion language.

**Interfaces:**

- Produces `docs/adr/0136-local-post-lifecycle.md`, cited by the architecture
  view.
- Defines **Deleted Post** and **Post Revision** independently of AtomPub Entry
  and inbound `ajr_entry_versions`.

- [ ] **Step 1: Write the failing documentation contract.** Require canonical
      Post identity; active-surface omission; active-only permalink and
      syndicated identity reuse; full prior revision fields—source, rendered
      representation, summary, tags, audiences, media,
      creation/modification/publication/deletion timestamp/state;
      meaningful-change rule; storage-API immutability; owner list/detail
      without revert; indefinite retention; revision-aware ordinary media guard;
      force-delete override; and future purge explicitly undecided.

- [ ] **Step 2: Verify it fails before the ADR.** Confirm ADR-0009 is
      inbound-only and no local lifecycle ADR defines retention, revision
      boundary, or access. Expected: FAIL.

- [ ] **Step 3: Write draft and projection.** Cite ADR-0009 as a near-miss,
      ADR-0015, ADR-0020, ADR-0021, ADR-0064, and ADR-0090. Cite Task 1's
      lifecycle issue for partial/no-op revisions, missing history, and released
      media; cite its Deleted Post replay issue separately. Add/clarify glossary
      terms and Collection active-Post membership. Assess `docs/DESIGN.md`:
      update it if it promises erasure or omits retained local content;
      otherwise record in the ADR consequence that its goals make no lifecycle
      promise and leave it unchanged.

- [ ] **Step 4: Verify the contract.** Confirm local/inbound terms cannot
      conflate, target/current stays labelled, forced deletion is explicit,
      alternatives/tradeoffs are recorded, and every deviation links its issue.
      Expected: PASS.

- [ ] **Step 5: Run documentation checks.** Run:
      `devtool run -- cargo xtask check --no-test` Expected: PASS.

- [ ] **Step 6: Commit.** Tick this task, run
      `devtool run -- cargo xtask check`, stage tracked projection files, and
      commit: `docs(adr): define local Post lifecycle`.

### Task 6: Promote and verify the accepted decision set

**Files:**

- Move: four drafts from `docs/adr/drafts/` to numbered `docs/adr/` files using
  `cargo xtask adr promote`.
- Modify: `docs/README.md` and tracked draft-path citations, owned by promotion.
- Verify: `docs/ARCHITECTURE.md`, `CONTEXT.md`, and all promoted ADRs.

**Interfaces:**

- Consumes four valid proposed drafts and all tracked citations.
- Produces four accepted collision-free numbered ADRs, generated index entries,
  and rewritten architecture citations.

- [ ] **Step 1: Rebase and inspect before numbering.** Rebase onto
      `origin/main`. Resolve documentation conflicts without changing accepted
      ADR text. Confirm all six Task 1 issues exist and every deviation links
      its resolver. For every draft, check exact `ADR-DRAFT` heading, `proposed`
      status, date, #937 link, Context/Decision/Consequences, governing ADR
      constraints, and rejected alternatives or explicit tradeoffs.

- [ ] **Step 2: Promote drafts.** Run: `devtool run -- cargo xtask adr promote`
      Expected: four collision-free numbers, `accepted` status, moved files,
      regenerated `docs/README.md`, rewritten citations, and staged output.

- [ ] **Step 3: Verify promotion outputs.** Confirm no tracked path still links
      `docs/adr/drafts/`; each accepted ADR is cited in the architecture view;
      and `Un-ADR'd reality` no longer has #937's four bullets while #938 stays
      intact.

- [ ] **Step 4: Run the full documentation gate.** Run:
      `devtool run -- cargo xtask validate --no-e2e` Expected: PASS, including
      ADR format, generated README parity, view parity, links, formatting,
      static checks, and coverage.

- [ ] **Step 5: Commit.** Tick this task, inspect staged files, and commit:
      `docs(adr): record protocol and local content lifecycle decisions`.

- [ ] **Step 6: Recover from post-promotion collision.** If another ADR lands
      before merge and rebase exposes a collision, run
      `devtool run -- cargo xtask adr renumber`, stage all rewritten references
      and generated index changes, then amend the promotion commit. Do not use a
      fixup. Expected: rebase plus
      `devtool run -- cargo xtask validate --no-e2e` passes.

## Self-review

- **Spec coverage:** Tasks 2–5 map to AC2–AC5/D2–D5; Task 1 and Task 6 map to
  AC1/AC6–AC9; Task 5 maps D6. No production implementation task appears,
  preserving D7/out-of-scope boundaries.
- **Placeholder scan:** Every task names files, contracts, verification, and
  commit boundary. Only ADR numbers are deliberately deferred to promotion.
- **Type consistency:** `WebSub Publish Ping`, `Deleted Post`, `Post Revision`,
  `HybridWindow`, ETag, and persisted representation-modification time are
  consistent; this docs-only plan invents no Rust API.

## Execution handoff

Plan complete and saved to
`docs/superpowers/plans/2026-08-14-issue-937-protocol-content-lifecycle-adrs.md`.
After approval, execute with `jaunder-iterate`, ticking each checkbox before its
commit gate and using `jaunder-dispatch` only for independently verifiable
tasks.
