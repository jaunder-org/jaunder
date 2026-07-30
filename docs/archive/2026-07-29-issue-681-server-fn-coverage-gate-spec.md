# Issue #681 — trace-derived `#[server]` fn flow-coverage gate

**Status:** spec, awaiting approval (revised after cold soundness review)
**Issue:** [#681](https://github.com/jaunder-org/jaunder/issues/681) **Blocks:**
[#601](https://github.com/jaunder-org/jaunder/issues/601) (flow docs)

## Problem

`web/src` has **55** `#[server]` fns. The Playwright suite has ~111 tests across
**19** spec files. Nothing connects the two.

The gap is specifically **browser-driven flow coverage**, not testedness. Most
server fns do have server-side integration coverage
(`server/tests/web/web_media.rs`, `web_backup.rs`, `web_sessions.rs`,
`web_posts.rs`), and those bodies _are_ host-measured — ADR-0050's exemption
covers `#[component]` bodies and CSR UI, not `#[server]` fn bodies. So the
honest statement of the problem is narrower and more useful than "untested
code": **no one knows which server entry points a real browser session actually
drives.** An endpoint can be reachable only from code no UI path exercises, or
its only e2e test can be deleted, with no signal either way.

That matters because #601 wants to document user flows and anchor each to the
e2e spec that pins it. Without evidence, those anchors are unfalsifiable prose.

## Decisions

Each was resolved in the design interview; rationale recorded so it is not
re-litigated.

### D1 — Coverage is derived empirically from traces, never asserted in prose

A doc saying "`media.spec.ts` covers `delete_media`" is an assertion a gate can
only check for _well-formedness_ (both names resolve), not truth. The suite
already exports OTLP traces (`capture/otel-traces.jsonl`, #332) and
`xtask/src/traces/` already parses them through a pure `analyze_spans` seam
(ADR-0028). Coverage is computed from recorded evidence of real requests.

### D2 — The derived span name is the primary signal; `uri` covers what is not yet instrumented

**The span name is the better identity and is the primary signal**, because once
derived it is mechanically tied to the fn ident — no URL parsing, no `/api/`
prefix assumption, no endpoint indirection, and immune to any future path
change. What made it unusable was never the signal; it was that the names are
currently retyped by hand:

| span name                    | actual fn                |
| ---------------------------- | ------------------------ |
| `web.site.get_identity`      | `get_site_identity`      |
| `web.site.update_identity`   | `update_site_identity`   |
| `web.backup.warning_visible` | `backup_warning_visible` |
| `web.backup.get_settings`    | `get_backup_settings`    |
| `web.backup.update_settings` | `update_backup_settings` |

Five of the eleven existing names disagree with their fn, and
`web.auth.require_auth` (`web/src/auth/server.rs:112`) is on a fn that is not a
`#[server]` fn at all.

**So this cycle derives them.** The eleven `name = "…"` arguments are deleted,
making `#[tracing::instrument]` use the fn ident. That is a small, safe change —
verified that nothing reads the old names: no reference in
`docs/observability.md`, none in `xtask/src/traces/`, and the only hits are
archived plan documents. Deriving is what makes the primary signal correct _by
construction_ rather than by a convention a gate has to police.

> **Correction, found during implementation.** "The derived name _is_ the fn
> ident" is **false**, and this spec asserted it in several places. `#[server]`
> relocates the annotated body — carrying its `#[tracing::instrument]` — into a
> generated fn named `__server_<ident>` (`server_fn_macro`'s `to_dummy_ident`),
> so the emitted span is `__server_login`, not `login`. Measured against a real
> 21,372-span capture: **zero** span names equal an inventory ident; exactly
> **eleven** are `__server_<ident>`.
>
> The consequence was worse than a wrong sentence. The extractor compared the
> bare ident, so the primary signal matched **nothing** — and it failed
> _silently_, because `uri` went on carrying every hit and the totals still
> looked plausible. The union was a union of one, and the resilience this
> decision is built on (the span-name signal surviving #698 dropping explicit
> endpoints) was not actually attached. Worse, the hand-authored test fixture
> fabricated bare-ident spans, so the signal's unit tests pinned a shape that
> never occurs.
>
> Fixed by requiring the `__server_` prefix — required rather than merely
> tolerated, since the bare form occurs nowhere, so a match can only have come
> from a server fn's generated body. `code.namespace` needed no change: it holds
> the plain declaring module (`web::auth::api` for `session`), verified across
> all eleven. AC2's test now measures each signal **in isolation** so neither
> can go dead unnoticed again.

To identify a span as a server fn, both must hold: the span's name is
`__server_<ident>` for an ident in the syn-derived inventory, **and** its module
matches that fn's declared module. That module check is what stops a same-named
non-`#[server]` fn elsewhere in `web` from registering as a hit.

**The module attribute is `code.namespace`, not `target`.**
`tracing-opentelemetry` attaches the module path to a _span_ as `code.namespace`
(`layer.rs:905`, gated on `location`, default on); `target` is attached only to
_events_ (`layer.rs:1044`). Matching on `target` would find nothing on any span
— and would fail **silently**, leaving `uri` to carry all 55 fns while the
snapshot still looked plausible. Its value is crate-prefixed (`web::site::api`),
so the comparison against the enumerator's module must account for the `web::`
prefix.

**`uri` is the complementary signal, not a fallback.** Only 11 of 55 fns are
instrumented today; #511 owns instrumenting the rest. Until it lands, `uri` is
what covers the other 44, and afterwards it costs nothing and independently
corroborates. **Coverage is the union of the two signals** — a fn is covered if
either fires. (A fn that is instrumented yet shows a `uri` hit with no span hit,
or vice versa, is a genuine anomaly worth surfacing, but it is reported, not
failed, since either signal alone is sound evidence of a real request.)

Extraction details: strip the query string before matching (Leptos GET server
fns encode args there, so `uri` is `/api/get_post?id=…`; routes mount at
`/api/{*fn_name}`, `server/src/lib.rs:65`). URIs outside `/api/` are ignored.

**Match through the declared endpoint, not through the naming convention.** The
syn enumerator records each fn's _declared_ `endpoint`, and the extractor maps
`uri → fn` using that recorded value — it must never assume
`uri == "/api/" + fn_name`. Today those coincide for all 55, but the coincidence
is not load-bearing: it keeps the gate correct if any endpoint is ever renamed,
and it keeps working if the repo later drops the explicit `endpoint =`
attributes in favour of `DISABLE_SERVER_FN_HASH`-derived paths
(`server_fn_macro/src/lib.rs:510,527-544` — without that flag, an omitted
`endpoint` yields `/api/<fn_name><xxh64-hash>`, which exact-name matching would
miss entirely).

The **inventory** always comes from the syn enumerator, never from traces.

### D3 — Attribution is structural, via a per-test traceparent

`playwright.config.ts:64` sets one run-wide `traceparent` for every request, so
trace id does not distinguish tests; `workers` defaults to 2 with
`fullyParallel: workers > 1`, so time-window correlation would be ambiguous.
Instead, propagate the per-test `e2e.test` span id as the traceparent
parent-span-id, so the server's request span carries
`parentSpanId == <that test's span id>` — `make_request_span`
(`server/src/observability.rs:491`) adopts an inbound W3C context as parent.

Mechanism, stated because none of it is inferable:

- `extraHTTPHeaders` is a static `use` option and `JAUNDER_E2E_TRACEPARENT` is
  per-worker-process, so neither can carry a per-test value. Propagation is
  `context.setExtraHTTPHeaders()` called per test.
- `otel.ts`'s `buildSpan` mints `spanId` unconditionally at export time
  (`otel.ts:140`). It gains a caller-supplied span-id override so the id exists
  at test start.
- `browser.newContext()` does **not** inherit config-level `extraHTTPHeaders`,
  so the throwaway contexts (`fixtures.ts:283,333`) carry no traceparent at all
  today. Every context the fixtures create routes through one helper that
  applies it.
- `perf.ts` emits its own spans reading the same env (`perf.ts:52-58`); it is
  updated to use the same per-test id so its spans do not contradict the join.

`xtask/src/traces/parse.rs` deliberately drops `spanId`/`parentSpanId` ("no
report reads them", `parse.rs:18`). `Span` gains `span_id` and `parent_span_id`;
existing analyzer sections are unaffected.

**Attribution is an ancestor walk, not a direct parent check.** A
`#[tracing::instrument]` span is a _child of the request span_, so its parent is
the request span, not the test span — only the request span carries the test's
span id as its parent. The extractor therefore builds a
`span_id → parent_span_id` map and walks upward from a hit span until it reaches
a span whose parent is a known `e2e.test` span id. One walk serves both signals:
`uri` hits resolve in one hop, span-name hits in two. Anything whose walk
terminates without reaching a test lands in the orphan bucket (AC4).

### D4 — A committed snapshot, checked in both CI lanes

Traces exist only in the e2e lane, but fast feedback lives in
`validate --no-e2e`. The coverage result is committed: the static lane checks
the inventory against it cheaply, and the e2e lane regenerates it and fails on
drift. Committing it makes a coverage regression a reviewable line in the PR
diff rather than only a failing job.

### D5 — Hard gate with an explicit, evidence-seeded allowlist

A server fn may be uncovered only via an allowlist entry carrying a reason and a
filed issue. Per D-problem framing, **"already covered by server integration
tests, no browser flow" is an acceptable reason** — the allowlist records
absence of a _flow_, not absence of testing.

The ratchet must not loosen: an allowlist entry for a fn the snapshot shows as
covered is itself a failure, so entries cannot become write-only.

### D6 — `sqlite × chromium` is the authoritative combo

Each combo runs on its own runner and `e2e-gate` downloads no artifacts, so
union aggregation would require CI restructuring. Verified sound:
`flake.nix:657` runs `--project ${browser} --project ${browser}-admin`;
`chromium`'s `testIgnore` and `chromium-admin`'s `testMatch` are exact
complements covering all 19 spec files; and no test in `end2end/tests/` is
browser- or backend-conditional. Choosing one combo drops nothing.

### D7 — The `#[server]` enumerator becomes a shared seam

`server_fn_registrar_check.rs` (#426) already has a mature syn enumerator. It is
extracted once and reused rather than hand-rolled a third time (#511 proposes
another consumer).

### D8 — Regeneration is a per-combo operation only

`checks.e2e` is a `symlinkJoin` over every `e2e-*` check (`flake.nix:1023-1028`)
and both sqlite combos emit a file literally named `capture-sqlite.tar.gz`, so
in the joined output the sqlite-chromium and sqlite-firefox captures collide
unpredictably. Regeneration and the drift check therefore run **only** on the
per-combo `cargo xtask e2e sqlite chromium` path. Local aggregate
`cargo xtask validate` skips them; the static-lane check still runs there.

## Artifacts

Two **separate** committed files — the generator rewrites the snapshot on every
run, so the hand-maintained allowlist must not live inside it.

| File                                      | Owner        | Contents                                                          |
| ----------------------------------------- | ------------ | ----------------------------------------------------------------- |
| `docs/coverage/server-fns.json`           | generated    | server fn → the named tests that exercised it; the orphan bucket  |
| `docs/coverage/server-fns-allowlist.json` | hand-written | one entry per knowingly-uncovered fn: fn name, reason, issue link |

**Location rationale.** `end2end/` is `src = ./end2end` **unfiltered**
(`flake.nix:546`), so committing them there would change the e2e npm package's
source hash and force a cold rebuild of all four e2e VM checks on every
coverage-affecting commit. `docs/` is excluded by the app derivation's allowlist
filter (`flake.nix:278-297`). Both files are JSON because crane's cargo-source
filter can match `.toml`.

**No provenance fields.** The snapshot records coverage data only — no commit,
no timestamp. A recorded commit is necessarily an ancestor of the commit under
test, so with fail-on-any-difference it would make the gate permanently red.
Keys are stably sorted and pretty-printing is stable, so an unchanged run is
byte-identical and drift comparison is total.

### Developer workflow this implies

Adding a new `#[server]` fn reddens `validate --no-e2e` until the author either
adds an e2e test and regenerates, or adds an allowlist entry with a reason and a
filed issue. That friction is the intended ratchet.

Regeneration is `cargo xtask server-fn-coverage regenerate`, reading the capture
`cargo xtask e2e sqlite chromium` lifts to
`.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`
(`xtask/src/steps/nix.rs:94-101,138-145` already copies it out on passing runs,
so this needs no new plumbing). Whether the host-runner `cargo xtask e2e-local`
path can also produce a usable capture is determined in the plan's first task
and documented in the failure message either way.

## Acceptance criteria

1. **AC1 — shared enumerator.** The syn `#[server]` enumerator lives in a module
   consumed by both `server_fn_registrar_check` and the new gate. The registrar
   guard's existing test **assertion bodies are unchanged** (relocation and
   `use`-line edits permitted), showing no behavior change.
2. **AC2 — extractor is unit-tested against a real capture, on both signals.**
   The capture used to seed the allowlist is committed as the test fixture.
   Tests cover: the span-name signal resolving to the right fn; the `uri` signal
   including query-string stripping (`/api/get_post?id=…` → `get_post`); that
   the union of the two is taken; that non-`/api/` traffic (static assets,
   feeds) does not appear; and that a span whose name matches an inventory fn
   but whose `target` is a different module is **not** counted.
3. **AC3 — span names are derived, and cannot regress.** The eleven
   `#[tracing::instrument(name = "…")]` arguments in `web/src` are removed so
   the span name is derived from the fn — which, per the correction under D2, is
   `__server_<ident>` rather than the bare ident, because `#[server]` relocates
   the body into a generated fn. A check fails if any `#[server]` fn in
   `web/src` carries an explicit `name =` on its instrument attribute, so the
   second source of truth cannot be reintroduced. Existing behaviour is
   otherwise untouched: no `skip(...)` argument is altered. (Twelve arguments
   were in fact removed: the twelfth, `web.auth.require_auth`, sits on a fn that
   is not a `#[server]` fn — noted under D2.)
4. **AC4 — inventory drift fails loudly.** A server fn whose `endpoint` does not
   match its fn name fails the gate with a message naming it. A **bare
   `#[server]`** with no `endpoint` (which the shared enumerator accepts,
   `server_fn_registrar_check.rs:109`) counts as drift and fails, since its
   generated endpoint is not the fn name.
5. **AC5 — per-test attribution, orphans enumerated.** After a real
   `sqlite × chromium` run the snapshot maps each covered fn to the named
   test(s) that exercised it, and reports an **orphan bucket** of hits
   attributable to no test. The bucket contains only traffic occurring outside
   any test (global setup/seeding), each entry enumerated.
6. **AC6 — the two non-fixture specs are converted.** `atompub.spec.ts` and
   `feeds.spec.ts` import `test` from `@playwright/test`, so they emit no
   `e2e.test` span and their traffic — including `feeds.spec.ts:263`'s direct
   `POST /api/update_post` — is orphaned by construction. Both are moved onto
   the fixtures' `test`, and AC5's orphan bucket is what demonstrates it.
7. **AC7 — snapshot committed and regenerated.** The `sqlite × chromium` e2e job
   regenerates the snapshot and fails on any difference from the committed copy.
8. **AC8 — static lane gate.** `cargo xtask validate --no-e2e` fails when a fn
   in the syn-derived inventory is absent from both snapshot and allowlist —
   i.e. a newly added `#[server]` fn reddens the fast lane without an e2e run.
9. **AC9 — allowlist has teeth.** An entry lacking a reason or issue link is
   rejected. A fn neither covered nor allowlisted fails the build.
10. **AC10 — the ratchet cannot loosen.** An allowlist entry for a fn the
    snapshot shows as **covered** fails the gate, so stale entries must be
    removed rather than accumulating.
11. **AC11 — allowlist seeded from evidence.** Every entry at merge names a fn
    absent from the hit-set of the committed AC2 capture. (Committing that
    capture is what makes this checkable rather than a claim about intent.)
12. **AC12 — the gate is proven to bite, in-repo.** A checked-in unit test feeds
    the gate a synthetic inventory containing an uncovered, unallowlisted fn and
    asserts it fails. (Prose in a PR describing a temporary fn that does not
    ship is not evidence.)
13. **AC13 — no e2e regression.** The full `sqlite × chromium` combo passes with
    the fixture, spec, and span-name changes in place, showing they did not
    destabilize the suite.
14. **AC14 — actionable failure.** The failure message names the offending fn
    and both remedies (add an e2e test and regenerate, with the exact command;
    or add an allowlist entry with reason and issue).
15. **AC15 — a broken capture fails closed.** A missing, empty, or unparseable
    `otel-traces.jsonl` fails with a message saying so, and is never treated as
    "no uncovered fns".
16. **AC16 — regeneration does not invalidate the e2e derivations.** Changing
    either artifact leaves the four e2e VM checks' input hashes unchanged (they
    live outside every derivation source).

## Out of scope

- **Flow documentation** (`docs/flows/`, Mermaid diagrams) — #601, which
  consumes this snapshot as its source of truth.
- **Closing the coverage gaps.** Writing the missing e2e tests is filed per-gap;
  this cycle produces the evidence and the ratchet. (Converting the two
  non-fixture specs under AC6 is attribution plumbing, not new coverage.)
- **Route coverage** — #601's, and advisory there.
- **Union aggregation across all four combos** (D6).
- **Instrumenting the other 44 server fns** — #511. This cycle _derives_ the
  eleven existing span names (AC3) so the primary signal is sound, but adds no
  new `#[tracing::instrument]` attributes and changes no `skip(...)` arguments.
  Until #511 lands, `uri` carries the uninstrumented fns.
- **Deriving server-fn paths** via `DISABLE_SERVER_FN_HASH` — #698. The gate is
  correct either way, since it matches through each fn's declared endpoint.

## Risks

- **R1 — shared-fixture blast radius.** The traceparent change touches
  `fixtures.ts`, which most tests run through, plus two spec files. Mitigated by
  AC13 and by landing it as its own commit.
- **R2 — capture availability. Resolved, not open.** `flake.nix:667-691` copies
  diagnostics **unconditionally** before the `assert pw_status == 0` at `:696`,
  and `xtask/src/steps/nix.rs:94-101` lifts `capture-<backend>.tar.gz` into
  `.xtask/diagnostics/`. The capture is available on passing runs.
- **R3 — snapshot churn.** Test-title edits produce diffs. Accepted: that is the
  signal working. Per AC16 it costs no _e2e VM_ rebuild — those four checks'
  source filter is an allowlist that excludes `docs/*.json`. The `static-checks`
  derivation's filter (`flake.nix:1134-1139`) is exclusion-only, so its cache
  does bust on every regeneration; that is a cheap `runCommand`, and it is the
  reason AC16 is scoped to the e2e checks rather than to "no derivation at all".
- **R4 — `chromium-admin` depends on `chromium`.** If `chromium` fails,
  `chromium-admin` is skipped and the run yields partial coverage. Consistent
  with regenerating only on passing runs, but the reason that ordering must not
  be relaxed.
