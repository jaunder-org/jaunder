# ADR-0111: The e2e suite boots each page once

- Status: accepted
- Date: 2026-08-10
- Issue: [#867](https://github.com/jaunder-org/jaunder/issues/867)

## Context

The app is a pure-CSR `leptos_router` SPA.
[ADR-0076](0076-no-full-load-spa-navigation.md) forbids app code from performing
in-app full document loads, and enforces it with the `no-full-reload` static
check: within a live SPA session all navigation is client-side.

The e2e suite was never held to the same rule from the outside. #867's audit,
over the certified corpus at `~/measurements/jaunder/issue-866-preload/traces/`,
measured **211 test-attributed navigations across 137 tests — 1.54 per test**
(plus 20 further loads on secondary-page spans that the 211 does not count). 56
tests navigate more than once and carry 73% of the total; `posts.spec.ts` and
`profile.spec.ts` alone hold 53% of the navigations on 31% of the tests.

A navigation is not cheap. #866 measured `commitToMount` — what the suite
actually waits on — at **911 ms on firefox** and 689 ms on chromium. Over the
208 mounted navigations of a run that is 190 s per suite run, against a
single-worker firefox suite of ~470 s in the same corpus. Firefox is the gate's
critical path.

Three causes, all in test code:

- The `registeredPage` fixture seeded a session and then navigated to `/`. 42
  tests took it and almost all immediately navigated somewhere else — the
  fixture was guessing the test's entry point, and paying a full boot for the
  wrong guess.
- Tests moved between pages with a second `page.goto`, a full document load,
  where the app's own router would have served. This exercises a path no user
  takes: in a CSR SPA, arriving at a route by router push and arriving by cold
  load run different code.
- Some tests re-load the same URL to prove a value persisted. That is an
  assertion rather than waste **only where the app offers no way back to the
  route**. Where it does, an in-app re-entry remounts the page and its
  `Resource` refetches from the server, so the reload proves nothing the router
  would not have proved — a premise this cycle initially got wrong for
  `/admin/site` and `/admin/backups`, and corrected once it was tested (see the
  archived classification's Amendment 3).

Two prior cycles tried to make a navigation _cheaper_ and both underdelivered:
#836 cut the wasm bundle 57.6% for ~60 ms/navigation, and #866's preload removed
166–195 ms of serial waiting for ~19 ms of boot total and was reverted for
missing its floor. Something else in the boot chain stays serial and absorbs
segment-level wins. Removing a whole navigation has no such problem.

[ADR-0099](0099-e2e-does-not-pre-warm.md) rules out the other direction: the
suite does not pre-warm, at any scope. So the lever is the count.

## Decision

**Each page in the e2e suite performs exactly one document load — its entry — at
the URL under test. All subsequent movement within that page is in-app.**

This is the test-side counterpart of ADR-0076.

1. **Fixtures do not choose the entry point.** `registeredPage` yields a
   function taking the entry path; the test's first line is its single boot, and
   a second call throws rather than booting again. No fixture navigates without
   a destination supplied by its consumer.

2. **The budget unit is the Playwright `Page`, not the test.** A test needing a
   second identity opens a second page and boots it — one boot per page, no
   declaration. What requires a declaration is precisely **a second document
   load on an already-booted `Page`**, whatever its URL.

3. **That second load is declared with a mandatory reason.** Legitimate cases
   exist and are kept, not smuggled: the destination page's cold render being
   the subject (permalink, boot marks, flash/CLS probes), and re-reading state
   to prove persistence **on a route the app offers no way back into**. The
   reason string is the record of what was deliberately left alone, so it must
   name the thing that makes the load necessary — "the value is re-read from the
   server" is not that, since an in-app re-entry re-reads it too; "nothing in
   the app links to this route" is.

4. **An allowance that nothing consumes fails the test.** A declaration does not
   expire, so one written for a load that never happens sits in the queue and
   silently absorbs the _next_ extra load — precisely the undeclared second load
   the budget exists to catch. Over-declaring therefore does not waste a line;
   it disarms the check for the rest of the page's life, invisibly. This is
   [ADR-0094](0094-gate-exemptions-in-source-markers.md)'s orphan-marker rule
   ("a marker whose site no longer exists fails") in runtime form: an exemption
   nothing re-verifies must at least be checked to still apply. It is not a
   theoretical hazard — arming the budget caught a real over-declaration, an
   `authed-flash` test that declared two further loads where only one arrives on
   chromium.

   **One exemption, and only one: a load whose occurrence depends on the browser
   engine** (see the `DOMContentLoaded` consequence below). Whether such a load
   happens is not the test's choice, so an unconsumed declaration for it is no
   evidence that the test over-declared, and a second declaration form
   authorises at most one and is exempt from this rule. It stays narrow — a
   reason is still mandatory and must say _what_ varies by engine — because it
   is the one place the budget gives up its only machine-checkable claim about a
   written exemption. A load that always happens keeps the exact form.

   **That form is scoped to the path of the load it describes, and the scope is
   what bounds it.** An exemption that can survive unconsumed is an exemption
   looking for something to absorb: unscoped, it would be handed to whatever
   loaded next on that page, and a genuinely undeclared load would pass silently
   — the one outcome the budget exists to prevent. Scoped, it matches only its
   own path and is inert against everything else, and consumption spends a
   matching scoped declaration before an exact one, so a page carrying one of
   each behaves the same on both engines whatever order the two lines were
   written in.

   **The residual cost, stated rather than solved:** an exempt declaration still
   blinds this rule by one slot on its page. With exact declarations A and B
   alongside a scoped one, if B's load regresses away, the loads that remain
   spend A and B, the scoped declaration survives exempt, and B's disappearance
   is never reported. Nothing closes that while loads carry no identity beyond
   their URL; the path scope narrows it to same-path loads, which is why the
   scope is mandatory and why the form stays rare. The unconsumed allowances are
   collected by the same teardown sweep that raises violations (point 5) — one
   pass over the pages a test armed, reporting both kinds — because they are two
   readings of the same state and a second pass would be a second chance for the
   two to disagree about which pages a test touched.

5. **Two enforcement surfaces with distinct jobs.** The **budget** is enforced
   at runtime, at the `Page` level, by subscribing to the page's own
   `domcontentloaded` event — armed by the traced-context fixture on every page
   it creates, so a test opts out of the budget only by not using the suite's
   own fixtures, not by forgetting a call. It is not armed from the navigation
   wrapper. Counting at the wrapper would leave every raw call site as a blind
   spot, including the ones that legitimately cannot use the wrapper;
   subscribing at the page sees a document load whoever issued it — though what
   it sees is the event, not the navigation, which is the engine-dependent limit
   recorded below. `domcontentloaded` rather than `framenavigated` is
   load-bearing: `framenavigated` also fires for same-document
   `history.pushState`, which is exactly what an in-app router move is, so
   counting it would flag every conversion this work makes. The cost is stated
   below: a document that is replaced before it reaches `DOMContentLoaded` is
   not counted.

   **Detecting a breach is only half of it; raising one takes two raisers, and
   both are needed.** The event handler cannot reject its caller's promise, so
   it records the breach and something else must raise it. The navigation
   wrapper raises at its next call, which is the earliest and most informative
   moment — the test fails near the offending line. The teardown sweep raises
   the rest, because a page whose test issues no later wrapper call reaches no
   such moment, and the sites that legitimately cannot use the wrapper are
   exactly the ones on that path. With only the wrapper raiser, a breach on
   those pages would be detected and then discarded: enforcement that lapses
   precisely where the exemptions live. The sweep runs unconditionally but
   throws last, after the trace export and only on a test that otherwise passed,
   so a budget failure can never mask a real one.

   One honest limit on the arming claim: a declaration made on a page the
   fixtures have not armed arms it late and infers the entry load, and it infers
   one only from a real document — a page still at `about:blank` has not booted,
   and treating that as the entry would let the page's first real load consume
   the declaration and leave the genuine second load uncounted.

   Separately, the `e2e-goto-wrapper` xtask static check forbids `page.goto`
   outside the wrapper, so navigations keep their synchronisation barrier — the
   same shape as `no-full-reload`, and for the same reason: without a gate,
   nothing forbids the drift that produced 1.54. That check's in-source
   exemption markers follow ADR-0094 (line above the site, reason required,
   orphan fails, census derived and printed), and **no file is exempt, including
   the wrapper's own module** — its single `page.goto` carries an ordinary
   marker, so a second call added there later is not silently covered and the
   wrapper appears in the census like every other exempt site. Such a marker
   excuses a site from the style rule only; it never excuses it from the budget,
   which is why the budget does not read them.

6. **In-app movement carries a synchronisation barrier.** The wrapper gives
   every document load a mount wait; a router push has no equivalent, so the
   in-app helper waits for the destination route to settle, and requires a
   readiness selector that does **not** already match before the move — a
   barrier that waits for nothing is rejected rather than silently accepted.
   Replacing a barrier with scattered selector waits would trade suite time for
   flake.

## Consequences

- **The count is bounded and the bound is checked.** 1.54 navigations per test
  accumulated over years because no rule and no gate opposed it. A per-page
  budget makes the next regression fail at the offending test rather than show
  up in a suite-time audit two cycles later — at the offending line where a
  later wrapper call exists, and at that test's teardown otherwise.
- **Entry points are visible.** Every test states where it boots, on its first
  line, instead of inheriting a fixture's guess.
- **Tests exercise the user's path.** Mid-test router pushes are what the app
  actually does; a mid-test `goto` was testing a transition the SPA never
  performs.
- **Cold-load coverage becomes deliberate rather than incidental.** It survives
  where it is the subject, declared, instead of being scattered across tests
  that did not intend it. This is a real movement of coverage, not a free win:
  assertions that previously ran against a freshly-booted runtime now run
  against a warm one. Which destination pages lose an incidental cold render is
  enumerated, not asserted — #867's classification artifact lists every
  converted navigation.
- **This does not warm anything.** ADR-0099 is untouched: fewer navigations, not
  warmer ones, and no browser state is reused across tests. Per-test isolation
  (ADR-0039, a fresh context per test) is likewise unchanged — no navigation
  that exists to guarantee a clean starting state is removed.
- **A declaration is a cost.** Adding one is deliberately mildly annoying; that
  is the mechanism. The failure mode to watch is declarations added reflexively
  with a thin reason, which review should treat as it would an exemption marker
  anywhere else.
- **Whether a document replaced before `DOMContentLoaded` is counted depends on
  the engine.** Measured on the pre-paint `/`→`/app` redirect, a
  `location.replace` run during head parsing: `framenavigated` fires for both
  URLs everywhere, but `domcontentloaded` fires for `/` on firefox and not on
  chromium, which replaces the document first. So the budget counts two loads
  there and one here. This is the price of not counting router pushes, and it
  falls on a document that, where it is uncounted, nothing ever rendered.

  It has a consequence for the API: **no fixed number of declarations is right
  for such a flow**, so the budget needs a second declaration form authorising
  _at most one_ load **of a named path** and exempt from the orphan rule (point
  4). It is deliberately narrow — only a load whose existence varies by engine
  may use it, and its reason must say what varies — because the orphan rule is
  the only thing a machine can check about a written exemption, and this form
  gives that up. It also means the budget's count and the trace corpus's
  navigation count can legitimately differ by one on such a flow, on one engine
  and not the other; #867's classification artifact records both observations.

- **Secondary-page loads remain unattributed.** 20 loads sit on `e2e.page` spans
  without navigation detail and do not reconcile with the per-test totals, so
  the headline count under-reports. Filed separately; it is a tracing gap in the
  ADR-0096 lineage, not a count problem.
