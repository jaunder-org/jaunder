# Spec — issue #794: close the e2e trace-coverage gaps from the #788 investigation

Status: awaiting approval Issue:
<https://github.com/jaunder-org/jaunder/issues/794> Provenance: the #788
investigation
(<https://github.com/jaunder-org/jaunder/issues/788#issuecomment-5154329459>),
CI run 30714621799.

## Problem

The #788 investigation hit eight places where e2e trace data could not answer a
question directly. The headline: **28–31 % of per-test wall-clock is invisible**
— it falls outside the `e2e.test` span, so no amount of trace analysis can
attribute it. Downstream work is blocked on this: #792 must measure what the
per-test warmup buys before deleting it, and the mount-cost lever cannot be
sized without a `commit_to_mount` breakdown.

All eight gaps are closed in this cycle.

## Governing principle — factor, then instrument once

**Instrumentation is never cut-and-pasted into call sites.** Where the same
sequence of steps recurs, the sequence is factored into one helper and _that_ is
instrumented. This is already the repo's practice: `posts.ts` records that
"'Create a post' was inlined dozens of times in two styles … These promote both
into one place" (#262).

This principle decides the shape of gaps 3, 4 and 6. It is why gap 6 becomes one
polling primitive rather than six wrapped loops, and why gap 4 becomes one
context-level attach rather than per-page opt-in.

## Design decisions

### D1 — Capture and attribution are separate concerns

`_autoPerfSpan` attaches page instrumentation _after_ `warmupPageContext`
(`fixtures.ts:426` warmup, `:432` traceparent, `:447`/`:467` capture). The
comment at `:429-431` attaches the reason to the **traceparent**: warmup traffic
must stay unattributed per #681's orphan-bucket design. Capture's late
attachment is ordering, not a constraint of its own — and that incidental fusion
is why warmup's duration is measured nowhere.

They are split: **capture attaches at context creation; the per-test traceparent
is still applied only after warmup.** #681's contract is untouched.

### D1a — Capture is phase-aware, so `e2e.test`'s attributes do not move

Warmup is **on** in the gate (`flake.nix:947`, `JAUNDER_E2E_WARMUP=1`).
Attaching capture earlier therefore means warmup's navigation and its ~10
requests start flowing into the same `requests[]` / `navigations[]` arrays that
produce `e2e.request_count`, `e2e.navigation_count`,
`e2e.request_top_slow_json`, `e2e.navigation_top_json` and
`e2e.action_top_json`. Left unhandled, this would shift those values by about
one navigation and ten requests per test — silently breaking comparability with
every #788 number, which is exactly what D2 exists to prevent.

So the capture **sink is phase-aware**. `attachTraceCapture` writes into a sink
the fixture can swap; `_autoPerfSpan` swaps it from the warmup sink to the test
sink at the same moment it applies the traceparent. Records collected before the
swap populate `e2e.warmup`; records after populate `e2e.test`, whose arrays
therefore contain exactly what they contain today. AC-3 asserts this on values,
not just keys.

### D2 — Span hierarchy: a lifecycle envelope

`e2e.test`'s span id is the #681 attribution join and its time range is what
every existing analysis means by "in-span time". It is **not** widened — doing
so would silently redefine every number in #788.

```
e2e.test.lifecycle                 (first auto-fixture stamp → just before export)
├── e2e.warmup                     (only when JAUNDER_E2E_WARMUP is on)
├── e2e.context_mint
├── e2e.test                       (unchanged: same span id, same range, same attrs)
│   └── … server request spans, as today
├── e2e.page                       (one per instrumented page beyond the default)
└── e2e.teardown
```

**`e2e.test`'s parent changes, and that is safe by construction.** The analyzer
selects on exact span name (`analyze.rs:66, 450, 472` — `s.name == "e2e.test"`,
so `e2e.test.lifecycle` cannot collide), and the coverage extractor walks
`parent_span_id` _upward_ to a span named `e2e.test`
(`server_fn_coverage/extract.rs:47-53, 76`). Nesting `e2e.test` under a new
parent therefore cannot break #681. AC-3 deliberately omits "parent" from its
unchanged-list for this reason.

### D2a — `e2e.context_mint` needs a stamp from before the `page` fixture

`_autoPerfSpan` declares `{ page, testSpanId }` (`fixtures.ts:424`), so
Playwright has already built context and page before its body runs — context
mint cannot be timed from inside it. The stamp comes from `_autoTestTimeout`
(`fixtures.ts:325-333`), the existing auto fixture with no `page` dependency,
which runs first. `e2e.context_mint` spans that stamp to `_autoPerfSpan`'s
entry.

### D3 — The residual is named, not hidden

Playwright tears fixtures down in reverse setup order, so `_autoPerfSpan`'s
teardown — where the span is built and `exportSpans` POSTs — runs _before_
`context.close()`. Our own OTLP export and the context teardown are structurally
unmeasurable from inside the thing doing the measuring.

This residual is **measured and then documented in `docs/observability.md` as
the known floor**, not papered over. Its size is not pre-committed (see AC-4).

### D4 — Polling is one primitive, built on Playwright's `toPass`

The poll loop is copy-pasted **six** times, `deadline` / `while` / `sleep`:

| Site                                        | Interval | Timeout | Polls                       |
| ------------------------------------------- | -------- | ------- | --------------------------- |
| `mail.ts:63` `waitForNewEmail`              | 100 ms   | 5 s     | mail capture file           |
| `fixtures.ts:384` `mailbox.waitForNewEmail` | 100 ms   | 5 s     | mail file, recipient-scoped |
| `websub.ts:61` `waitForNewPing`             | 250 ms   | 30 s    | websub capture file         |
| `websub.ts:87` `waitForPingMatching`        | 250 ms   | 30 s    | websub file, predicate      |
| `visibility.spec.ts:313`                    | 500 ms   | 25 s    | feed via `page.request.get` |
| `feeds.spec.ts:33` `fetchFeedContaining`    | 500 ms   | 25 s    | feed via `page.request.get` |

None is wrapped in `withTimedAction`, which is why 17.6 s of the WebSub test
(#793) vanishes from the trace.

One `pollUntil` in a new `end2end/tests/polling.ts`, built on
`expect(...).toPass({ timeout, intervals })`, replaces all six. Three properties
are load-bearing:

- **Timeouts and intervals stay per-call.** The six sites span 5 s to 30 s; a
  single shared default would be a flake generator.
- **The first probe runs _outside_ the retry.** `toPass` retries on _any_ throw,
  not just assertion failures — so a misconfigured run (`capturePathViaTool`
  throws when `JAUNDER_CAPTURE_DIR` is unset, `capture.ts:7-10`) would degrade
  from an instant stack trace to a 30 s timeout. Probing once before entering
  `toPass` keeps that failure fast and loud, and confines the degradation to
  mid-run corruption.
- **`visibility.spec.ts:313` is the one site that does not throw** — it `break`s
  and lets `expect(body, "feed contains the Public post").toContain(...)`
  produce the diff. Its assertion and message are preserved at the call site;
  only the waiting moves into `pollUntil`.

`toPass` also surfaces retries as steps in the Playwright report — but only for
the two sites whose probe is a Playwright call (`page.request.get`). The three
file-polling sites read with plain Node `fs`, so no per-attempt step appears for
them; they gain OTel attribution only.

**A static check was considered and declined** for polling: the factored
primitive is self-enforcing, since there is one place a capture-wait can be
written. (This reasoning does **not** extend to AC-11 — see there.)

### D5 — `withTimedAction` accepts a nullable page

`mail.ts` and `websub.ts` poll files with no page in scope. Threading a `Page`
through purely to satisfy the tracer would be churn _and_ a lie — an out-of-band
file poll has no page. `ActionRecord.pageUrl` is already optional
(`actions.ts:19`), so the signature relaxes to `Page | null`; the unconditional
`page.url()` calls at `actions.ts:48` and `:62` are guarded.

### D6 — Mount decomposition: marks for boot, derivation for post-mount

`data-mounted` is set the instant `mount_to_body` returns
(`csr/src/lib.rs:49-54`), so `commit_to_mount` **ends before the mount-path
fetches resolve**. #788's lever 4 sizes mount cost as including those fetches;
they are not inside the number it is sized from. Decomposing `commit_to_mount`
alone would close gap 2 while leaving the mount-cost issue half-blind.

Two mechanisms, because the two halves have different natures:

- **Boot phases → `performance.mark` from Rust**, emitted via a new
  `client::perf::mark` (beside `client::dom`, the crate that already carries
  `web-sys`) and called from `csr/src/lib.rs`. These are internal to wasm
  execution and invisible from outside.
- **Pre-entry (fetch / compile / instantiate) → derived**, not marked. It is not
  reachable from Rust, which only runs _after_ instantiation. It is derived from
  the navigation's `committedMs`, the `.wasm` `PerformanceResourceTiming` entry,
  and the first boot mark.
- **Post-mount settle → derived harness-side.** The mount-path fetches are
  **per-route and partly serialized** — `cockpit/component.rs:36` is
  `resolve_initial_page(session.reconcile.await, || list_home_feed(...))`, so
  the timeline explicitly awaits the session reconcile, and the timeline is
  route-level (`CockpitPage`) while only `BackupBanner` / `SiteBaseUrlBanner`
  are shell-level (`app/component.rs:57-58`). There is no single settled point
  on any route, and creating one would couple session, banners and timeline in
  production purely for measurement. The fixture already records every request's
  start/end and each navigation's `mountedMs`, so settle is derivable with no
  app change.

**"Mount-path request" is defined mechanically** (AC-7 depends on it): a request
whose start is at or after the navigation's `committedMs`, which finishes after
`mountedMs`, and which starts **before** the earlier of — the first timed action
recorded after `mountedMs`, or the next navigation's `startedMs`. That bounds it
to the app's own post-mount fetches and excludes anything a test action
provoked.

**Marks ship unconditionally, not behind a cargo feature.** They are a handful
of microsecond-scale calls; feature-gating them means the binary being measured
is not the binary being shipped.

**Marks are read back by prefix (`jaunder.*`), never by name**, so adding a mark
in Rust needs no TypeScript change and no cross-language literal can drift the
way `MOUNTED_ATTR` can. Accepted risk: nothing reserves the prefix, so a future
dependency emitting under it would be exported as a boot phase. Judged remote,
and visible in the analyzer's phase breakdown if it ever happens.

### D7 — Truncation is marked, not merely raised

Raising a cap only moves the cliff, and OTLP attribute size limits are real.
Five lists truncate in `fixtures.ts`, not three:

| List                 | Site   | Cap          | Already derivable?                                                        |
| -------------------- | ------ | ------------ | ------------------------------------------------------------------------- |
| `topSlowRequests`    | `:717` | 20           | yes — `e2e.request_count` (`:771`)                                        |
| `topActions`         | `:720` | 30           | yes — `e2e.action_count` (`:793`)                                         |
| `topNavigations`     | `:760` | 20           | yes — `e2e.navigation_count` (`:795`)                                     |
| `resources.topSlow`  | `:682` | 20           | **no**                                                                    |
| `__jaunderLongTasks` | `:673` | `slice(-20)` | **no** — and it is a _tail_ slice, so the earliest long tasks are dropped |

All five get a dropped count. The last two are the genuinely silent ones.

### D8 — Factoring reaches the subject specs

Every repeated multi-step sequence is factored into an instrumented helper,
**including** sequences a spec exists to test (`email.spec.ts`'s verification
flow, `password_reset.spec.ts`'s request flow). Those specs call the helper and
keep their own assertions.

The flow-coverage risk is bounded and checkable: the #681 gate attributes
server-fn hits by **traceparent**, not by where the driving code lives, so
moving steps into a helper cannot change which fns are driven or which test they
are attributed to. AC-14 asserts that byte-identically rather than assuming it.

## Acceptance criteria

### Gap 1 — fixture-phase visibility

- **AC-1**: An `e2e.test.lifecycle` span is exported per test, starting at the
  `_autoTestTimeout` stamp (D2a) and ending at or after the `e2e.test` span's
  end.
- **AC-2**: `e2e.context_mint` and `e2e.teardown` child spans are exported for
  every test, each with **non-zero** duration; `e2e.context_mint` starts at the
  `_autoTestTimeout` stamp and ends no later than `_autoPerfSpan`'s entry.
  `e2e.warmup` is exported exactly when `JAUNDER_E2E_WARMUP` is truthy.
- **AC-3**: On an identical run, `e2e.test` is unchanged in **span id**,
  **start/end range**, **attribute keys**, and the **values** of
  `e2e.request_count`, `e2e.navigation_count` and `e2e.action_count`. (Parent is
  deliberately excluded — D2.) Verified by comparing captures from `main` and
  the branch.
- **AC-4** — ✅ **met**. `cargo xtask traces analyze` gained a per-test
  **span-coverage** section reporting, per test: Playwright-reported duration
  (joined from `playwright-report-<backend>.json` on test + project + retry),
  lifecycle-tree covered time (interval **union**), and the uncovered remainder.
  Measured on `sqlite × chromium`, 127 tests: **28–31 % → 3.9 %** aggregate.
  Residual mean 176 ms/test (p50 171, max 373); correlation with test duration
  **0.21**, i.e. a fixed floor rather than a proportional cost — which is the
  evidence for D3's claim. Recorded in `docs/observability.md`. **No threshold
  was pre-committed**: the floor was measured after the mechanism existed.

### Gap 2 — `commit_to_mount` decomposition

- **AC-5**: `csr` emits `performance.mark`s under the `jaunder.` prefix at the
  boot boundaries: wasm entry (`main` start), seed parsed, render start, mount
  done.
- **AC-6**: The harness exports every `jaunder.*` mark by prefix discovery.
  Demonstrated by adding a mark **in Rust** whose name appears nowhere in
  TypeScript and observing it exported — injection via `page.evaluate` does not
  demonstrate the claim.
- **AC-7**: Each navigation record carries `mount_to_settled_ms`, computed by
  D6's mechanical rule; `null` when no qualifying request exists. Pinned by a
  `parse.rs`/`analyze.rs` fixture case including the exclusion boundary.
- **AC-8**: Each navigation record carries the pre-entry breakdown derived in D6
  (commit → wasm fetch start, wasm fetch duration, fetch end → wasm entry mark).
- **AC-9**: The marks are present in a **release** wasm build, verified by
  grepping the built `pkg/jaunder.wasm` (or its JS glue) for the `jaunder.`
  prefix.

### Gap 3 — composite flows

- **AC-10**: `login`, `subscribeTo`, `unsubscribeFrom`, `followEmailLink`,
  `composePost` and `fillLoginForm` each emit a `flow.*` timed action, matching
  the existing `flow.register` convention.
- **AC-11**: These four named sequences exist as exactly one instrumented helper
  each, with **zero remaining inline copies**, verified by `rg` for the
  sequence's marker call: (a) set-email + verify (`fixtures.ts` `verifiedUser`,
  `email.spec.ts`); (b) forgot-password request (`password_reset.spec.ts`); (c)
  feed-poll-for-marker (`feeds.spec.ts:33`, `visibility.spec.ts:313`); (d)
  register-then-login round trip. A closed enumeration, not a judgment call — no
  static check, but mechanically re-checkable.

### Gap 4 — extra-page capture

- **AC-12**: Page instrumentation exists as **one**
  `attachTraceCapture(context, …)` called from exactly two sites:
  `_autoPerfSpan` and `tracedContext`. No page-level instrumentation is
  duplicated.
- **AC-13** — ✅ **met**. "Private post: hidden from anonymous and
  non-subscriber, visible to author" exports **4** `e2e.page` spans, and the sum
  of `navigation_count` over its lifecycle tree is **8** (3 on the default
  page + 5 across the extra contexts). #788 measured `navigation_count` = 3 for
  this test — the default page alone — so the under-reporting gap is closed and
  the figure is now a committed literal.

  Deliberately not asserted equal to the test's `page.goto` count: a `goto` is
  not 1:1 with a recorded navigation (client-side `pushState` routing produces
  navigations with no `goto`, and an aborted or same-document `goto` may not
  commit). Suite-wide the run emitted 27 `e2e.page` spans.

- **AC-14** — ✅ **met**. `cargo xtask e2e sqlite chromium` reported
  `server-fn-coverage-verify — 54 covered; snapshot current`: byte-identical
  snapshot, orphan reason set unchanged, with `e2e.test` reparented under the
  envelope. D2's "reparenting cannot break #681" argument holds empirically.

### Gap 5 — truncation marker

- **AC-15**: All five truncated lists in D7's table carry a dropped-count
  attribute, non-zero exactly when entries were dropped — including
  `resource_summary_json`'s `topSlow` and `long_tasks_json`, whose losses are
  currently unrecoverable from any other attribute.

### Gap 6 — un-wrapped waits

- **AC-16**: One `pollUntil` exists in `end2end/tests/polling.ts`; all **six**
  sites in D4 are rewritten on it, and no `deadline`/`while`/`sleep` poll loop
  remains under `end2end/tests/`.
- **AC-17**: Every `pollUntil` call emits a timed action under its own name
  (`wait.mail`, `wait.websub_ping`, `wait.feed`), so the WebSub test's poll is
  attributed in the trace. Timeouts and intervals remain per-call.
- **AC-18**: `feeds.spec.ts:267`'s `await page.waitForTimeout(2_000)` — the
  suite's only `waitForTimeout`, and the other half of gap 6 — is wrapped in a
  timed action (`wait.settle`) so it is attributed. #794 makes it **visible**;
  #793 removes it.
- **AC-19**: The first probe runs outside the retry (D4), so a run with
  `JAUNDER_CAPTURE_DIR` unset fails immediately rather than after the poll
  timeout. Covered by a test asserting the fast failure.
- **AC-20**: `visibility.spec.ts`'s feed assertion message
  (`"feed contains the Public post"`) survives the rewrite.

### Gaps 7 and 8 — documentation

- **AC-21**: `docs/observability.md` states that Firefox reports zero long-tasks
  because Gecko implements no `longtask` PerformanceObserver — an engine
  limitation, not a capture bug.
- **AC-22**: `docs/observability.md` states the actual trace shape: actions,
  navigations and resources are **per-test JSON attributes** on `e2e.test`,
  while `request`, `storage.*`, `crypto.*`, `e2e.test` and the new lifecycle
  spans are **spans**. The "`action.timed` ×1233 spans" phrasing from #788 is
  corrected.
- **AC-23**: `docs/observability.md` documents the D2 hierarchy, the D3 residual
  with its measured size, and the D6 mark contract (prefix discovery,
  unconditional).

## Verification

Two altitudes, because #794 changes only observability — nothing here makes a
failing test pass, so a green gate proves nothing on its own.

1. **Shape, cheaply and permanently.**
   `xtask/src/traces/testdata/otel-traces-sample.jsonl` is extended with the new
   span shapes (envelope, phase children, `e2e.page`, mount marks, the AC-7
   boundary case). `parse.rs`/`analyze.rs` tests prove the analyzer consumes
   them and that attributes are named what `docs/observability.md` claims. This
   runs in every gate.
2. **Numbers, once, end-to-end.** One `cargo xtask traces run` before and after,
   read through AC-4's new span-coverage section. Shipping that computation
   rather than doing it ad hoc (as #788 did) is what makes the number
   re-checkable later.

Plus AC-14's coverage-snapshot check, the guard against this cycle disturbing
#681.

## Out of scope

- Acting on what the new instrumentation reveals. Deleting or amortizing the
  warmup is #792; removing the WebSub settle-sleep is #793 (this cycle only
  makes it visible, AC-18); seeding users via API is #791.
- Cutting mount cost itself (#788 lever 4). Gap 2 sizes it; it does not reduce
  it.
- A gate on residual invisible time. Considered as a companion to #789; not
  taken, to avoid a threshold-tuned flaky gate in the same cycle that
  establishes the measurement.
- Hardening `pollUntil` against the general class of non-transient throws inside
  the retry. AC-19 covers the known instance; a mid-run capture-file corruption
  would still surface as a timeout. Accepted.
