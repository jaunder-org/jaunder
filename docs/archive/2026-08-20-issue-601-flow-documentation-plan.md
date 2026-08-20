# Flow Documentation Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to `jaunder-dispatch` when useful). Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish checked CSR journey documentation whose route, endpoint,
matrix, and telemetry references cannot silently drift.

**Architecture:** `docs/flows/README.md` owns the complete route graph; one
Markdown document per CSR matrix heading owns concise journey narrative,
endpoint declarations, and only substantive sequence diagrams. A host-only
`flow-docs` xtask step derives route/server-function facts from existing source
enumerators, parses explicit typed tokens from the flow corpus, and compares
coverage state only to deterministic snapshot/allowlist inputs.

**Tech Stack:** Markdown, Mermaid, Rust (`xtask`), `syn`, existing
`web_server_fns`, CSR E2E matrix, server-function coverage artifacts.

## Global Constraints

- Implement approved spec `2026-08-20-issue-601-flow-documentation-spec.md`;
  retain its D1–D9 and AC1–AC8 exactly.
- `docs/coverage/csr-e2e-matrix.md` alone maps flows to Playwright tests; flow
  docs link `matrix:` headings and never duplicate `Pinned by` or telemetry
  test-title evidence.
- Typed backticked declarations are the only checkable references:
  `route:<pattern>`, `endpoint:/api/<vertical>/<operation>`, and
  `matrix:docs/coverage/csr-e2e-matrix.md#<heading>`.
- Endpoint assignment is source-derived and fail-closed; route omissions report
  only. `server-fns-evidence.json` is forbidden input.
- No new ADR. Before each commit tick the relevant plan steps, run
  `devtool run -- cargo xtask check`, stage formatter changes, and commit
  without a `Co-Authored-By` trailer.

---

## Review Header

**Scope in:** 13 flow documents and index; CSR matrix/Architecture links;
`flow-docs` xtask parser, inventory comparison, tests, and normal gate wiring.

**Scope out:** generated diagrams, protocol surfaces, changes to Playwright
coverage, server-function behavior, and telemetry attribution.

**Tasks:**

1. Implement and unit-test the typed flow-reference inventory/check core.
2. Author the route-map index and first six substantive journey documents.
3. Author the seven remaining matrix-aligned flow documents and endpoint census.
4. Wire the guard into normal gates and project flow documentation links.

**Key risks:** route parsing must distinguish `ParamSegment` from
`TildeUsername`; all 55 endpoints need exactly one declaration without bloating
diagrams; matrix anchors require fragment checking that `doc-links` deliberately
omits; coverage status must remain independent of timing-dependent attribution.

## File Structure

| File                              | Responsibility                                                                                                 |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `xtask/src/steps/flow_docs.rs`    | Parse typed references, derive inventories, validate matrix fragments and endpoint assignment, render reports. |
| `xtask/src/lib.rs`                | Assemble `flow_docs`; run it in both `check` and `validate`.                                                   |
| `docs/flows/README.md`            | Index, Mermaid route map, typed route declarations.                                                            |
| `docs/flows/*.md`                 | 13 matrix-aligned journey documents, matrix and endpoint declarations, required sequences.                     |
| `docs/coverage/csr-e2e-matrix.md` | Replace temporary #601 consumer link with the flow index.                                                      |
| `docs/ARCHITECTURE.md`            | Link flow index and list the new xtask guard.                                                                  |

### Task 1: Build the flow-document guard

**Files:**

- Create: `xtask/src/steps/flow_docs.rs`
- Modify: `xtask/src/lib.rs` (the inline `mod steps` declaration)
- Test: `xtask/src/steps/flow_docs.rs`

**Interfaces:**

- Consumes: `crate::web_server_fns` for derived `(vertical, operation)` values;
  `web/src/app/component.rs` route declarations; `docs/coverage/server-fns.json`
  and `server-fns-allowlist.json` only.
- Produces: a test-only `flow_docs` module containing `run() -> StepResult`,
  parsed `FlowRefs { routes, endpoints, matrix_refs }`, and deterministic
  diagnostics; Task 4 promotes it to the production static-step surface.

- [x] **Step 1: Write failing parser and inventory tests**

  In temporary fixture trees, assert extraction accepts tokens from prose,
  tables, and Mermaid fences; ignores arbitrary paths; normalizes
  `ParamSegment("username")` to `/:username` and the full
  `TildeUsername("username"), year, month, day, slug` tuple to
  `/~:username/:year/:month/:day/:slug`; recognizes `<shell>`; and excludes the
  fallback. Add cases that fail for malformed/unknown typed tokens,
  duplicate/unassigned endpoints, a non-index flow document without `matrix:`,
  missing matrix files/headings, and an endpoint missing both snapshot and
  allowlist. Matrix fragments use the existing Markdown heading-slug form:
  lowercase, punctuation removed, word separators collapsed to `-`; include
  `Audiences, subscriptions, and visibility`. Add a fixture with
  absent/incorrect `server-fns-evidence.json` and assert identical report
  output.

- [x] **Step 2: Run RED**

  Run:
  `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -p xtask -E 'test(flow_docs::tests)'`

  Expected: FAIL because `flow_docs` does not exist.

- [x] **Step 3: Implement the deterministic guard**

  Parse only backticked typed tokens under `docs/flows/`. Add a test-only
  `flow_docs` module declaration to `lib.rs`’s inline `mod steps`: the guard
  cannot enter `check` before the corpus exists, and a private unused production
  step is rejected by the workspace lint policy. Derive endpoint paths as
  `/api/<vertical>/<operation>` from the shared server-fn enumerator; reject
  zero or multiple flow assignments. Derive mounted patterns from router syntax,
  including both username forms. Validate matrix fragments by the stated
  heading-slug algorithm. Return a failed `StepResult` for invalid declared
  references and endpoint assignment defects; include an informational sorted
  unmapped-route section and covered/allowlisted endpoint status. Do not open or
  name the evidence artifact. Add a separately named
  `repository_flow_corpus_is_valid` test, run only after Task 3, which invokes
  the same checker over repository docs.

