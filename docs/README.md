# Documentation Index

Entry point and map for Jaunder's documentation. New here? Start with the
[root README](../README.md), then [CONTRIBUTING](../CONTRIBUTING.md) — the
definitive working hub for humans and agents.

## Durable docs

| Doc                                                           | What it's for                                                                      |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| [CONTRIBUTING](../CONTRIBUTING.md)                            | Definitive working guide: setup, hooks, workflow, testing, invariants. Read first. |
| [CONTEXT](../CONTEXT.md)                                      | Domain glossary / ubiquitous language for Jaunder.                                 |
| [ARCHITECTURE](ARCHITECTURE.md)                               | System architecture (Leptos, single-binary, storage); links the ADRs.              |
| [DESIGN](DESIGN.md)                                           | High-level design / how an instance runs.                                          |
| [ROADMAP](ROADMAP.md)                                         | Completed-milestone ledger and direction.                                          |
| [observability](observability.md)                             | OpenTelemetry tracing for backend + e2e.                                           |
| [web-style-guide](web-style-guide.md)                         | Conventions for `web/src/pages/` components and widgets.                           |
| [atompub-marsedit-acceptance](atompub-marsedit-acceptance.md) | Manual MarsEdit/AtomPub (RFC 5023) acceptance checklist.                           |

## Architecture Decision Records

Architecture decisions live in [`adr/`](adr/), one file per decision (see
[ADR-0000: Documentation Strategy](adr/0000-documentation-strategy.md) for the
convention). All are currently `accepted`.

