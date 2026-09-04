# ADR-0081: Server-fn flow coverage is derived from e2e traces, not asserted

- Status: accepted
- Date: 2026-07-29
- Issue: [#681](https://github.com/jaunder-org/jaunder/issues/681)

## Context

ADR-0050 exempts CSR web code from line coverage: the reactive shells are not
host-instrumentable, so counting lines there measures nothing. That exemption is
correct, but it left a different hole. `web/src` has 55 `#[server]` fns and the
Playwright suite has ~111 tests across 19 spec files, with no link between them.

The hole is **browser-driven flow coverage**, not testedness — a distinction
worth stating precisely, because the loose version invites the wrong remedy.
Most server fns do carry server-side integration coverage, and `#[server]` fn
bodies are host-measured; ADR-0050 exempts `#[component]` bodies and CSR UI, not
these. What nobody knows is which server entry points a real browser session
actually drives: an endpoint can be reachable only from code no UI path
exercises, or its sole e2e test can be deleted, with no signal either way.

The obvious cheap fix is documentary: have each flow doc declare "pinned by
`<spec>.spec.ts`" and have a gate check the names resolve. That gate can only
verify _well-formedness_ — that both names exist. It cannot verify the claim is
**true**, so a doc naming a spec that never touches the fn stays green forever.
A coverage convention whose central claim is unfalsifiable is worse than none,
because it reads as assurance.

The evidence to do better already exists and was simply never read. The e2e VM
run exports OTLP traces per combo (`capture/otel-traces.jsonl`, #332);
`xtask/src/traces/` already parses and analyses them through a pure
`analyze_spans` seam (ADR-0028); and the server's per-request span records `uri`
and adopts an inbound W3C `traceparent` as its parent (ADR-0011's propagation,
implemented in `make_request_span`).

## Decision

**Server-fn flow coverage is computed from recorded trace evidence of real
requests, and a coverage claim that cannot be derived from a trace is not
accepted.**

Concretely:

- The **inventory** of `#[server]` fns always comes from the syn enumerator
  shared with the registrar guard (#426) — never inferred from traces. If the
  endpoint/fn-name correspondence drifts, the gate fails loudly rather than
  mis-mapping silently.
- The **hit set** is extracted from a passing `sqlite × chromium` e2e run from
  two signals, unioned. The **primary** signal is the `#[tracing::instrument]`
  span name, because a _derived_ span name is mechanically tied to the fn ident
  — no URL parsing, no path convention, and immune to any future change in how
  endpoints are spelled. That required a prior fix: the eleven names already in
  `web/src` were retyped by hand and five had drifted (`web.site.get_identity`
  for `get_site_identity`), with one attached to a fn that is not a server fn.
  The remedy is **derivation, not compliance** — omitting `name` deletes the
  second source of truth outright, leaving nothing to drift and nothing for a
  gate to police, whereas re-aligning the names and guarding them would preserve
  the duplication that caused the drift. A span counts only when its name is in
  the inventory _and_ its module matches that fn's — read from `code.namespace`,
  which is where `tracing-opentelemetry` records a span's module (`target`
  exists only on events, so matching it would silently find nothing).
- **The span name is matched forward from the inventory, never inverted out of
  the name** — and the reason is the cautionary tale this ADR should be read
  for. There is no single span name shape. `server-fn-tracing` (#511, ADR-0011)
  _writes_ `web.<vertical>.<ident>`; omitting the explicit `name` instead
  _derives_ `__server_<ident>`, because `#[server]` relocates the annotated body
  — carrying its `#[tracing::instrument]` — into a generated fn of that name
  (`server_fn_macro`'s `to_dummy_ident`); and were it to stop relocating,
  derivation would yield the bare ident.

  The first implementation compared the bare ident and so matched **nothing**:
  measured against a real 21,372-span capture, zero span names equalled an
  inventory ident and exactly eleven were `__server_<ident>`. It failed
  **silently** — `uri` carried every hit, the totals looked plausible, and the
  hand-authored unit fixture fabricated bare-ident spans, so the tests agreed
  with the bug. Fixing it to require `__server_` would have repeated the mistake
  one regime later: #511 landed explicit names and that shape stopped occurring
  too.

  Two lessons, both structural. **A union whose members are never measured
  _separately_ is indistinguishable from a single signal** — so each signal is
  tested in isolation against real data, asserting each alone covers everything
  the union does. And **compute the candidates from the inventory rather than
  parsing them out of the observation**, so a regime change is a code update
  that a test catches, not a silent outage.

  > **Annotation (2026-08-12).** `server-fn-tracing` no longer authors the span
  > name: `#[macros::server]` derives it (#714, and ADR-0011's 2026-07-30
  > addendum). The gate survives as the recordable-type default-deny and its
  > siblings. The decision this bullet records — match the name forward from the
  > inventory, never invert it — is unchanged, and the cautionary tale is
  > stronger for it: authorship of the name moved again and the extractor needed
  > no edit, because it computes candidates rather than parsing them.

- **`code.namespace` disambiguates, not the name.** `web.<vertical>.<ident>`
  uses the module's _first_ segment, so `posts::api` and `posts::api::listing`
  both render `web.posts.…` — a lossy key. `(module, ident)` cannot collide,
  because Rust forbids two items of one name in one module. This is also why the
  derived form remains viable as a human-facing scheme: the OTLP span carries
  `code.namespace`, `code.filepath` and `code.lineno`, which locate a fn more
  precisely than grepping an ambiguous dotted literal.

  > **Annotation (2026-09-04, #948).** The preceding rationale records the
  > accepted state when this decision was made. It is stale for the current
  > explicit `web.<vertical>.<ident>` form: `#[macros::server]` now enforces
  > `web/src/<vertical>/api.rs` and rejects deeper server-function modules, so
  > that form is unique rather than a lossy key shared by `posts::api` and an
  > invalid `posts::api::listing` placement. `code.namespace` now defensively
  > corroborates the explicit name and rejects foreign or malformed evidence; it
  > remains the load-bearing disambiguator for retained `__server_<ident>` and
  > bare `<ident>` compatibility forms, which omit the vertical. See the current
  > [flow-coverage guidance](../observability.md#server-flow-coverage-681).

- The **complementary** signal is the request span's `uri`, resolved through
  each fn's declared endpoint recorded by the enumerator — never by assuming
  `uri == "/api/" + fn_name`. It covers the fns not yet instrumented (#511 owns
  the remaining 44) and afterwards corroborates at no cost.
- **Attribution to a specific test is structural, not heuristic.** The per-test
  `e2e.test` span id is propagated as the traceparent parent-span-id, so each
  server request span names the test that caused it. Time-window correlation is
  explicitly rejected: the suite runs `fullyParallel` and the windows overlap.
- **That propagation is itself gate-enforced, because relying on discipline
  failed.** A context from `browser.newContext()` does not inherit Playwright's
  config-level `extraHTTPHeaders`, so it sends the run-wide traceparent and its
  traffic is unattributable. Providing a helper and documenting "call it for
  every context" was not enough — 15 of 18 call sites never did, and the first
  seeding run consequently reported two fns as uncovered that tests demonstrably
  drove. Specs therefore obtain contexts from a `tracedContext` fixture that
  closes over the ids, and a static check rejects raw `newContext(` in
  `end2end/tests`. The failure mode this prevents is **silent under-reporting**:
  the suite still passes, the snapshot just quietly shrinks.
- The result is **committed**, checked cheaply in the static lane and
  regenerated (fail-on-drift) in the e2e lane, because traces exist only in the
  latter.
- **It is committed as two files, and only one of them is compared (#745).**
  `docs/coverage/server-fns.json` carries the covered `<vertical>::<ident>` set
  plus the orphan reason sets and is byte-compared;
  `docs/coverage/server-fns-evidence.json` carries the per-test titles,
  regenerated beside it and never compared. The static lane cross-checks that
  their key sets agree, in both directions.

  This was originally one file. Splitting it is not tidying — the single file
  **could not converge**, and the reason matters for anyone tempted to merge
  them back:
  - **Only the fn set was ever asserted.** The verdict has never read a test
    title; the titles were load-bearing for red/green solely because the whole
    file was byte-compared.
  - **The titles are not reproducible.** Across four forced re-executions of
    `checks.x86_64-linux.e2e-sqlite-chromium` on one tree, the covered key set
    (54) and the orphan reason sets were identical every time, while the title
    sets moved. Three of those runs are committed as testdata under
    `xtask/src/server_fn_coverage/testdata/determinism/`, with a test asserting
    they project to one byte-identical snapshot — so this claim is checkable in
    milliseconds rather than by re-running the e2e matrix.
  - **Nothing is misattributed.** Every hit is attributed to the test whose
    browser context actually issued the request. What varies is _post-assertion
    trailing traffic_: a test that ends mid-navigation leaves its page booting,
    and the boot is truncated at a different point each run. Do not go looking
    for a trace-propagation bug — there isn't one.
  - **A time-window rule does not fix it.** Refusing hits that begin after a
    test's span closed was implemented and measured: it removes the wide-margin
    cases and exposes narrow ones (a `tags::list` hit at +31 ms in one run and
    −90 ms in others), so it relocates the race rather than removing it. It is
    the same objection this ADR already raises against time-window correlation.

  The accepted cost: the evidence file's titles can go stale unnoticed, because
  only its key set is checked — a renamed or deleted test leaves a wrong title
  in a green tree. Whether that file is worth its weight at all is **#757**.

  > **Addendum (2026-08-24, #757).** The two-file compromise above was retired.
  > `docs/coverage/server-fns.json` became the sole durable generated artifact:
  > regeneration wrote only that deterministic snapshot, static verification
  > read it with the source-derived inventory, and e2e verification recomputed
  > and compared it from the authoritative capture. Structural per-test
  > attribution remained internal to extraction because it distinguished
  > test-driven requests from orphan traffic, but test titles were no longer
  > persisted. The accepted Decision and Consequences remain the dated
  > historical record; for current truth, read
  > [Server-fn gates](../ARCHITECTURE.md#server-fn-gates) and current repository
  > guidance rather than this historical Decision.

- A fn may be **uncovered only as a failing gate finding**. There is no
  server-fn flow-coverage allowlist; add a browser flow instead.

## Consequences

- Flow documentation (#601) can state coverage as a checked fact rather than a
  promise; its "pinned by" anchors become claims verified against this snapshot.
  #310's traceability matrix gains a substrate, so the two converge on one
  mapping instead of two. **Half of that survives the #745 split and half does
  not:** _whether_ a fn is covered remains a checked fact in the compared
  snapshot, but _which flow_ covers it now lives in the uncompared evidence file
  and is a promise again — accurate when regenerated, unpoliced thereafter.
- A new `#[server]` fn cannot land silently untested: absent from the snapshot,
  it reddens the fast lane without needing an e2e run.
- This commits us to keeping the e2e trace export working. If the capture
  pipeline breaks, coverage verification degrades with it — the gate must treat
  a missing or unreadable capture as failure, never as "nothing uncovered".
  "Unreadable" includes **stale**: the first implementation read its capture
  from a diagnostics directory whose copy step silently failed to overwrite the
  previous run's read-only files, which would have verified new builds against
  old traces. Freshness of the capture is part of the contract, not an
  incidental detail of how artifacts get lifted.
- It adds a standing obligation on **e2e authors**, not just on server-fn
  authors: a new browser context must be traced. That is why the constraint is a
  gate rather than a convention — the cost of forgetting is invisible.
- The artifacts are generated files under review: test-title edits produce diff
  churn. That is accepted as the signal working, and since #745 it is confined
  to the **uncompared** file — so it is a diff to read past, never a red build.
  (It previously read "confined to one file", which was true of the layout but
  missed the consequence that mattered: while that one file was compared,
  unrelated title churn could and did turn the gate red.)
- It does **not** commit us to union aggregation across the e2e matrix. One
  combo is authoritative because neither backend nor browser changes which
  server fns the UI invokes; if that assumption breaks it surfaces as snapshot
  drift, which is the moment to revisit.
- It rules out the hand-asserted "pinned by" model as sufficient on its own —
  that form may remain as prose for readers, but it carries no gate authority.