- [x] **Step 4: Run GREEN**

  Run the Step 2 command. Expected: PASS.

- [x] **Step 5: Commit guard core**

  Tick this task, run `devtool run -- cargo xtask check`, stage changed xtask
  files, and commit `test(xtask): validate flow documentation references`.

### Task 2: Author the route map and substantive flows

**Files:**

- Create: `docs/flows/README.md`, `application-shell-and-boot-state.md`,
  `authentication.md`, `profile-email-verification.md`,
  `invitation-registration.md`, `post-authoring-lifecycle.md`,
  `audiences-subscriptions-visibility.md`, `password-reset.md`
- Test: Task 1 fixture promoted against these real docs once corpus is complete.

**Interfaces:**

- Produces: matrix tokens, route tokens, and unique endpoint tokens assigned to
  their owning user journey; required Mermaid diagrams.

- [x] **Step 1: Author README route map and declarations**

  Write index prose and a single `graph TD` covering every mounted route and
  `<shell>`, grouped into the four approved regions. Add hand-authored
  navigation and redirect arrows; make emailed invitation/verification/reset
  links dotted. Include typed route declarations matching source patterns,
  including both `/:username` and `/~:username/...`; do not treat
  fallback/protocol routes as journey nodes.

- [x] **Step 2: Author the seven substantive documents**

  Write concise docs for shell/boot, authentication, profile/email verification,
  invitation registration, post lifecycle, audiences/subscriptions/visibility,
  and password reset. Each names its matrix anchor and assigns relevant
  endpoints exactly once. Add accurate `sequenceDiagram`s for login/authed
  transition, verification, invitation registration, post lifecycle,
  audiences/visibility, and reset; show Browser/Page, server fn, storage, and
  mailer only when they explain the journey.

- [x] **Step 3: Validate partial-corpus fixtures**

  Run the Task 1 focused fixture-test command. Expected: PASS for the parser and
  partial-corpus fixtures. The repository corpus intentionally remains
  incomplete until Task 3, so do not run its complete-corpus test in this task.

- [x] **Step 4: Commit substantive flow docs**

  Tick this task, run `devtool run -- cargo xtask check`, stage `docs/flows/`,
  and commit `docs: map substantive CSR journeys`.

### Task 3: Complete matrix-aligned flow corpus

**Files:**

- Create: `docs/flows/public-reading.md`, `authenticated-cockpit.md`,
  `app-password-management.md`, `administration.md`, `media-management.md`,
  `tag-browsing.md`
- Modify: `docs/flows/README.md`

**Interfaces:**

- Consumes: remaining source-derived endpoints and mounted route patterns.
- Produces: exactly 13 flow documents with every endpoint declared once.

- [x] **Step 1: Write remaining flow narratives and endpoint census**

  Add the six named documents, each with one or more stable `matrix:` links,
  relevant checked routes, concise journey prose, and unique endpoint tokens.
  Use prose/table endpoint census entries rather than low-value sequence arrows.
  Assign `sessions/revoke` to app-password management and let its allowlist
  status remain visible; do not add E2E coverage or hide the gap.

- [x] **Step 2: Reconcile complete inventories**

  Run
  `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -p xtask --run-ignored ignored-only -E 'test(repository_flow_corpus_is_valid)'`.
  Expected: PASS; the repository corpus has zero unassigned/duplicate endpoints
  and any unmapped routes are an informational report. Do not invoke
  `cargo xtask check --no-test` yet: Task 4 has not registered the step.

- [x] **Step 3: Commit complete corpus**

  Tick this task, run `devtool run -- cargo xtask check`, stage all flow docs,
  and commit `docs: complete CSR flow index`.

### Task 4: Wire and project the guard

**Files:**

- Modify: `xtask/src/lib.rs`, `docs/coverage/csr-e2e-matrix.md`,
  `docs/ARCHITECTURE.md`
- Test: `xtask/src/lib.rs` command tests as applicable

**Interfaces:**

- Consumes: `flow_docs::run()`.
- Produces: one named static check in both `check` and `validate`, Architecture
  and matrix links that lead to the single flow-index owner.

- [x] **Step 1: Add failing command-registration tests**

  Extend the existing xtask command/step tests to assert both `check` and
  `validate` invoke a `flow-docs` result. Assert no e2e command is needed for
  this static guard.

- [x] **Step 2: Run RED and wire the step**

  Run:
  `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -p xtask -E 'test(check_and_validate_include_flow_docs)'`.
  Expected: FAIL before registration. Invoke the already-assembled step in both
  command paths; update Architecture’s xtask table.

- [x] **Step 3: Project documentation ownership**

  Replace matrix’s temporary #601 link with `docs/flows/README.md`; add the
  Architecture CSR/frontend link. Do not duplicate the Mermaid graph or matrix
  evidence lists.

- [x] **Step 4: Run GREEN and commit integration**

  Run the Step 2 test and `devtool run -- cargo xtask check`. Expected: PASS.
  Tick this task, stage integration docs/source, and commit
  `feat(xtask): guard CSR flow documentation`.

## Verification

- [x] Run `devtool run -- cargo xtask check` after each committed task.
- [ ] Run `devtool run -- cargo xtask validate` on the final clean branch before
      shipping.
