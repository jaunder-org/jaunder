# Issue #310: CSR flow-to-e2e traceability matrix

- Issue: [#310](https://github.com/jaunder-org/jaunder/issues/310)
- Status: proposed
- Date: 2026-08-14

## Problem

CSR/WASM user-visible flows do not contribute to the host line-coverage
measurement. The current Playwright suite exercises many of those flows, but
there is no reviewable inventory connecting a flow to the spec files that cover
it. A CSR feature can therefore land without an e2e spec and the omission is not
visible in the source tree.

ADR-0050 defines the stateless host coverage gate, while ADR-0070 makes CSR
components wasm-only and absent from its denominator. ADR-0081 separately
derives server-fn flow coverage from e2e trace evidence. Neither artifact is a
matrix of user-visible CSR flows and their Playwright specs.

## Decision

Add `docs/coverage/csr-e2e-matrix.md`, the canonical checked-in Markdown mapping
from current user-visible CSR flows to the Playwright `*.spec.ts` file or files
that exercise each flow.

A **CSR flow** is a stable user-visible capability available after the CSR app
mounts, named in product terms and identified by its route or user entry point.
It is not an implementation module, a helper, a `#[server]` function, or a
protocol-only endpoint. One flow may cite more than one spec; one spec may cover
more than one flow.

The matrix is documentation, not a generated artifact or a new coverage gate.
Its maintenance instructions require a contributor changing a CSR flow or its
e2e coverage to update the relevant matrix row in the same change. This is a
review obligation, not a name-resolution check: existence of a cited spec file
does not prove that it exercises the stated flow.

The matrix links to the checked server-fn coverage artifacts governed by
ADR-0081. It links to [#601](https://github.com/jaunder-org/jaunder/issues/601)
until `docs/flows/` exists; #601 flow documentation must reference these
canonical matrix rows rather than repeat the mapping. The matrix does not claim
ADR-0081's derived trace evidence as its own.

## Scope

In:

- Inventory candidates from every mounted CSR route and other stable user entry
  point. The matrix records each candidate's disposition: one user-visible flow
  row, or an explicit out-of-scope reason when it is not a user-visible CSR
  capability.
- Map every resulting user-visible CSR flow to the existing Playwright spec file
  or files that exercise it.
- Make coverage state explicit in the matrix, including an issue link for every
  uncovered flow.
- Search for duplicate follow-ups for every uncovered flow. Reuse one matching
  open issue when found; otherwise file one focused issue. Record the
  duplicate-search result and chosen issue link in the matrix.
- Document the same-change maintenance workflow in the matrix.
- Link the matrix to ADR-0050, ADR-0070, ADR-0081's server-fn coverage
  artifacts, and #601 without duplicating their data.

Out:

- Adding, changing, or broadening e2e flows to close discovered gaps.
- A generated trace artifact, a static spec-name validator, or any new coverage
  gate.
- Mapping implementation modules, helpers, `#[server]` functions, or
  protocol-only endpoints as if they were user-visible CSR flows.
- An ADR or `CONTEXT.md` change.

## Acceptance criteria

1. `docs/coverage/csr-e2e-matrix.md` is checked in and includes a candidate
   inventory of every mounted CSR route and other stable user entry point. Each
   candidate is either assigned once to a named user-visible CSR flow, with its
   route or entry point, or has an explicit out-of-scope reason.
2. Every flow row cites one or more existing `end2end/tests/*.spec.ts` files
   whose exercised user flow matches the row, or states that the flow is
   uncovered. No row claims coverage from a helper, an implementation module, or
   a server-fn inventory alone.
3. Every uncovered flow records its duplicate-search result and links to exactly
   one focused open follow-up issue: an existing matching issue is reused, or a
   new issue is filed when no duplicate exists. The implementation adds no
   unrelated e2e behavior to make such rows covered.
4. The document states that CSR flows are user-visible mounted-app capabilities,
   distinguishes them from modules, `#[server]` functions, and protocol-only
   endpoints, and requires matrix updates in the same change as affected CSR
   flows or e2e coverage.
5. The matrix is the canonical CSR-flow-to-e2e-spec mapping. It links to
   ADR-0050, ADR-0070, `docs/coverage/server-fns.json`,
   `docs/coverage/server-fns-evidence.json`, and #601; until `docs/flows/`
   exists it links to #601's issue, and its eventual flow documentation must
   reference matrix rows rather than copy this mapping or the server-fn
   inventory/per-test evidence.
6. The matrix remains ordinary Markdown with no generated source, static
   name-resolution gate, trace-pipeline change, or test/configuration interface
   change.

## Verification

- Review every candidate route and stable entry point against its recorded
  disposition, then review every flow row against its cited Playwright spec
  file(s).
- Confirm every uncovered row records a duplicate search and links to exactly
  one open issue whose scope is limited to that flow's missing e2e coverage.
- Run the repository documentation/static gate selected by the approved plan.
