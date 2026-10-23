# Simplify Post browser-test setup (#1708)

**Status: proposal only — pending explicit human scope/resumption approval.**
This refreshed specification is not implementation authority,
test/build/measurement authority, or tracker authorization. Do not implement,
measure, commit, or change tracker state until the human explicitly approves
this scope and resumption through the established route. Lead approval to draft
this proposal is not that approval.

## Outcome

Reduce incidental composer interaction in exactly seven existing
`end2end/tests/posts.spec.ts` cases by arranging real Posts through the existing
`createPostViaApi` helper under each test's isolated seeded authenticated
Session before the first, one-shot document entry. Preserve the case identities,
all subject assertions and the intended cold/in-app authenticated/anonymous Home
and Local surfaces. This is a narrow test-only setup change, not a suite
redesign.

## Exact scope

The seven runtime cases are:

1. `published Org source blocks are distinct and readable on narrow screens`.
2. `${fixture.name} tokens appear on public, Local, and Home` — the four
   existing fixture rows: Org Haskell (Org), Markdown Emacs Lisp, Markdown
   Haskell, and Markdown Rust.
3. `clearing the body in the editor disables save`.
4. `published post renders at permalink`.

Only incidental UI creation/setup in these cases may change. Use the existing
real `createPostViaApi` mutation and returned Post ID/permalink; it does not
navigate. The `registeredPageFixture` seeds a unique user and applies its
Session before the consumer invokes its one-shot entry function. Arrange the
required published Post or draft using that authenticated context, then make the
first entry the permalink or existing draft editor under test. Do not add
fixture/framework abstractions.

For the rendering/token cases, preserve all current subject checks and
deliberately exercised surfaces: authenticated cold permalink rendering; in-app
movement to Home and its assertions; anonymous cold permalink rendering; and
in-app movement to Local and its assertions, as applicable to the existing case.
Preserve source text, syntax highlighting/token assertions, narrow-screen
geometry/style assertions, and all visibility/authentication checks. For the
empty-body editor case, arrange a draft and enter its editor directly; retain
empty-body disable and non-empty re-enable behavior. For the cold permalink
case, create/publish through the API before entry and make its permalink the one
cold document entry; retain title/body assertions.

Keep tracing, unique per-test identity isolation, backend/browser populations,
current test identities, and one document load per Playwright Page. Seeded
authentication does not navigate (ADR-0098); all movement after entry is in-app
unless an existing narrowly justified allowance remains necessary (ADR-0111). Do
not pre-warm (ADR-0099). Preserve trace capture/attribution (ADR-0096) and the
existing Playwright project/report population and independent trace-gate
authority (ADR-0165). Per-test isolation remains consistent with ADR-0039.

## Independent consumer proofs to retain

The setup removed here must not become the only proof of these separate
behaviors. Keep and cite these existing tests in issue/PR evidence; these are
attribution references, not claims of execution evidence:

- UI Post creation: `authenticated user can create a post through the UI`
  (`posts.spec.ts:117`); additionally
  `create post with tags via UI: tags persist and appear on the post` (`:3127`).
- Format choice and round-trip:
  `full composer: narrow layout stays reachable and format round-trips`
  (`:1644`),
  `edit page: format control prefills accessibly and round-trips a change`
  (`:1772`), and `inline composer: format toggle is named and keyboard-operable`
  (`:3079`).
- Summary/permalink links: `authenticated user can create a post with a summary`
  (`:1476`), `inline composer: publish flash is a link to the post permalink`
  (`:3011`), and
  `inline composer: draft flash links to the draft's canonical permalink`
  (`:3045`).
- Draft lifecycle: `authenticated user can save a draft through the UI`
  (`:1627`) and `draft lifecycle: create, view, edit, and publish` (`:2034`).
- In-app discovery/navigation:
  `inline composer: published post appears in timeline without page reload`
  (`:2914`),
  `inline composer: titleless Post permalink is discoverable from Home and Local`
  (`:2933`), and `tag chip on permalink navigates to site tag listing`
  (`:3162`).

Do not delete or weaken these consumer proofs as part of #1708. Retain the exact
names and current source locations in the eventual issue/PR evidence, updating
locations if source moves.

## Timing acceptance — refreshed campaign policy

Reuse the accepted frozen campaign evidence only where its population,
environment, revision, and execution-duration data establish a meaningful
comparison for this slice. The frozen baseline describes Rust storage-test and
aggregate CI activity; it does not by itself establish comparable execution
timings for these seven browser cases. If compatible pre-change browser evidence
is unavailable, report ordinary post-change timings and explicitly state that a
comparable pre-change browser observation is unavailable and slice-specific
savings cannot be established. Separate browser execution from build/startup
costs where available. Do not collect or repeat a baseline, construct new
baseline arms/matrices, add measurement infrastructure, or perform prolonged
profiling. The historical proposal's >=3-observation-per-arm obligation is
explicitly superseded.

Timing is diagnostic evidence, not a guaranteed performance outcome or an
implementation success claim. Do not claim slice-specific savings without
compatible comparison evidence; cached results, larger timeout budgets, and a
successful test alone are not speed evidence. Do not inherit the historical
proposal's quiescence/resource recipes unless separately authorized and required
by current campaign guidance. No measurement is authorized by this proposal.

## Acceptance after separate human approval

After explicit human scope/resumption approval, implementation acceptance
requires:

- Exactly the seven cases above use real API-created Posts before their first
  entry; no production, snapshots, generic fixture/framework, matrix, or
  unrelated changes.
- All subject assertions, intended cold/in-app authenticated and anonymous
  surfaces, identity isolation, trace behavior, backend/browser populations, and
  one-boot discipline remain intact.
- Independent consumer proofs above remain present and are explicitly attributed
  in issue/PR evidence.
- Focused browser proofs for the changed cases pass; broader required gates and
  canonical VM/CI verdicts remain governed by `CONTRIBUTING.md`. A focused local
  run does not replace hermetic backend/browser parity, coverage, or CI
  authority.
- Timing evidence follows the refreshed campaign terms above, with no guaranteed
  performance claim.

## Boundaries and approval gate

No production changes, snapshot changes, Rust-test consolidation, assertion
repairs outside this slice, timeout/retry/skip/coverage changes, browser/backend
matrix reduction, smoke-selection tooling, or unrelated cleanup. Do not weaken
tracing, identity, population, or validation requirements.

This spec supersedes the historical #1708 proposal's three-observations-per-arm
timing clause only. The archived proposal and review remain historical evidence,
not execution approval; the review's “OK for spec soundness” explicitly did not
approve implementation. Before any implementation, measurement, commit, or
tracker action, obtain explicit human approval to resume this precise scope and
acknowledge its qualifications. Until then, the proposal remains pending and
dormant.
