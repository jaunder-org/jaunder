# #601 — flow documentation with checked references

Issue: [#601](https://github.com/jaunder-org/jaunder/issues/601). Milestone:
Test infrastructure & E2E.

## Problem

Jaunder’s user-visible CSR journeys are distributed across the router,
server-function API modules, and Playwright tests. The canonical
[`csr-e2e-matrix`](../../coverage/csr-e2e-matrix.md) maps a reviewed user
journey to browser evidence, but it does not show route transitions or request
sequences. Server-function telemetry separately proves an endpoint was reached
somewhere, but its per-test attribution is intentionally timing-dependent and
cannot establish a user journey.

Readers therefore cannot inspect the application’s route-level journey map, a
substantive journey’s browser/server/storage/mailer exchange, or the ownership
of a route/endpoint without reconstructing it from implementation files.

## Decisions

- **D1 — One flow document per stable CSR-matrix heading.** `docs/flows/` has an
  index plus 13 flow documents corresponding to the matrix’s stable headings:
  application shell and boot state; public reading; authenticated cockpit;
  authentication; profile and email verification; app-password management;
  audiences, subscriptions, and visibility; invitation registration;
  administration; post authoring lifecycle; media management; password reset;
  and tag browsing. The matrix remains the only flow→Playwright evidence map.
- **D2 — The index is the only route map.** `docs/flows/README.md` owns one
  Mermaid `graph TD` of all mounted CSR routes, grouped by anonymous reading,
  authenticated authoring, token-in-URL journeys, and administration. It
  hand-authors navigation and redirect edges; dotted edges identify only emailed
  invitation, verification, and reset links. Individual documents contain
  concise prose and sequence diagrams only where the journey is substantive;
  they do not duplicate the route graph.
- **D3 — Sequence diagrams name the user-facing exchange.** Mermaid
  `sequenceDiagram`s are required for invitation→registration, email
  verification, password reset, login/authed `/app` transition, post lifecycle,
  and audiences/subscriptions/visibility. They show only Browser/Page, server
  function, storage, and mailer participants that explain the journey; simple
  resource reads need not become arrows solely to enumerate endpoints.
- **D4 — Explicit typed reference tokens are the checked contract.** Flow docs
  declare source-backed references as backticked `route:<mounted-pattern>`,
  `endpoint:/api/<vertical>/<operation>`, and
  `matrix:docs/coverage/csr-e2e-matrix.md#<heading>` tokens. The gate parses
  these tokens in prose, tables, and Mermaid fences; it does not scan arbitrary
  slash-paths. `ParamSegment("username")` normalizes to `route:/:username`;
  `TildeUsername("username")` normalizes to `route:/~:username` in the permalink
  pattern. Other user-facing route prose remains unchecked.
- **D5 — Deterministic source inventories own completeness.** The gate derives
  mounted route patterns from `web/src/app/component.rs` and endpoints from the
  existing `#[macros::server]` inventory. Every endpoint is declared exactly
  once across the 13 flow documents. Invalid, duplicate, or unassigned endpoint
  tokens fail the check. Every declared route and endpoint must exist; invalid
  declared references fail. The `ParentRoute` shell is reportable as
  `route:<shell>`; the router fallback is excluded from unmapped reporting.
- **D6 — Telemetry is coverage state, never journey attribution.** For each
  declared endpoint, the gate reports whether the checked
  `docs/coverage/server-fns.json` snapshot covers it or its allowlist records a
  reason. It does not copy, validate, or infer flow ownership from
  `server-fns-evidence.json` test titles. This preserves ADR-0081’s distinction
  between deterministic covered keys and non-reproducible per-test attribution.
- **D7 — Matrix anchors are mechanically live.** Every flow document declares at
  least one `matrix:` token. The gate validates its target file and heading
  fragment, so an evidence link cannot silently rot as the existing Markdown
  link check currently ignores fragments.
- **D8 — Unmapped routes are visible but informational.** The check prints
  mounted child routes and the shell absent from `route:` declarations but does
  not fail for them initially. It does not report unmapped endpoints: endpoint
  assignment is fail-closed under D5.
- **D9 — Architecture links to the index, not a duplicate graph.** The CSR/web
  section of `docs/ARCHITECTURE.md` links the flow index. No Mermaid graph is
  embedded there.

No ADR is needed: this is a concrete documentation and xtask consistency gate
that composes existing router, server-function, matrix, and telemetry decisions.

## Acceptance criteria

- **AC1 — Flow corpus.** `docs/flows/README.md` and exactly 13 matrix-aligned
  flow documents exist. Each flow document links one or more relevant stable
  matrix headings through a checked `matrix:` token and does not reproduce
  `Pinned by` test lists or telemetry attribution.
- **AC2 — Route map.** The index’s Mermaid `graph TD` represents every mounted
  route pattern from the router, including `route:<shell>`, with the four named
  regions. It contains hand-authored navigation and redirect edges; dotted edges
  are reserved for emailed-link journeys. It excludes protocol-only surfaces and
  the fallback from journey claims.
- **AC3 — Required journey sequences.** Invitation registration, email
  verification, password reset, login/authed transition, post lifecycle, and
  audiences/subscriptions/visibility each contain an accurate Mermaid sequence
  diagram with only the relevant Browser/Page, server-function, storage, and
  mailer interactions.
- **AC4 — Endpoint assignment.** Every source-derived server-function endpoint
  appears exactly once as an `endpoint:` token in the flow corpus. The docs may
  list endpoints outside a sequence diagram; diagrams remain legible.
- **AC5 — Reference gate validity.** A new xtask step parses typed tokens from
  `docs/flows/`; fails on malformed/unknown routes or endpoints, duplicate or
  unassigned endpoints, missing/malformed matrix references, and unresolved
  matrix heading fragments; reports unmapped mounted routes without failing.
- **AC6 — Telemetry status.** The same step reports each declared endpoint as
  covered by the checked server-function snapshot or identified by its explicit
  allowlist reason. It never reads `server-fns-evidence.json` or presents a
  test-title as flow evidence; a focused test proves missing or altered evidence
  data cannot affect the report.
- **AC7 — Documentation integration.** `docs/ARCHITECTURE.md` links the flow
  index from the CSR/frontend section. The matrix intro replaces its temporary
  issue #601 link with `docs/flows/README.md` while retaining its canonical
  Playwright evidence mapping.
- **AC8 — Verification.** A named `flow-docs` xtask step is registered in both
  `cargo xtask check` and `cargo xtask validate`, and Architecture’s xtask table
  documents it. Focused xtask tests cover parser normalization (including
  `ParamSegment` and `TildeUsername`), malformed/unknown/duplicate/unassigned
  reference failures, matrix fragment validation, unmapped-route reporting,
  coverage/allowlist status, and deliberate missing/altered evidence data. The
  project’s full `cargo xtask validate` passes.