| #                                                                  | Title                                                                                      | Status   |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------ | -------- |
| [0000](adr/0000-documentation-strategy.md)                         | Documentation Strategy                                                                     | accepted |
| [0001](adr/0001-storage-backends.md)                               | Pluggable Storage Backends                                                                 | accepted |
| [0002](adr/0002-frontend-framework.md)                             | Frontend Framework Selection                                                               | accepted |
| [0003](adr/0003-asset-management.md)                               | Asset Management for Single-Binary Distribution                                            | accepted |
| [0004](adr/0004-pagination-strategy.md)                            | Pagination Strategy                                                                        | accepted |
| [0005](adr/0005-unified-content-model.md)                          | Unified Content Model                                                                      | accepted |
| [0006](adr/0006-storage-isolation.md)                              | Storage Isolation (Shared Ingestion vs. Private Copies)                                    | accepted |
| [0007](adr/0007-auth-mechanisms.md)                                | Dual-Path Authentication Mechanisms                                                        | accepted |
| [0008](adr/0008-deployment-model.md)                               | Single-Binary Deployment Model                                                             | accepted |
| [0009](adr/0009-edit-delete-policy.md)                             | High-Fidelity Retention Policy                                                             | accepted |
| [0010](adr/0010-protocol-integration.md)                           | Multi-Protocol Integration Strategy                                                        | accepted |
| [0011](adr/0011-unified-observability.md)                          | Unified Observability Strategy                                                             | accepted |
| [0012](adr/0012-environment-aware-timeouts.md)                     | Environment-Aware E2E Timeouts                                                             | accepted |
| [0013](adr/0013-server-submodule-pattern.md)                       | Server Submodule Pattern for Web-Layer Modules                                             | accepted |
| [0014](adr/0014-atompub-authentication.md)                         | AtomPub Authentication via App-Specific Passwords                                          | accepted |
| [0015](adr/0015-atompub-serialization-surfaces.md)                 | Separate Serialization Surfaces for Syndication and AtomPub                                | accepted |
| [0016](adr/0016-dependency-injection-and-appstate.md)              | Dependency Injection, the `AppState` Bundle, and the Composition Root                      | accepted |
| [0017](adr/0017-error-handling-and-the-public-boundary.md)         | Error Handling — Typed Domain Errors, a Masked Public Boundary, and Typed Internal Sources | accepted |
| [0018](adr/0018-constant-time-authentication.md)                   | Timing-Equalized Authentication (Username-Enumeration Resistance)                          | accepted |
| [0019](adr/0019-generic-storage-backend-via-dialect.md)            | Generic storage backends via a `Backend` marker and per-trait `Dialect`                    | accepted |
| [0020](adr/0020-content-visibility-and-subscription-model.md)      | Content Visibility and Subscription Model                                                  | accepted |
| [0021](adr/0021-sqlite-transaction-discipline.md)                  | SQLite Dialect Transaction Discipline                                                      | accepted |
| [0022](adr/0022-validate-before-expensive-work.md)                 | Validate Cheaply Before Expensive Work for High-Entropy Secrets                            | accepted |
| [0023](adr/0023-atompub-jaunder-wire-extensions.md)                | AtomPub Jaunder Wire Extensions                                                            | accepted |
| [0024](adr/0024-server-side-org-canonicalization.md)               | Server-Side Org Canonicalization and the Local-vs-Served Representation Principle          | accepted |
| [0025](adr/0025-unicode-slug-generation.md)                        | Unicode-Preserving, Never-Fail Slug Generation                                             | accepted |
| [0026](adr/0026-test-fault-injection-hooks-feature.md)             | Test-Only Fault-Injection Hooks Behind a `test-utils` Feature                              | accepted |
| [0027](adr/0027-scheduled-publishing-time-gated-visibility.md)     | Scheduled Publishing — Time-Gated Visibility and Restart-Durable Go-Live                   | accepted |
| [0028](adr/0028-devtool-vs-xtask-boundary.md)                      | The `devtool` / `xtask` Boundary — In-Sandbox Producer vs. Host Analyzer                   | accepted |
| [0029](adr/0029-git-enforced-verify-gate.md)                       | Git-Enforced Verify Gate — Hook-Routed check/validate and Clean-Tree Gating                | accepted |
| [0030](adr/0030-coverage-reanchor-text-identity.md)                | Coverage Re-Anchor by Text Identity                                                        | accepted |
| [0031](adr/0031-elisp-separately-tested-subproject.md)             | Elisp as a Separately-Tested Subproject                                                    | accepted |
| [0032](adr/0032-e2e-zero-panic-gate.md)                            | E2E Zero-Panic Gate and Visible-by-Default Server Log                                      | accepted |
| [0033](adr/0033-shared-db-test-harness-crate.md)                   | In-`storage` `test_support` Module for Both-Backend Test Parametrization                   | accepted |
| [0034](adr/0034-ci-e2e-matrix-distribution.md)                     | CI E2E {backend}×{browser} Matrix Distribution                                             | accepted |
| [0035](adr/0035-elisp-live-integration-harness.md)                 | Live Integration Testing of the Emacs Client via a Self-Booting Harness                    | accepted |
| [0036](adr/0036-identifier-collision-policy.md)                    | Identifier-Collision Policy for ADRs and Migrations                                        | accepted |
| [0037](adr/0037-e2e-failure-diagnostics-capture.md)                | e2e VM Diagnostics Captured Before Failure and Recovered from the Kept outPath             | accepted |
| [0038](adr/0038-emacs-http-transport-plz-not-url-el.md)            | The Emacs AtomPub Transport Uses `plz` (curl), Not `url.el`                                | accepted |
| [0039](adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md) | Per-test identity fixtures for parallel-safe e2e specs (parallelism deferred to #173)      | accepted |
| [0040](adr/0040-web-rendering-leptos-csr.md)                       | Web rendering: leptos-CSR (drop concurrent reactive SSR)                                   | accepted |
| [0041](adr/0041-public-projector-and-csr-client.md)                | Public projector and CSR client (SSR the data, not the components)                         | accepted |

## Archive

Superseded planning docs, design specs, and dated snapshots are kept in
[`archive/`](archive/) rather than deleted, named `YYYY-MM-DD-<topic>.md` (the
date the work happened or shipped). They are frozen historical records — read
them for "why we did X," not for current behavior.
