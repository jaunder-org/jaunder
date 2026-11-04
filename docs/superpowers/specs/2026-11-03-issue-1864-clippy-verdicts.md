# Proposed #1864 discrete Clippy verdicts

**Status: human-approved six-unit design and approved implementation outline.**
Human request `2f0929a1-ebbd-4735-9a6c-b53e99a7a141` /
`issue1864-clippy-v2-coverage` approved “Approve six; design cached xtask.”
Approval explicitly accepts removing the old combined Web(server+CSR) config;
retain distinct root/tools lock and test contexts, and retain the
performance-parity qualification. No other coverage removals are approved.
Cached raw xtask unit is separately design-authorized only; it is not a seventh
unit or part of this six-unit scope. This approval does not authorize
implementation. This proposal replaces all five old Code Clippy invocations with
six discrete package/target/configuration units that can be aggregated and
scheduled independently. Detailed metadata and lock-context evidence is in
`.xtask/evidence/clippy-discrete-coverage.md`.

## Outcome and proposed units

| Unit                | Selected Clippy command                                                                              | Selected lint targets                                                                                                                                                                |
| ------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Product core**    | `cargo clippy --workspace --exclude web --exclude client --exclude csr --all-targets -- -D warnings` | Eight root default packages: server, common, host, macros, storage, test-support, csr_bundle, performance. Excluded packages may compile as dependencies but are not Clippy targets. |
| **Web server**      | `cargo clippy -p web --features server --all-targets -- -D warnings`                                 | Web’s server-feature host targets.                                                                                                                                                   |
| **Web featureless** | `cargo clippy -p web --no-default-features --all-targets -- -D warnings`                             | Web with its declared empty defaults and no server feature.                                                                                                                          |
| **Client host CSR** | `cargo clippy -p client --features csr --all-targets -- -D warnings`                                 | Host client targets with CSR feature, preserving the root graph’s host `client: csr` package unit.                                                                                   |
| **CSR wasm**        | `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings` | Web/client/csr wasm targets; intentionally no `--all-targets`.                                                                                                                       |
| **Tools**           | `cargo clippy --manifest-path tools/Cargo.toml --workspace --all-targets -- -D warnings`             | All seven tools workspace packages, retaining the existing tools-lock context pending the performance lint-configuration qualification below.                                        |

Remove all five historical Code calls—`clippy`, `web-server-clippy`,
`web-no-server-clippy`, `wasm-clippy`, and `tools-clippy`—from both Nix Code
execution and the old host-step mapping. Replace them exactly once with
`product-core-clippy` (new), `web-server-clippy`, `web-no-server-clippy`,
`client-host-clippy` (new), `wasm-clippy`, and `tools-clippy`; update existing
names where target populations change. Residual Code contains only non-Clippy
checks. Keep separate host-only `xtask-clippy`; Nix never invokes xtask. Cargo
dependencies may compile in multiple leaves but do not count as duplicated
Clippy targets.

## Coverage decisions requiring human approval

Root metadata resolves 11 default packages: nine explicit members plus implicit
`csr_bundle` and `performance`; tools has seven and overlaps those two. A locked
root feature-tree comparison with `web`, `client`, and `csr` excluded leaves the
other root packages’ local feature rows unchanged. The excluded packages may
remain build dependencies, but Cargo Clippy does not lint them as selected
targets in product core.

Root feature unification enables both `web/server` and `web/csr`. Source
inspection found independent server-gated code and wasm CSR code, but no source
cfg requiring both web features together; the server executable selects server
and the CSR artifact selects CSR. The combined root web(server+CSR) signal
therefore appears to arise from workspace unification, not a demonstrated
product configuration. It may still catch a dependency-level incompatibility.
**Human approval must explicitly accept removing that old combined signal or
request it as an additional check.** The proposal does not assert equivalence.

The root `csr` host target is empty under `csr/src/lib.rs:1`; its source is
covered by CSR wasm. `performance` and `csr_bundle` are both root/tools members.
`performance` is one lib target with identical resolved feature trees across
workspaces, but **effective inherited Clippy lint settings have not been
established**: `tools/performance/Cargo.toml` uses `lints.workspace = true`, and
package selection from the root command alone does not prove which workspace
lint table applies. Therefore the proposed tools leaf retains the full tools
workspace for now. Before excluding performance, establish effective lint/config
parity with bounded source/docs/metadata evidence; if parity is not established
or contexts differ, retain the tools target or return that population removal
for human decision. `csr_bundle` has different tempfile test dependency
resolutions across root/tools locks; preserve both `--all-targets` contexts.

## Nix composition, inputs, and tradeoffs

