# CSR flow-to-e2e traceability matrix Implementation Plan

> **For agentic workers:** Execute this plan with `jaunder-iterate` (delegating
> an individual step through `jaunder-dispatch` when useful). Steps use checkbox
> (`- [ ]`) syntax for tracking.

**Goal:** Make current CSR flow coverage auditable through one canonical,
reviewable mapping to the Playwright specs that exercise it.

**Scope:**

- In: duplicate-safe follow-up issues for uncovered flows; a complete mounted
  CSR candidate inventory; the canonical Markdown matrix, evidence links,
  maintenance workflow, and #601 contract alignment.
- Out: e2e implementation; generated mappings; static name-resolution checks;
  trace-pipeline, test/configuration, ADR, and `CONTEXT.md` changes.

**Task:**

1. Audit, publish, and commit the canonical CSR-flow-to-e2e mapping; align #601.

**Key risks/decisions:** CSR components are wasm-only under ADR-0070, so host
coverage cannot establish browser-flow coverage. A spec filename is a reviewable
behavioral claim, not generated proof. The matrix makes that claim visible
through same-change review; it does not add a misleading name-only gate. Every
flow has a stable heading anchor, so #601 can link to the canonical row instead
of recreating a second mapping.

**Architecture:** `web/src/app/component.rs` owns the mounted `ParentRoute`,
child `Route`, and fallback candidate inventory. Manual review of existing
Playwright behavior establishes the flow-to-spec relation. One Markdown document
records candidate disposition, anchored flow coverage, and uncovered-flow issue
evidence. ADR-0081 server-fn coverage remains a linked, separate evidence
surface.

**Tech Stack:** Markdown, GitHub issues, existing Playwright TypeScript specs,
`prettier`, `cargo xtask`.

## Global Constraints

