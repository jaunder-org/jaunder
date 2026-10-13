# ADR-DRAFT: Nix owns cached host-tooling and qualification provenance

- Status: proposed
- Date: 2026-10-10
- Issue: [#1734](https://github.com/jaunder-org/jaunder/issues/1734)

## Context

Agent and developer entry points invoke `cargo xtask` before useful work can
begin. Previously, the orchestrator's build script required Git metadata,
embedded the current commit/dirty state, and watched every tracked file. This
made executable freshness unnecessarily depend on unrelated repository edits and
revisions.

[ADR-0028](../0028-devtool-vs-xtask-boundary.md),
[ADR-0052](../0052-devtool-unifies-static-checks.md), and
[ADR-0146](../0146-devtool-owns-compiling-static-check-definitions.md) keep
xtask execution out of derivations and its source out of expensive application
producers. Those boundaries prevent recursive orchestration and cache coupling;
they do not require Cargo to compile every host invocation or prohibit Nix from
building an independently packaged host executable.

Qualification's previous exact-clean-commit executable checks provided a
conservative proxy for compatible tool and harness inputs. They also rejected an
identical tool across commits with unrelated changes. Nix already owns source
closures, derivations and immutable output identities; another fingerprint
protocol or identity ledger would duplicate that authority.

## Decision

Package xtask independently through Nix and execute it only on the host. Normal
entry points resolve the current checkout's flake so unchanged tooling inputs
reuse the same output across checkouts and unrelated application revisions.

Declare the complete tool closure in Nix, including local dependencies,
manifests/lockfiles, build configuration, embedded data and host-produced theme
resources. Keep xtask absent from the application producer source/dependency
closures. A shared input that the tool genuinely embeds may invalidate it.

Nix owns qualification's tool and runtime harness inputs. Reuse its existing
browser, test-support and VM packages and bind execution to the outputs selected
for the admitted source. Use standard derivation/output identity for admission
and evidence, not an independently computed source digest, tool-commit stamp,
compatibility manifest or provenance ledger. Qualification must reject missing
or mismatched selected outputs before lifecycle mutation or publication.

Git commits continue to identify clean, immutable harness/product source and the
source/target revisions under test. They do not have to equal a Git revision
embedded in a content-equivalent executable. Browser execution uses the admitted
Nix inputs, or an invocation-owned copy of them; mutable ambient source cannot
bypass the graph. Preserve checkout-drift rejection and existing qualification
fixture, backend, lease, lifecycle, sanitization and evidence guarantees.

Resolve live checkout paths at runtime rather than using a compiled build-source
directory. Refuse newly created untracked tooling inputs that the Git-backed
flake snapshot would omit, with guidance to include them before execution.
Neither missing inputs nor failed provenance admission may select an older
binary as a successful fallback.

This narrowly replaces local-Cargo compilation and exact-commit executable
matching as tool-provenance requirements. It preserves ADR-0028's execution
litmus and [ADR-0141](../0141-cargo-workspace-execution-boundaries.md)'s
separate workspace and explicit host-unit-test authorities. Cargo remains
available for focused tests and explicit source-development diagnostics, not as
a separate qualification lane.

## Consequences

Cold realization still compiles the tool; warm use and matching source closures
reuse Nix outputs. Tool edits do not invalidate application coverage/E2E merely
because the host tool orchestrates those producers.

Qualification no longer attributes an executable to a fabricated current Git
commit. It admits the Nix-selected executable and runtime graph while recording
the actual source/product revisions being qualified. Ordinary evidence uses
schema v2 to require the selected harness's standard derivation, output and NAR
identity fields alongside its source commit and manifest authorities.

Nix source completeness, supported native/runtime dependency closures, local
checkout discovery, and untracked-input refusal need regression proof. Changing
the provenance boundary does not reduce testing or substitute a source-identity
probe for executing the real consumer.

No new domain vocabulary is introduced: this is development/qualification
infrastructure, not a change to Jaunder's ubiquitous language.
