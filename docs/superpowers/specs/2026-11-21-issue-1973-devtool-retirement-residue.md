# Issue #1973: finish devtool retirement — rebuild isolation, one wasm parser, ADR record

## Outcome

Editing an unrelated tools crate (for example `tools/traced-run/src`) no longer
changes the product binary's derivation. Diagnostic-build reads wasm through the
same library as `host-proof`, so the repository has no hand-rolled wasm section
parser. An ADR draft records the experiment-endpoint retirement and the
delivered owners that ADR-0246 still describes as pending.

## Approach and tradeoffs

**1. Rebuild isolation.** Today `systemArtifactsBin` is built by `mkToolsBin`
from the shared `toolsSrc`, which admits nearly all of `tools/`. The CSR bundle
runs `system-artifacts`, and `mkJaunderBin` embeds the bundle. Measured on
`5a87343be`: appending a comment to `tools/traced-run/src/main.rs` changed
`packages.jaunder` from `i8hvpjhi…-jaunder-0.1.0.drv` to `iidlcz4g…`. Only
`jaunder-tools-cargo-source` changed; the shared dependency build did not.

The fix uses an existing builder. `mkHostTool` (`nix/host-tool.nix`) already
builds `host-proof`, `coverage-check`, `perf-data`, `pr-observer` and
`qualification-host`. Each is built from its own Cargo path closure, with other
workspace members as placeholders, on a dependency build that only the lockfile,
manifests, toolchain and features change. It already admits the non-Cargo inputs
`host` compiles in (`server/assets/jaunder.css`, `host/system_theme_sources/`).
ADR-0258 already lists `system-artifacts` among the packages built this way; the
code never caught up. `systemArtifactsBin` and the `qualification`-featured
build move to `mkHostTool`. Their closure is `tools/system-artifacts`, `host`,
`common` and `macros`. `mkToolsBin` keeps coverage-sandbox, doctests and
perf-producer.

The live-checkout admission check for CSR builds (`csrSourceAdmits`) currently
ORs in the whole `toolsSourceFilter`. It switches to the system-artifacts
package's `sourceAdmits`, so admission matches what the build reads. The
`toolsSourceFilter` comment that says not to narrow sources "to manufacture
cache hits" is revised to describe its remaining consumers. Eval-time assertions
on `sourceAdmits`, following `nix/qualification.nix:15-17`, lock the boundary.
The system-artifacts source must admit its own and `host`'s sources and the
theme inputs, and must not admit another tools crate's sources.

- _Accepted cost:_ system-artifacts gets its own dependency build (dev profile,
  like the other closure-built tools) rather than sharing the release tools
  dependency build. That build reruns only on lockfile, manifest, toolchain or
  feature changes and is cached in Cachix. `mkHostTool`'s header ("never
  executes inside a derivation") is reworded: system-artifacts also runs inside
  the CSR bundle derivations.
- _Not done:_ narrowing coverage-sandbox, doctests or perf-producer. None of
  them reaches the product binary, and each would need its own input audit.

**1b. The product's own source (found during implementation).** With
system-artifacts isolated, the probe edit still changed `packages.jaunder`. The
cause was its direct `source` input. `productSourceFilter`, the source behind
`commonArgs` (product binary, CSR wasm, nextest, coverage and every other
root-workspace build), admits all of `tools/`. It excludes only the
repository-policy, qualification and host-proof implementations and the
coverage/doctests host and test code. That breadth dates from when devtool
shared this source.

The root workspace reaches only `tools/performance` (from `test-support`). The
five hand-written exclusions are replaced by one rule: from `tools/`, admit only
the root workspace's Cargo path closure, computed by `cargoPackageClosure`.
Eval-time assertions require `tools/performance` sources to be admitted and
traced-run, system-artifacts and the host-proof manifest to be left out.
Verified: with this rule, `packages.jaunder` is `wnr4vv45…` both with and
without the probe edit. `nix flake check --no-build --keep-going` reports no
assertion or evaluation error, only import-from-derivation paths that
`--no-build` will not build.

- _Accepted cost:_ every product derivation's source changes once, so the first
  build after this lands rebuilds the root workspace's derivations.
- _Accepted cost:_ the host-tool manifests that were kept (repository-policy,
  qualification, host-proof) leave the product source too. No root-workspace
  build reads them.

**2. One wasm parser.** `host-proof` parses wasm with the `wasmparser` crate,
which is already in `tools/Cargo.lock`. `coverage-sandbox diagnostic-build`
hand-rolls LEB128, section, custom-name and export parsing
(`diagnostic_build.rs:413-525`). That parsing moves onto `wasmparser` at the
same version, as a plain dependency of the `coverage` crate. The
`coverage-check` build therefore also compiles it; this is accepted. For valid
modules the recorded results stay the same: per boundary, whether each coverage
section is present, whether `__wasm_bindgen_unstable` is present, and the export
names. Malformed input is still an error. `wasmparser` is somewhat stricter, and
the error text changes. ADR-0246 forbids a coverage-to-host-proof dependency, so
the crates share the library, not `host-proof`'s module.

**3. ADR draft amending ADR-0246** (`jaunder-adr`, numberless). The draft
records three decisions:

- The experiment-only coverage emit/worker/aggregate endpoints are retired. The
  human confirmed this on 2026-11-21 in the #1973 comment, closing the open
  decision on #1859. This replaces ADR-0246:95-99's DEFERRED clause and its "not
  claimed or permitted".
- The delivered owners replace ADR-0246:72-74 and :81-82: `coverage-sandbox` in
  `tools/coverage` (emit, validate-status, wasm, diagnostic-build), `doctests`,
  the `perf-producer` crate, and the `system-artifacts` crate.
- From `tools/`, the product source admits only the root workspace's Cargo path
  closure (1b). This replaces the per-crate exclusions that ARCHITECTURE.md's
  "host tools are host-only" invariant describes.

The draft notes that `system-artifacts` is built as ADR-0258 describes, which
retires ADR-0251's shared-source consequence for that executable. Current state
goes into `docs/ARCHITECTURE.md` in the same change, citing the draft:

- the in-derivation executables bullet (~5328),
- the "host tools are host-only" invariant paragraph (~5404),
- the Nix-built host tools paragraph (~5420),
- §Residual tooling owners (~5640-5675).

CONTEXT.md is considered; no term changes.

## Acceptance

Proof to execute (results go in the PR, not the tree):

- With the same probe edit to `tools/traced-run/src/main.rs`, `packages.jaunder`
  and `packages.system-artifacts` keep their `drvPath`. A probe edit to
  `host/src` must still change both.
- The following build: `packages.jaunder`, `csrBundle`, both system-artifacts
  builds, and the system-artifact qualification packages.
- The existing diagnostic-build, coverage, doctest and performance checks pass.
  Status files keep the same schema and verdicts. Derivation-identity fields
  (for example the performance compatibility identity) are expected to change.
- `tools/` tests pass, including diagnostic-build's metadata and truncation
  tests, now running on `wasmparser`.
- The `dev` hermetic gate (`dev validate-no-e2e`) passes before the PR.

Deliverables to commit:

- the `nix/packages.nix` and `nix/host-tool.nix` changes, with their assertions;
- the diagnostic-build parser change;
- the ADR draft;
- the ARCHITECTURE.md projection.

## Boundaries

- No narrowing of coverage-sandbox, doctests or perf-producer, and no change to
  traced-run or repository-policy packaging.
- No change to producer outputs, status-file schemas or CLI arguments.
- The step-6 jq/Python rewrite of diagnostic-build is dropped (issue scope).
- Numbered ADRs are not edited in place; the draft amends them.