- Follow the approved
  [Issue #310 specification](../specs/2026-08-14-issue-310-csr-e2e-traceability-spec.md).
- A CSR flow is a stable user-visible mounted-app capability; never substitute a
  module, helper, `#[server]` function, or protocol-only endpoint.
- Record one disposition for the shell `ParentRoute`, every child `Route`, the
  `Routes` fallback, and every stable entry point outside the route table:
  assigned flow or explicit out-of-scope reason.
- Cite a Playwright spec only after reading its tests, setup, navigation, and
  assertions and confirming it exercises the stated flow. A filename, helper, or
  server-fn snapshot alone is not coverage evidence.
- Uncovered flow follow-ups are `Task` issues in milestone 6. Use verified topic
  labels only; `test-infra` is appropriate, `web` is not. Do not add e2e
  coverage in this issue.
- `docs/coverage/csr-e2e-matrix.md` is the canonical mapping. It must link
  ADR-0050, ADR-0070, ADR-0081, `server-fns.json`, `server-fns-evidence.json`,
  and #601 without copying any sibling inventory or per-test evidence.
- Matrix flow entries use stable `###` headings. A future #601 flow document
  links to the relevant heading fragment, never to a copied `Pinned by` list.
- Format Markdown with pinned `prettier`. Stage the new matrix and completed
  task checkbox before `cargo xtask` checks, because tracked-document checks use
  the staged population. Run `devtool run -- cargo xtask check --no-test` while
  editing and the full `devtool run -- cargo xtask check` before commit; inspect
  and restage fix-mode changes.
- Stage before committing. No `Co-Authored-By` trailer.

---

### Task 1: Audit, publish, and commit the canonical matrix

**Files:**

- Read: `web/src/app/component.rs:105-164`
- Read: `end2end/tests/*.spec.ts`
- Read: `docs/coverage/server-fns.json`
- Read: `docs/coverage/server-fns-evidence.json`
- Create: `docs/coverage/csr-e2e-matrix.md`
- Modify: Issue #601 body — Context, Proposal spec-anchoring, Relationship to
  #310, Acceptance, and Refs

- Create: one GitHub Issue for an uncovered CSR flow only if its duplicate
  search finds no exact-scope open issue

**Interfaces:**

- Consumes: the mounted route table, existing Playwright behavior, approved spec
  criteria 1–6, and the issue-tracker triage contract.
- Produces: one committed `docs/coverage/csr-e2e-matrix.md`; one exact-scope
  open follow-up issue per uncovered flow; an updated #601 body that links its
  future documentation to matrix heading anchors.

- [x] **Task 1 complete:** Matrix, follow-up ownership, #601 contract, staged
      checks, and commit satisfy this task's steps.

- [x] **Step 1: Build the complete candidate inventory.**

  Read the full `App` route table. Record the shell `ParentRoute`, every literal
  and parameterized child `Route`, and the `Routes` fallback. Inspect
  route-owning components and navigation surfaces for stable user entry points
  outside that table. Assign every candidate to exactly one user-visible flow or
  an explicit out-of-scope reason; retain every candidate path even where
  multiple paths are one flow.

- [x] **Step 2: Prove each flow's current e2e state.**

  Read relevant `end2end/tests/*.spec.ts` test titles, fixture setup,
  navigation, and behavioral assertions. Record exact spec paths only when a
  test exercises the flow. Mark a flow uncovered when no such test exists; do
  not infer coverage from server-fn evidence or implementation structure.

- [x] **Step 3: Resolve every uncovered flow in the tracker.**

  Before searching, call GitHub `get_me` and list the organization issue types;
  verify that `Task` is available. For each uncovered flow, search open
  `jaunder-org/jaunder` issues using its route and capability terms. Read every
  candidate issue body. In the matrix, record the search query, every candidate
  issue URL, the exact matching scope of each acceptable issue, and the mismatch
  for each rejected one. If one or more exact-scope issues remain, select the
  lowest-numbered and record that deterministic selection; never create another
  issue. Otherwise create one through GitHub `issue_write` with type `Task`,
  title `test(e2e): cover <CSR flow>`, label `test-infra`, and milestone `6`.
  Its body states only the route/entry point, missing behavioral coverage, and
  implementation work that remains out of scope.

  For the selected issue—reused or newly created—read it back. If GitHub
  stripped angle-bracket markup, rewrite that passage in prose and reread until
  intact. Verify type `Task`, milestone `6`, and that its complete label set
  contains only verified topic labels, including `test-infra`; `web` or another
  layer label requires normalization. If any metadata or label rule differs, use
  GitHub `issue_write` with `method: "update"`, the selected issue number,
  `type: "Task"`, `milestone: 6`, and only the verified topic labels; reread the
  issue to confirm normalized metadata and unchanged exact-scope body.

  Query this issue's own project item; never scan the whole project:

  ```bash
  devtool run -- gh api graphql -F number=<issue-number> -f 'query=query($number:Int!){repository(owner:"jaunder-org",name:"jaunder"){issue(number:$number){projectItems(first:10){nodes{id project{number} fieldValueByName(name:"Priority"){... on ProjectV2ItemFieldSingleSelectValue{name optionId}}}}}}}'
  ```

  If the query, `item-add`, or `item-edit` reports missing `project` scope, run
  `devtool run -- gh auth refresh -s project`, retry the failed operation, and
  rerun this per-issue query. If no returned node has project number `1`, add
  the issue to Jaunder Backlog:

  ```bash
  devtool run -- gh project item-add 1 --owner jaunder-org --url <issue-url>
  ```

  After every successful `item-add`, normal or retried, rerun the per-issue
  query. Select its Project #1 item id, set P3, and rerun the query to confirm
  `Priority.name == "P3"`:

  ```bash
  devtool run -- gh project item-edit --project-id PVT_kwDOECw7os4BblPP --id <item-id> --field-id PVTSSF_lADOECw7os4BblPPzhWUx50 --single-select-option-id 0bba09bc
  ```

  First read the selected issue's existing `blocked_by` list. If and only if a
  real open prerequisite exists and is absent, get its database id and create
  the native dependency, then read the issue's list again to confirm it:

  ```bash
  devtool run -- gh api repos/jaunder-org/jaunder/issues/<issue-number>/dependencies/blocked_by
  devtool run -- gh api repos/jaunder-org/jaunder/issues/<blocker-number> --jq .id
  devtool run -- gh api --method POST repos/jaunder-org/jaunder/issues/<issue-number>/dependencies/blocked_by -F issue_id=<blocker-database-id>
  devtool run -- gh api repos/jaunder-org/jaunder/issues/<issue-number>/dependencies/blocked_by
  ```

  Record all created/reused issue URLs and metadata, project/P3, and dependency
  verification results in the matrix.

- [x] **Step 4: Write the anchored canonical matrix.**

  Create `docs/coverage/csr-e2e-matrix.md` with a candidate table containing
  each route/entry point, its disposition, and either a flow name or an
  out-of-scope reason. Add one `###` heading per user-visible flow; that heading
  is the stable link target. Under each heading, give a compact row/table with
  candidate routes or entry points, exact relative Playwright spec links, and
  coverage state. Each uncovered entry also contains the Step 3 search result,
  candidate/rejection evidence, selected issue URL, and selection reason.

- [x] **Step 5: State the evidence boundary and maintenance contract.**

  Define the matrix as user-visible mounted-app flow documentation, distinct
  from implementation modules, helpers, `#[server]` functions, and protocol-only
  endpoints. Link ADR-0050 and ADR-0070 for the host-versus-wasm coverage
  boundary; link ADR-0081, `server-fns.json`, and `server-fns-evidence.json` as
  separate server-fn evidence without copying them. Link Issue #601 only until
  `docs/flows/` exists; once it lands, replace that issue link with its
  documentation index while requiring every flow page to point at the relevant
  matrix heading anchor. Require contributors to update affected candidates and
  flow sections in the same change as a CSR-flow or e2e-coverage change. State
  this is review discipline, not a generated or name-resolution guarantee.

- [x] **Step 6: Repair and align Issue #601's full tracker contract.**

  Read Issue #601's complete persisted body before editing. Reconstruct every
  already-mangled angle-bracket passage in prose—especially the old
  `web/src/pages/mod.rs` route authority, vertical path template, flow filename
  template, and Router reference—using the current `web/src/app/component.rs`
  route table and its 24 mounted routes. Through GitHub `issue_write`, replace
  the Proposal's spec-anchoring paragraph, Relationship to #310 section, and
  Acceptance spec-link criterion so future flow documents link relevant
  `csr-e2e-matrix.md` heading anchors rather than declaring a second `Pinned by`
  mapping. State that #601 remains the matrix's linked future consumer until
  `docs/flows/` exists, then its index replaces that issue link. Preserve #601's
  Mermaid, route/endpoint-validation, and sequence-diagram scope. Read back the
  full updated body; if GitHub stripped markup, rewrite it in prose and retry
  until all repaired passages and all three canonical-link requirements persist.

- [ ] **Step 7: Format, stage, gate, and commit the atomic documentation task.**

  Run: `devtool run -- prettier -w docs/coverage/csr-e2e-matrix.md`

  Expected: the matrix is formatted by the repository-pinned formatter.

  Review it against every route-table candidate and cited Playwright spec.
  Verify one disposition per candidate, behavior-backed citations for covered
  flows, and exactly one verified canonical open follow-up for every uncovered
  flow. Tick **Task 1** in this plan. Stage this plan and the matrix, then run:

  `devtool run -- cargo xtask check --no-test`

  Expected: PASS. Inspect intended fix-mode changes and restage them. Then run:

  `devtool run -- cargo xtask check`

  Expected: PASS. Inspect and restage intended fix-mode changes. Inspect the
  staged diff; it contains the matrix, completed plan task, and only intended
  mechanical changes. Commit with
  `docs(coverage): map CSR flows to e2e specs (#310)` and no `Co-Authored-By`
  trailer.

## Execution handoff

Execute after plan approval with `jaunder-iterate`. This is one atomic
traceability deliverable: the audit establishes only evidence that the matrix
persists, uncovered flows receive independently triaged ownership before being
recorded, #601 consumes the same anchored mapping, and the staged document is
covered by both required check tiers before commit.
