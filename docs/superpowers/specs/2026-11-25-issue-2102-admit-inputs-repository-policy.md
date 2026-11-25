# Issue #2102: move admit-inputs into repository-policy, retire csr-prerequisite

## Outcome

`dev test-local` and `dev e2e-local` admit untracked flake inputs with an
`admit-inputs` executable built from `packages.repository-policy`, and
`test-local` no longer runs `csr-prerequisite`. `tools/host-proof` loses its two
CSR/admission executables and its Rust OpenTelemetry writer, so `dev` is the
only writer of its span-record format.

## Approach and tradeoffs

- **admit-inputs moves to its library's owner.** `tools/repository-policy` gains
  `src/bin/admit-inputs.rs`: the same `admit-inputs <source-admits>` command
  line over the unchanged `repository_policy::checkout::admit_untracked_inputs`
  on the working directory. Argv is parsed by hand like the package's other
  executables, with no clap dependency: exactly one argument not starting with
  `-`, otherwise a usage line and exit 2 (`--help` no longer exits 0). A refusal
  still exits 1 with `admit-inputs: <reason>` on stderr.
  `nix/repository-policy.nix` lists it among `bins`, and `dev` resolves it as
  `repository_policy("admit-inputs")`. Its process test moves unchanged in
  substance to `tools/repository-policy/tests/admit_inputs.rs`.
- **csr-prerequisite is retired.** `server/build.rs` already runs the same
  validators on a declared `JAUNDER_CSR_BUNDLE_DIR` and fails closed: the Vite
  manifest (`host::vite_manifest::csr_outputs`), the system-artifact inventory
  and the rendered `index.html` (`build_staging`). `test-local` instead sets
  `JAUNDER_CSR_BUNDLE_DIR` to `os.path.realpath(bundle)`. The canonicalization
  keeps today's value: the built bundle is reached through a fixed GC-root
  symlink, and `build.rs` tracks the variable (`rerun-if-env-changed`), so
  passing the resolved store path makes a new bundle a new value rather than
  relying on directory-change detection through the symlink. `realpath` also
  makes a relative override absolute, which matters because build scripts run in
  `server/`. A missing explicit directory resolves to an absolute path that
  `build.rs` refuses. The first Cargo step, `image-worker`
  (`cargo build -p jaunder`), compiles `build.rs`, so an invalid bundle still
  fails before `nextest-build`/`nextest` start.
- **Removed with it:** `host-proof`'s `csr-prerequisite` binary and
  `steps/csr_prerequisite.rs`, `span_records.rs` and its test
  `tests/span_records.rs`, the moved `admit-inputs` binary and test, and the
  `opentelemetry`/`opentelemetry_sdk` dependencies if nothing else uses them.
  `nix/packages.nix` drops both binaries from `hostProofBin`. The
  corruption-matrix unit tests in `csr_prerequisite.rs` go too; the same cases
  are already covered by `server/src/build_staging.rs` (no `index.html`),
  `host/src/vite_manifest.rs` (manifest, outputs) and the host system-artifact
  inventory tests.
- **Side effect:** neither workflow builds `packages.host-proof` any more.
  `test-local` and `e2e-local` build `packages.repository-policy`, whose closure
  is narrower (no `host`); with `JAUNDER_CSR_BUNDLE_DIR` set, `test-local`
  builds no host tool at all.

Accepted losses (from the issue): the separately named `csr-prerequisite` step
and its early failure (the `image-worker` step fails instead, with the same
validators), the two `host-proof.csr-*` sub-phase spans, and any Rust child
joining a `dev` trace. Also, validation now runs only when Cargo reruns
`build.rs`, not on every invocation: an explicit, mutable override corrupted in
place with its mtimes preserved would go undetected. That is acceptable for a
cooperating developer's own directory.

## Acceptance

Proof to run:

- `cargo test -p repository-policy --test admit_inputs` passes: a clean checkout
  and a harmless untracked file are admitted; an untracked declared input exits
  1 naming it.
- `tools/dev` pytest: `test-local` has no `csr-prerequisite` step; the Cargo
  steps receive the resolved bundle root, for the built bundle and for an
  explicit override (a missing explicit root is passed on as an absolute path,
  and no tool build runs); admission builds and runs the repository-policy
  binary; `e2e-local` admits through the same binary and builds
  `repository-policy`, not `host-proof`. The fixtures that hard-code the old
  layout change with it: `conftest.py` (tool packages, the fake
  `csr-prerequisite`), `test_gates.py` (`dev check`'s `test-local` sequence),
  `test_test_local.py` and `test_e2e_local.py` (the fake `nix` build exemption).
- Real `dev test-local -p storage <one test>` with `JAUNDER_CSR_BUNDLE_DIR` set
  to a writable copy of a real bundle (`cp -rL .xtask/gcroots/csr-bundle`) with
  `index.html` removed fails in step `image-worker`, and no `nextest` step runs;
  with no override it passes.
- `dev check` green.

Deliverables to commit: the code moves and removals above; ADR amendments
(ADR-0246 admits `admit-inputs` as a fixed repository-policy operation; ADR-0251
drops the Rust writer and `csr-prerequisite`; ADR-0258 and ADR-0247 update the
package/executable lists) projected into `docs/ARCHITECTURE.md`. Docs that
describe the retired step or the Rust writer: CONTRIBUTING (tool list, the
`test-local` steps and explicit-override note, the trace section),
`docs/invocation-traces.md`, `docs/observability.md`,
`tools/repository-policy/README.md`, host-proof's crate doc, and the
`tools/dev/dev/trace.py` and `tools/dev/tests/test_records.py` docstrings.

## Boundaries

- `admit_untracked_inputs` and its Nix predicate are unchanged.
- host-proof's in-crate `checkout.rs` admission tests and its remaining
  executables stay; retiring `tools/host-proof` is the follow-up issue.
- No change to `build.rs` validation or to `e2e-local`'s flow beyond which
  package supplies `admit-inputs`.
