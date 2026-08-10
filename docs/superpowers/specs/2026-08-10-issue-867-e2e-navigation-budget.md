# Spec — #867: one boot per page, and a budget that holds it

- Issue: [#867](https://github.com/jaunder-org/jaunder/issues/867)
- Date: 2026-08-10
- Status: awaiting approval

## The finding

The audit ran first, over the existing certified corpus at
`~/measurements/jaunder/issue-866-preload/traces/` (no new capture needed). All
12 traces give identical counts, so arm and browser do not change the answer.

|                                                       |                                 |
| ----------------------------------------------------- | ------------------------------- |
| tests                                                 | 137                             |
| test-attributed navigations                           | 211 (1.54/test)                 |
| tests with 0 navigations                              | 23                              |
| tests with >1 navigation                              | 56, carrying 153 of 211 (73%)   |
| further loads on `e2e.page` spans, **not** in the 211 | 20, on 17 spans across 12 tests |

Concentration: `posts.spec.ts` 90 navigations on 35 tests (2.57),
`profile.spec.ts` 22 on 7 (3.14). Together **53% of navigations on 31% of
tests**. At the other end 23 tests navigate zero times (`polling`,
`static-assets`, `boot-marks`, `feeds`, `media`).

Three causes account for essentially all the excess:

1. **The fixture boots a page the test discards.** `registeredPage`
   (`end2end/tests/fixtures.ts:483`) seeds a session and then `goto(page, "/")`
   at `:486`. 42 consumers take it — `posts.spec.ts` 31, `profile.spec.ts` 7,
   `unicode-slug.spec.ts` 2, `auth.spec.ts:212`, `authed-flash.spec.ts:116` —
   and almost all immediately navigate elsewhere. The fixture is guessing the
   test's entry point and guessing wrong. Its own comment (`:480`) concedes the
   mount is kept only "because its consumers assume one".
2. **Full document loads where the SPA router would serve.** The app is pure
   CSR, and ADR-0076 forbids app code from doing full loads — the suite does
   them from the outside. The repeated `posts.spec.ts` shape is `/` →
   `/posts/new` → permalink, three document loads for one user journey.
3. **Re-loading to prove persistence** — `/profile ×3`, `/admin/site ×3`,
   `/admin/backups ×3`. Read in context (`profile.spec.ts:26`, "A fresh load
   reads the persisted value back through `profile::get`"), the second load **is
   the assertion**. This one is doing a job and stays.

Structural facts the fix rests on: the suite has **no** `beforeEach`/`beforeAll`
hooks anywhere in `end2end/`, so nothing is hidden in lifecycle code; nearly all
navigation funnels through one wrapper, `goto` at `end2end/tests/helpers.ts:67`;
**3 sites call `page.goto` raw**, bypassing it (`layout-shift.ts:67`,
`authed-flash.spec.ts:142`, `:151` — the wrapper's own call at `helpers.ts:73`
is not a bypass); and `page.reload`/`goBack`/`goForward`/`waitForNavigation`
appear nowhere. ADR-0098's seeded auth is substantially realised, and the
remaining login/register navigations are already marked in-source as deliberate
holdouts whose subject is the flow.

## The decision

**Each page performs exactly one document load — its entry — at the URL under
test; all later movement on that page is in-app.**

This is the test-side counterpart of ADR-0076. The suite tests a pure-CSR SPA,
where a user pays a document load once on entry and never again; the suite
should not exercise a path no user takes. Recorded as an ADR (draft at
`docs/adr/drafts/e2e-one-boot-per-page.md`, numbered at ship).

The budget unit is the **Playwright `Page` object**, not the test. A test
needing a second identity opens a second page and boots it, with no ceremony.
What requires a declaration is precisely **a second document load on an
already-booted `Page`** — regardless of whether the URL is the same. That covers
both kept idioms: re-reading the same URL to prove persistence, and loading a
_different_ URL because its cold render is the subject.

Five parts:

1. **`registeredPage` becomes callable and stops navigating on its own.** It
   yields a function taking the entry path; the test's first line is
   `const page = await registeredPage("/posts/new")`. Per-test granularity (a
   `test.use` option is file/describe-scoped and would force `posts.spec.ts`
   into three describe blocks); the entry path stays visible in the test body;
   and there is precedent — `tracedContext` is consumed as `tracedContext()`
   (`fixtures.ts:546`). Seeding stays at fixture-setup time, unchanged; only the
   navigation moves to call time. Calling it twice is an error (the second call
   throws, naming the first entry); never calling it is legal — the test simply
   has no booted page, as the 23 zero-navigation tests already do.

2. **Mid-test document loads become in-app navigation — unless the destination
   is the subject.** Route in-app when the test merely needs to be elsewhere.
   Keep, and declare, a cold load when the assertion is about that page
   rendering from cold: permalink render, boot marks, flash/CLS probes.

3. **In-app navigation gets a synchronisation barrier, not ad-hoc waits.** The
   `goto` wrapper today gives every navigation a `waitForMount`
   (`helpers.ts:78`); a router push has no equivalent, and replacing a barrier
   with scattered selector waits is the most likely source of new flake. A
   `navigateInApp` helper performs the push (clicking the real control where one
   exists) and waits for the destination route to settle, so every conversion
   keeps a barrier.

4. **Two enforcement surfaces, with distinct jobs and no shared blind spot.**
   - **Runtime, at the `Page` level.** The counter subscribes to Playwright's
     own main-frame navigation event on each page, _not_ to the `goto` wrapper.
     No call site can bypass it — raw `page.goto`, wrapper `goto`, and an in-app
     push that unexpectedly triggers a document load are all seen. An undeclared
     second load fails the test, naming the page and both URLs. Declared at
     runtime by `allowSecondBoot(page, reason)` with a non-empty reason.
   - **Static, at the call site.** An xtask check forbids `page.goto` outside
     the wrapper, so navigations keep their synchronisation barrier. Bypass
     sites carry an in-source exemption marker with a reason (ADR-0094). These
     are independent: a static marker exempts a site from the _style_ rule only.
     It never exempts it from the runtime budget, which sees the navigation
     regardless. The runtime counter is authoritative for the budget; the static
     check is authoritative for how a navigation is issued.

5. **Persistence reloads stay.** Idiom 3 is an assertion, not waste. Those tests
   still shed their wasted fixture boot (3 → 2), and their surviving reload
   carries an `allowSecondBoot` reason.

Three raw sites, resolved individually: `layout-shift.ts:67` **cannot** use the
wrapper — the wrapper unconditionally calls `waitForMount` and the probe
deliberately holds the wasm so mount never completes (`layout-shift.ts:55-64`) —
so it keeps a static marker. `authed-flash.spec.ts:142` and `:151` use
`waitUntil: "commit"` and then `waitForURL` through a pre-paint redirect;
routing them through the wrapper would change what is waited on, so they keep
markers too. All three remain subject to the runtime counter.

## Constraints carried in

- **ADR-0099 binds.** This runs _fewer_ navigations. Nothing here warms a cache,
  reuses browser state across tests, or reintroduces a warmup at any scope.
- **Test isolation is not traded.** Playwright still mints a fresh context per
  test; per-test identity fixtures (ADR-0039) are untouched. No navigation that
  exists to guarantee a clean starting state is removed.
- **ADR-0100.** `commitToMount` is Node-frame; it is used whole, multiplied by a
  count, and never decomposed into document-frame segments.
- **`wasmInstantiateMs` and per-segment attribution are not used** to justify or
  evaluate this work (#887). Navigation count and suite wall-clock are the
  measures, and both are sound.

## The classification — the audit deliverable

Before any conversion, every one of the 211 test-attributed navigations plus the
20 secondary-page loads is classified as **removed** (the wasted fixture boot),
**converted** (to in-app), or **kept** (with the reason that will become its
`allowSecondBoot` string or static marker). The classification is a checked-in
artifact.

It does three jobs: it is the issue's "breakdown by cause" and its "recorded
reason not to"; it names which destination pages lose incidental cold coverage,
so that loss is enumerated rather than asserted; and its arithmetic **is** the
pre-registered navigation prediction. The current estimate is ~45 removed; the
exact number is pinned by this artifact and registered **before** any timing arm
is captured.

## Measurement — pre-registered before collection

Protocol per #818/#836/#866: sqlite × {chromium, firefox}, 3 runs per arm,
interleaved, distinct `e2eSalt` per run so nix cannot replay a cached suite,
quiesced host.

- **Deciding set: single-worker** (`workers=1`) — keeps the result comparable
  with the three prior corpora. **Confirming set: gate settings** (`workers=2`),
  reported to convert the finding into the gate saving the issue promises. The
  confirming set carries **no pass criterion**, deliberately:
  `playwright.config.ts:50` ties `fullyParallel` to the worker count, so the two
  sets differ in more than workers and the single-worker ceiling does not
  translate linearly. It is reported, not gated. Fixed here, before collection.
- **Primary metric: suite wall-clock.** The effect is large enough for
  wall-clock to resolve it, unlike #866's per-segment question.
- **Ceiling.** Navigations removed × measured `commitToMount` (911 ms firefox,
  689 ms chromium). At the current ~45 estimate: **≈41 s firefox, ≈31 s
  chromium**, against single-worker suites of ~470 s and ~346 s (#866 corpus).
  Recomputed from the pinned count when the classification lands.
- **Floor: the realised saving must reach ≥60% of the ceiling in both engines.**
  A 3×SE floor would be toothless here (within-arm SD ≈3 s, so 3×SE ≈5 s against
  a 41 s prediction) — exactly the ceiling-only draft #866 was faulted for. A
  mechanism-tied floor can genuinely fail, and fails in the way #836 and #866
  did: a real reduction that does not convert to wall-clock.
- **Directional prediction.** Navigation count is deterministic — all 12 corpus
  traces agree exactly — so the post-change **total document loads** must equal
  the classification's arithmetic. The total counts both buckets
  (test-attributed _and_ secondary-page loads), because a conversion can move a
  load between them and a test-attributed-only count could then be hit or missed
  for attribution reasons alone.
- **Guardrail: `retries` is set above 0 for the measurement runs**, so `flaky`
  is observable at all. At the config default of 0 (`playwright.config.ts:17`) a
  "flaky + unexpected == 0" guardrail is structurally vacuous. Summed
  `flaky + unexpected` must be 0 across each browser's three runs.
- **Abort rule, declared now.** Below the floor, **the idiom and the gate still
  land** — they are justified on test-design grounds independent of wall-clock
  (hidden fixture navigation removed, the real user path exercised, regression
  gated). What fails is the **performance claim**, written up as a negative
  result with the residual investigated and filed. The floor governs the claim,
  not the landing. On the record before collection, so it cannot be a post-hoc
  rescue.

## Acceptance criteria

**A1.** `registeredPage` takes an entry path and performs exactly one
navigation, to that path. No fixture in `end2end/tests/fixtures.ts` navigates
without a destination supplied by its consumer. A second call throws; zero calls
is legal.

**A2.** Every consumer of `registeredPage` names its own entry path. No test's
entry path is `/` unless `/` is where at least one of its assertions runs.

**A3.** A second document load on an already-booted `Page` fails the test at
runtime unless declared, with a message naming the page and both URLs. The
counter is driven by Playwright's main-frame navigation event, so it observes
loads issued through the wrapper, through raw `page.goto`, and through any
in-app navigation that unexpectedly becomes a document load.

**A4.** `allowSecondBoot(page, reason)` rejects an empty or whitespace-only
reason.

**A5.** An xtask static check fails on any `page.goto` outside the `goto`
wrapper unless the site carries an in-source exemption marker with a reason.
`layout-shift.ts:67`, `authed-flash.spec.ts:142` and `:151` carry markers — they
are not routed through the wrapper, for the reasons stated above. A static
marker does not exempt a site from A3.

**A6.** Every surviving second load on an already-booted page carries a reason.
The persistence reloads in `profile.spec.ts`, `admin-site.spec.ts` and
`backup.spec.ts` are among them, as are the internally-navigating helpers
(`registerViaUi`, and the login/register holdouts) wherever a test's next
navigation follows one.

**A7.** Each cold-load subject named in the decision — permalink render, boot
marks, flash/CLS — retains at least one test that loads it cold, and that test's
`allowSecondBoot` reason (or entry-path comment) states the cold render is the
subject.

**A8.** The classification artifact exists, accounts for all 211 test-attributed
navigations and all 20 secondary-page loads, assigns each to removed / converted
/ kept-with-reason, and is committed **before** the first measurement arm is
captured.

**A9.** Observed total document loads after the change equal the
classification's arithmetic exactly.

**A10.** No `expect` assertion is deleted in the conversion. Any test whose
subject changes is named in the write-up, with what it tested before and after.

**A11.** `navigateInApp` (or the clicked control it wraps) waits for the
destination route to settle; no conversion replaces `waitForMount` with a bare
selector wait.

**A12.** `end2end/tests/helpers.ts`'s usage-rules docblock (`:4-38`) and
`CONTRIBUTING.md` state the one-boot-per-page rule, the declaration API, and the
static check.

**A13.** `cargo xtask validate` passes, including all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos.

**A14.** The measurement is executed to the protocol above and the result —
whether it clears the floor or not — is written into `docs/observability.md` as
a `#867` section, with the deciding set, the arms, the pinned prediction, the
ceiling, the floor, and the realised delta.

**A15.** An ADR draft records the one-boot-per-page decision and its
relationship to ADR-0076 and ADR-0099.

## Separable concerns — filed as issues, not folded in

- **Secondary-page navigations are not attributed.** 20 loads sit on `e2e.page`
  spans that carry no `navigation_top_json`, and they do not reconcile with the
  per-test totals (`visibility.spec.ts` "Subscribers post: visible after
  Subscribe…" reports 1 on its test span while its page spans sum to 5). So 211
  under-counts real page loads. Fixing the attribution is a tracing change in
  the ADR-0096 lineage, not a navigation-count change.
- **`verifiedUser` still drives the set-email/verify UI** (`fixtures.ts:541`, 2
  navigations × 5 uses). Deliberate under ADR-0098 — it is
  `email::request_verification` / `email::verify` coverage — and out of scope
  here. Recorded, not filed.

## Out of scope

Anything resembling pre-warming (ADR-0099). Reducing per-navigation cost — that
is #864, #868, #869, #870. Changing worker counts or the CI matrix.
