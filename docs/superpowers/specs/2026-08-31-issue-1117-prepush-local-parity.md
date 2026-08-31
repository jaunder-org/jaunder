# #1117 — Fast local prepush parity by failure surface

Issue: [#1117](https://github.com/jaunder-org/jaunder/issues/1117). Milestone:
Developer tooling & DX.

## Outcome

`cargo xtask prepush` catches every cheap, cache-friendly host failure surface
that `cargo xtask validate --no-e2e` exercises, while CI and explicit validate
retain authority for environment-specific Nix proof. Documentation names the
remaining asymmetry per surface instead of implying literal parity.

## Load-bearing decisions

- Prepush remains a fast, non-hermetic early-failure gate; it does not become an
  alias for `validate --no-e2e` and invokes no Nix derivation.
- The existing clean-tree refusal and verify-only host/static surface remain the
  prepush entry boundary.
- Prepush already runs the existing xtask/tools non-doc host tests; that
  coverage remains exactly once. A measured warm run on this checkout took 16.9
  seconds for xtask and 1.7 seconds for tools.
- Prepush adds root-workspace doctest execution through exactly
  `cargo test --workspace --doc`, never a package list, plus the same complete
  bidirectional fence census and reconciliation policy used by the hermetic
  doctest gate. Measured execution took 120.1 seconds cold and 10.1 seconds on
  an immediate warm rerun; the lane runs after cache-warming root tests.
- New host Cargo work reuses the repository's existing shared compilation-cache
  environment where the invoked tool supports it.
- The local doctest verdict is authoritative for example execution and fence
  reconciliation on the host toolchain. The Nix doctest producer/gate remains
  authoritative for the pinned sandbox/offline environment.
- Hermetic static proof, Rust coverage/CRAP, wasm browser tests, Elisp coverage,
  and the wasm size budget remain CI/explicit-validate responsibilities because
  no cheap host lane preserves their sandbox, instrumentation, browser, VM, or
  artifact semantics.
- Server-function flow verification remains part of full validate/e2e, not this
  `validate --no-e2e` parity target; #824 already restored that boundary.
- ADR-0029 gains a #1117 supplement recording the cheap-local-versus-hermetic
  hook contract. `docs/ARCHITECTURE.md` and `CONTRIBUTING.md` project the same
  parity table. No new ADR is required because this refines ADR-0029's existing
  fast-prepush decision rather than establishing a separate boundary.

## Acceptance

- `cargo xtask prepush` runs, in one clean-tree command, the verify-only host
  surface, existing xtask/tools non-doc tests, host-native product tests, and
  root doctest execution plus fence reconciliation.
- Focused graph/command tests prove the auxiliary tests remain exactly once, the
  new doctest lane runs exactly once after root tests, clean-tree precedence is
  preserved, the intended cache environment is used, and no Nix command runs.
- A command-level test pins the root invocation to exactly
  `cargo test --workspace --doc`.
- Doctest tests cover a passing population, executable failure, a scanned fence
  with no run entry, a run entry with no scanned fence, duplicated execution,
  misclassified fences, and command failure without relying on a live Nix build.
- Documentation contains a row for every `validate --no-e2e` surface, stating
  its prepush coverage and exact authority/rationale; it does not claim “same
  tests, different environment.”
- The table explicitly marks hermetic static proof, coverage/CRAP, wasm browser
  tests, Elisp coverage, and wasm budget as validate/CI-only.
- The table records full-validate server-function flow verification separately
  so it is not mistaken for a missing no-e2e/prepush surface.
- ADR-0029 contains a #1117 supplement recording the selected cheap local
  surfaces, the hermetic-only surfaces and rationale, and the unchanged CI/full
  validate authority; `docs/ARCHITECTURE.md` projects that decision.
- `cargo xtask check --no-test`, the focused xtask tests, and prepush's own
  graph tests pass.

## Boundaries

- No local substitute whose semantics differ silently from the failure surface
  it claims to cover.
- No coverage, wasm, Elisp-coverage, wasm-budget, Nix, or e2e work in prepush.
- No change to precommit, full validate, CI distribution, clean-tree policy,
  changed-path routing, fail-fast behavior, or exact-tree receipt caching.
