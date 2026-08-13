# #307 — gate wasm-only browser unit tests

Issue: [#307](https://github.com/jaunder-org/jaunder/issues/307). Milestone:
Test infrastructure & E2E. The issue has no blockers.

## Problem

Jaunder deliberately places raw browser glue in the wasm-only `client` crate and
wasm-gated `web` component files. Host `nextest` and the instrumented coverage
check cannot execute those lines. The existing wasm clippy step proves they
compile and lint, while Playwright proves whole flows, but there is no
unit-granularity runner for behavior that genuinely requires `window`, the DOM,
or browser storage.

The issue's original references predate ADR-0069 and ADR-0070. The auth-marker
codec is now pure and host-tested; its localStorage binding delegates to
`client::storage`. The first wasm unit test therefore belongs at the raw browser
primitive, not in the domain-facing auth vertical or the full `csr` entry point.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | Standardize on `wasm-bindgen-test` in headless Chromium. Node is not the contract: it cannot exercise the browser APIs that make this code wasm-only.                                                                                                                                                                                                                                                                                                                                             |
| **D2** | The first test lives beside `client::storage` and proves one isolated localStorage lifecycle: absent key, set, get, remove, absent again. It uses a test-owned key and removes stale state before observing initial absence. The test captures every operation and observation without asserting or returning early, unconditionally attempts final removal, then evaluates the captured results and final absence. Cleanup therefore completes before a failed assertion can terminate the test. |
| **D3** | Add `wasm-bindgen-test` as a wasm-target development dependency of `client`. Configure `wasm-bindgen-test-runner` for `wasm32-unknown-unknown` in the workspace Cargo config and force browser execution in the test module with `wasm_bindgen_test_configure!(run_in_browser)`. The test therefore cannot silently fall back to Node because an environment variable is absent.                                                                                                                  |
| **D4** | Add one hermetic Linux-only `checks.x86_64-linux.wasm-tests` Nix check. It runs `cargo test -p client --target wasm32-unknown-unknown` with the pinned Rust toolchain and version-matched `wasm-bindgen-test-runner`. `CHROMEDRIVER` names the Nix-store driver path, while `WASM_BINDGEN_TEST_WEBDRIVER_JSON` names a Nix-generated capability file whose `goog:chromeOptions.binary` is the Nix-store Chromium path. Non-Linux systems do not expose this check.                                |
| **D5** | Inside `check`'s existing `if !no_test` Nix-test block, xtask invokes `wasm-tests` beside coverage and doctests. `cargo xtask validate` always invokes it, including under `validate --no-e2e`, so the existing Validate CI job gates it without a new workflow or duplicated command. The result has its own `wasm-tests` step rather than being folded into host coverage or Playwright e2e.                                                                                                    |
| **D6** | Document the extension pattern in `CONTRIBUTING.md`: first extract pure logic to host-tested code; use inline `wasm_bindgen_test` tests only for irreducible browser behavior; force the browser runtime; run the targeted Nix check or the normal xtask gate. Update the Nix-check inventory and architecture view to stop describing browser glue as e2e-only.                                                                                                                                  |
| **D7** | This is behavioral test execution, not wasm line coverage. The existing host coverage denominator and policy remain unchanged; the new check reports test pass/fail only.                                                                                                                                                                                                                                                                                                                         |
| **D8** | No ADR is needed. The change fills the unit-test gap already identified by issue #307 while preserving ADR-0069's browser-glue boundary and ADR-0070's host/wasm file split. The runner and gate are reversible tooling choices, recorded in the architecture view and contributor guide.                                                                                                                                                                                                         |

## Observable flow

1. `cargo xtask check` or `cargo xtask validate --no-e2e` reaches the dedicated
   `wasm-tests` Nix check.
2. Nix builds the `client` test target for `wasm32-unknown-unknown` from
   vendored inputs, selects its Nix-store chromedriver and Chromium paths, and
   starts that exact pair.
3. `wasm-bindgen-test-runner` serves and executes the test module in headless
   Chromium.
4. The localStorage lifecycle test passes or the named `wasm-tests` xtask step
   fails with the Nix diagnostic bundle.
5. `cargo xtask check --no-test` continues running the always-on xtask/tools
   host tests but omits the Nix coverage, doctest, and `wasm-tests` checks.

## Acceptance criteria

- **AC1 — genuine browser test.** At least one `#[wasm_bindgen_test]` executes
  in headless Chromium and exercises the real `web_sys` localStorage binding. A
  Node-only or compile-only test does not satisfy this criterion.

- **AC2 — storage contract and isolation.** The initial test removes stale
  state, observes an absent test-owned key, stores a value through
  `client::storage::set`, reads the same value through `get`, removes it through
  `remove`, and observes absence again. It captures those results without
  asserting or returning early, unconditionally attempts final cleanup, and only
  then asserts the captured lifecycle and final absence.

- **AC3 — forced runtime and binaries.** The test module forces
  `run_in_browser`, the workspace target runner is `wasm-bindgen-test-runner`,
  `CHROMEDRIVER` is an existing Nix-store executable, and the WebDriver
  capability file selects an existing Nix-store Chromium executable. Removing
  either provisioned binary makes the check fail rather than falling back to
  Node or a host-installed browser.

- **AC4 — hermetic Linux Nix check.** `checks.x86_64-linux.wasm-tests` succeeds
  without downloading a browser, WebDriver, Rust tool, or crate during the
  build. The wasm-bindgen CLI/runner version remains compatible with the
  workspace's locked wasm-bindgen version; non-Linux check sets do not advertise
  `wasm-tests`.

- **AC5 — one gate path.** Xtask invokes the Nix check rather than restating its
  cargo/browser command. `check`, `validate --no-e2e`, and full `validate`
  include a named `wasm-tests` result; `check --no-test` omits it.

- **AC6 — CI coverage.** The existing Validate (no e2e) workflow reaches the new
  check through `cargo xtask validate --no-e2e`; no second GitHub Actions
  implementation is introduced.

- **AC7 — documented extension pattern.** `CONTRIBUTING.md` tells contributors
  where wasm-only tests live, when they are appropriate, how browser execution
  is forced, and how to run them. Its Nix-check inventory includes `wasm-tests`.
  `docs/ARCHITECTURE.md` states that irreducible browser glue has focused
  headless-browser unit tests in addition to e2e coverage.

- **AC8 — regression proof.** A targeted build of the Nix `wasm-tests` check
  passes; xtask tests pin inclusion/skipping semantics; `cargo xtask check` and
  the full shipping gate pass.

## Non-goals

- Measuring wasm line coverage or changing the host coverage gate.
- Unit-testing pure codecs, state transitions, or formatting in wasm; those
  remain host-tested after extraction.
- Booting `csr::main`, mounting the full Leptos app, or replacing Playwright
  flow coverage.
- Adding Firefox/Safari/Node variants or a browser matrix for wasm unit tests.
- Migrating every existing browser primitive in this issue.
- Changing ADR-0069's crate layering or ADR-0070's module-placement rules.

## Risks

- **Tool/version drift.** `wasm-bindgen-test-runner` must match the crate-side
  wasm-bindgen toolchain closely enough to process the generated test module;
  the Nix override and Cargo lock must remain aligned.
- **Browser/driver mismatch.** Chromium and chromedriver must come from a
  compatible Nix package set and be selected explicitly, not whichever binaries
  happen to be on a developer host.
- **Hidden state.** localStorage survives within a browser profile. A unique,
  test-owned key plus cleanup keeps the first test isolated and establishes the
  pattern for later storage tests.
- **Gate cost.** Headless Chromium adds startup cost to the non-e2e validation
  lane. Keeping one dedicated cached Nix derivation avoids multiplying that cost
  across the four Playwright combinations.