Current Nix `static-code` runs the Code group plus its census consumer
(`nix/checks.nix:3300–3313`). Replace—not supplement—the five old Clippy
executions with the six leaves; preserve every non-Clippy verdict and required
aggregate. The six units preserve workspace/feature/target coverage while
enabling Nix scheduling. They cost additional derivation and dependency setup
and may repeat feature-specific compilation; no speedup is measured. Per-package
refinements remain possible, but must retain the same target/configuration units
and not lint dependencies redundantly.

Define the six units as independently buildable terminal Nix derivations. A
single narrow Clippy aggregate depends on all six outputs; its own success
requires every dependency to succeed and it does not rerun any lint command. The
required hermetic static consumer requests that aggregate once, allowing Nix to
schedule the unit dependency graph rather than sequentially launching six
builds. Expose the aggregate and individual units through the flake check
composition; repeated references to an identical derivation are dependencies,
not duplicate lint executions. Preserve the non-Clippy residual Code verdict and
unrelated static families.

Inputs include root/tools manifests and locks, selected sources/build scripts,
`.cargo`, `clippy.toml`, Rust toolchain and wasm target std, and existing
devtool/product/tools offline homes. Those transitive inputs include broad
tools/product and CSS coupling
(`nix/packages.nix:125–204,335–383,468–526,580–677`); no narrow-cache claim is
made without evaluated identities. Cargo-deny remains outside Clippy because
host checks advisories and sandbox omits them (ADR-0145). Preserve host Fix,
Git/index admission and reconciliation, prepush behavior, platform limits, and
required CI aggregates.

TS/ERT work remains independently approved and unblocked. Coordinate only actual
same-unit conflicts across assigned checkouts; there is no blanket completion
wait. Raw xtask-format’s spec and outline are approved (outline decision
`881f60c8-5c6d-471c-85f9-906093888121`, `issue1864-xtask-format-outline-v1`,
“Approve outline and begin”); its implementation is assigned to separate branch
`issue-1864-xtask-format` / checkout `w29` and is independent of Clippy. Scoped
Rust-format spec is also human-approved (request
`1b68e607-ca9a-460b-b005-179532a44a9d`, `issue1864-rust-format-v3`, “Approve
scoped-format spec”); its implementation outline remains pending approval and is
not a Clippy dependency. The six-unit implementation outline is approved under
request `b7a283f3-fbee-40a3-b8bc-3abb28bd3ab6` / `issue1864-clippy-outline-v1`,
exact answer “Approve outline and begin.” Implementation assignment startup is
not confirmed; Manager owns recovery. This status update does not authorize this
agent to take over that separate implementation. #1859 aggregate ownership is
separate and not a prerequisite. Do not freeze or duplicate in-progress timing
JSON; use accepted timing conventions and mark current execution unavailable
without positive current-request attribution. No hook/CI policy change, generic
framework, or new ADR is proposed.

## Acceptance and approval ask

After approval, prove every selected lint target/configuration is represented
exactly once, except the explicitly retained same-source root/tools lock
contexts. First establish effective performance lint/config parity before
removing its tools target; excluded dependencies must be described as
compile-only, not linted. Verify all remaining non-Clippy checks stay required,
and deliberate failures reach each leaf and aggregate. Resolve performance’s
effective lint configuration before removing its tools target. Compare relevant
and permitted unrelated derivation identities and valid local reuse without
forcing rebuilds/cache purges. Metadata and Cargo-tree evidence are not
Clippy/compiler proof.

Verify that the required aggregate has all six unit derivations as dependencies,
that every unit executes its selected lint command once, that any failed unit
prevents aggregate success, and that the caller does not serialize six
independent build requests. Distinguish aggregate request duration from per-unit
execution evidence; no per-unit timing is inferred from aggregate duration or
reused logs.

**Approval record:** the six-unit design is approved under the human request
above, including removal of the combined root web(server+CSR) configuration.
Keep root/tools lock and test contexts distinct; do not remove performance or
any other coverage absent the specified parity evidence and a scope decision.
The triggered outline is approved under request
`b7a283f3-fbee-40a3-b8bc-3abb28bd3ab6` / `issue1864-clippy-outline-v1`, exact
answer “Approve outline and begin”; implementation assignment startup is not
confirmed and remains with Manager for recovery.

Detailed evidence and output paths:
`.xtask/evidence/clippy-discrete-coverage.md`; resolved features
`result:rust-families#7`; prior feasibility `result:rust-families#9`; commands
`tools/devtool/src/check.rs:295–361`; Nix consumers `nix/checks.nix:3211–3313`;
workspace/feature refs listed in the evidence file. Canonical
policy/communication: `evidence:policy#212`, `evidence:communication#212`.
