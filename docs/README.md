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
convention). See the Status column below for each ADR's current status.

<!-- adr-table:begin -->

| #                                                                  | Title                                                                                                        | Status     |
| ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ | ---------- |
| [0000](adr/0000-documentation-strategy.md)                         | Documentation Strategy                                                                                       | accepted   |
| [0001](adr/0001-storage-backends.md)                               | Pluggable Storage Backends                                                                                   | accepted   |
| [0002](adr/0002-frontend-framework.md)                             | Frontend Framework Selection                                                                                 | accepted   |
| [0003](adr/0003-asset-management.md)                               | Asset Management for Single-Binary Distribution                                                              | accepted   |
| [0004](adr/0004-pagination-strategy.md)                            | Pagination Strategy                                                                                          | accepted   |
| [0005](adr/0005-unified-content-model.md)                          | Unified Content Model                                                                                        | accepted   |
| [0006](adr/0006-storage-isolation.md)                              | Storage Isolation (Shared Ingestion vs. Private Copies)                                                      | accepted   |
| [0007](adr/0007-auth-mechanisms.md)                                | Dual-Path Authentication Mechanisms                                                                          | accepted   |
| [0008](adr/0008-deployment-model.md)                               | Single-Binary Deployment Model                                                                               | accepted   |
| [0009](adr/0009-edit-delete-policy.md)                             | High-Fidelity Retention Policy                                                                               | accepted   |
| [0010](adr/0010-protocol-integration.md)                           | Multi-Protocol Integration Strategy                                                                          | accepted   |
| [0011](adr/0011-unified-observability.md)                          | Unified Observability Strategy                                                                               | accepted   |
| [0012](adr/0012-environment-aware-timeouts.md)                     | Environment-Aware E2E Timeouts                                                                               | accepted   |
| [0013](adr/0013-server-submodule-pattern.md)                       | Server Submodule Pattern for Web-Layer Modules                                                               | superseded |
| [0014](adr/0014-atompub-authentication.md)                         | AtomPub Authentication via App-Specific Passwords                                                            | accepted   |
| [0015](adr/0015-atompub-serialization-surfaces.md)                 | Separate Serialization Surfaces for Syndication and AtomPub                                                  | accepted   |
| [0016](adr/0016-dependency-injection-and-appstate.md)              | Dependency Injection, the `AppState` Bundle, and the Composition Root                                        | accepted   |
| [0017](adr/0017-error-handling-and-the-public-boundary.md)         | Error Handling — Typed Domain Errors, a Masked Public Boundary, and Typed Internal Sources                   | accepted   |
| [0018](adr/0018-constant-time-authentication.md)                   | Timing-Equalized Authentication (Username-Enumeration Resistance)                                            | accepted   |
| [0019](adr/0019-generic-storage-backend-via-dialect.md)            | Generic storage backends via a `Backend` marker and per-trait `Dialect`                                      | accepted   |
| [0020](adr/0020-content-visibility-and-subscription-model.md)      | Content Visibility and Subscription Model                                                                    | accepted   |
| [0021](adr/0021-sqlite-transaction-discipline.md)                  | SQLite Dialect Transaction Discipline                                                                        | accepted   |
| [0022](adr/0022-validate-before-expensive-work.md)                 | Validate Cheaply Before Expensive Work for High-Entropy Secrets                                              | accepted   |
| [0023](adr/0023-atompub-jaunder-wire-extensions.md)                | AtomPub Jaunder Wire Extensions                                                                              | accepted   |
| [0024](adr/0024-server-side-org-canonicalization.md)               | Server-Side Org Canonicalization and the Local-vs-Served Representation Principle                            | accepted   |
| [0025](adr/0025-unicode-slug-generation.md)                        | Unicode-Preserving, Never-Fail Slug Generation                                                               | accepted   |
| [0026](adr/0026-test-fault-injection-hooks-feature.md)             | Test-Only Fault-Injection Hooks Behind a `test-utils` Feature                                                | accepted   |
| [0027](adr/0027-scheduled-publishing-time-gated-visibility.md)     | Scheduled Publishing — Time-Gated Visibility and Restart-Durable Go-Live                                     | accepted   |
| [0028](adr/0028-devtool-vs-xtask-boundary.md)                      | The `devtool` / `xtask` Boundary — In-Sandbox Producer vs. Host Analyzer                                     | accepted   |
| [0029](adr/0029-git-enforced-verify-gate.md)                       | Git-Enforced Verify Gate — Hook-Routed check/validate and Clean-Tree Gating                                  | accepted   |
| [0030](adr/0030-coverage-reanchor-text-identity.md)                | Coverage Re-Anchor by Text Identity                                                                          | superseded |
| [0031](adr/0031-elisp-separately-tested-subproject.md)             | Elisp as a Separately-Tested Subproject                                                                      | accepted   |
| [0032](adr/0032-e2e-zero-panic-gate.md)                            | E2E Zero-Panic Gate and Visible-by-Default Server Log                                                        | accepted   |
| [0033](adr/0033-shared-db-test-harness-crate.md)                   | In-`storage` `test_support` Module for Both-Backend Test Parametrization                                     | accepted   |
| [0034](adr/0034-ci-e2e-matrix-distribution.md)                     | CI E2E {backend}×{browser} Matrix Distribution                                                               | accepted   |
| [0035](adr/0035-elisp-live-integration-harness.md)                 | Live Integration Testing of the Emacs Client via a Self-Booting Harness                                      | accepted   |
| [0036](adr/0036-identifier-collision-policy.md)                    | Identifier-Collision Policy for ADRs and Migrations                                                          | accepted   |
| [0037](adr/0037-e2e-failure-diagnostics-capture.md)                | e2e VM Diagnostics Captured Before Failure and Recovered from the Kept outPath                               | accepted   |
| [0038](adr/0038-emacs-http-transport-plz-not-url-el.md)            | The Emacs AtomPub Transport Uses `plz` (curl), Not `url.el`                                                  | accepted   |
| [0039](adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md) | Per-test identity fixtures for parallel-safe e2e specs (parallelism deferred to #173)                        | accepted   |
| [0040](adr/0040-web-rendering-leptos-csr.md)                       | Web rendering: leptos-CSR (drop concurrent reactive SSR)                                                     | accepted   |
| [0041](adr/0041-public-projector-and-csr-client.md)                | Public projector and CSR client (SSR the data, not the components)                                           | accepted   |
| [0042](adr/0042-emacs-org-atom-mapping-struct-seam.md)             | Emacs org→atom mapping: struct seam, `dom-print` serialization, Emacs 29.1 floor                             | accepted   |
| [0043](adr/0043-quick-xml-fork-patch.md)                           | quick-xml advisory: fork + git-patch bridge (RUSTSEC-2026-0194/0195)                                         | accepted   |
| [0044](adr/0044-authenticated-owner-flash-free-enhancement.md)     | Authenticated-owner flash-free enhancement (pre-paint marker + additive decoration)                          | accepted   |
| [0045](adr/0045-emacs-media-content-src.md)                        | Emacs client harvests media URLs from the response `<content src>`                                           | accepted   |
| [0046](adr/0046-test-support-seed-binary.md)                       | A `test-support` binary that links `storage` for out-of-process e2e seeding                                  | accepted   |
| [0047](adr/0047-emacs-publish-orchestration.md)                    | The Emacs Publish Orchestration — Multi-Blog Config via Dynamic Specials, ID-First Safe-to-Resume Write-Back | accepted   |
| [0048](adr/0048-adr-out-of-git-draft-workflow.md)                  | ADRs drafted out of git, numbered at ship                                                                    | accepted   |
| [0049](adr/0049-app-driven-scoped-server-diagnostics.md)           | App-driven scoped server-diagnostics capture                                                                 | accepted   |
| [0050](adr/0050-stateless-coverage-gate.md)                        | Stateless coverage gate — `cov:ignore` + `#[component]` exemption + CRAP threshold                           | accepted   |
| [0051](adr/0051-single-playwright-config.md)                       | One Playwright config for host and CI                                                                        | accepted   |
| [0052](adr/0052-devtool-unifies-static-checks.md)                  | devtool is the single implementation of the non-compiling static checks                                      | accepted   |
| [0053](adr/0053-storage-test-homing-and-dual-backend.md)           | Storage test homing and the dual-backend presumption                                                         | accepted   |
| [0054](adr/0054-backup-test-homing-and-uniform-restore-failure.md) | Backup test homing and the uniform restore-failure contract                                                  | accepted   |
| [0055](adr/0055-web-host-wasm-boundary-module-level.md)            | web host/wasm boundary is module-level, not line-level                                                       | superseded |
| [0056](adr/0056-web-canonical-colocated-leptos.md)                 | web converges on the canonical co-located Leptos CSR layout                                                  | superseded |
| [0057](adr/0057-e2e-capture-dir-contract.md)                       | Single `JAUNDER_CAPTURE_DIR` output-dir contract for e2e capture                                             | accepted   |
| [0058](adr/0058-host-crate-layering.md)                            | A `host` crate for strictly-host-focused shared code                                                         | accepted   |
| [0059](adr/0059-thin-web-shell-error-layering.md)                  | The thin web shell and the T1→T2→T3 error pipeline                                                           | accepted   |
| [0060](adr/0060-web-invalidator-revalidation-idiom.md)             | web revalidation goes through the `Invalidator` primitive, not `action.version()`                            | accepted   |
| [0061](adr/0061-web-keyed-list-reactive-store.md)                  | Web keyed lists render via a reactive Store, patch-fed                                                       | accepted   |
| [0062](adr/0062-macros-crate-proc-macro-home.md)                   | A `macros` crate as the workspace's proc-macro home                                                          | accepted   |
| [0063](adr/0063-domain-value-newtype-convention.md)                | Domain-value newtypes — when to introduce one, and the standard trailer                                      | proposed   |
| [0064](adr/0064-backup-target-auto-derivation.md)                  | Backup target set auto-derived from the live schema; restore defers FK checks                                | accepted   |
| [0065](adr/0065-client-side-domain-validation.md)                  | Typed `#[server]` wire args with client-side pre-validation via the shared newtype                           | proposed   |
| [0066](adr/0066-server-fn-test-registrar-guard.md)                 | Guard the server-fn test registrar with an xtask check                                                       | proposed   |
| [0067](adr/0067-server-integration-tests-one-binary.md)            | Server integration tests are one binary                                                                      | accepted   |
| [0068](adr/0068-tag-identity-label-split.md)                       | A domain value with a canonical identity and a preserved label is two newtypes, not one                      | proposed   |
| [0069](adr/0069-client-crate-wasm-only-home.md)                    | `client` crate — the wasm-only browser-infrastructure home                                                   | accepted   |
| [0070](adr/0070-web-vertical-wasm-only-component-files.md)         | web verticals split host/wasm at the file level — wasm-only `component.rs`                                   | accepted   |
| [0071](adr/0071-sqlx-string-newtype-bridge.md)                     | Transparent sqlx bridge for string newtypes                                                                  | accepted   |
| [0072](adr/0072-timestamps-cross-boundary-as-utcinstant.md)        | Timestamps cross the web boundary as a `UtcInstant` newtype (chrono is already in the wasm bundle)           | proposed   |
| [0073](adr/0073-url-crate-for-absolute-url-normalization.md)       | The `url` crate is the sanctioned absolute-URL normalizer, and it enters the `common`/wasm graph             | proposed   |
| [0074](adr/0074-str-enum-trailer.md)                               | `StrEnum` derive — the standard string-enum trailer                                                          | proposed   |

<!-- adr-table:end -->

## Archive

Superseded planning docs, design specs, and dated snapshots are kept in
[`archive/`](archive/) rather than deleted, named `YYYY-MM-DD-<topic>.md` (the
date the work happened or shipped). They are frozen historical records — read
them for "why we did X," not for current behavior.
