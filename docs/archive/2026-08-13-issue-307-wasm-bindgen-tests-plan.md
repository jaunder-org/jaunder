# Wasm-Only Browser Unit Tests — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run focused tests for irreducible wasm-only browser glue in headless
Chromium through the normal Nix, xtask, and CI gates.

**Architecture:** `client` owns inline `wasm-bindgen-test` tests beside its raw
browser primitives. Cargo compiles them for `wasm32-unknown-unknown` and hands
the artifact to the version-matched `wasm-bindgen-test-runner`. One Linux-only
Nix check owns Chromium/chromedriver selection and execution; xtask invokes that
check as one member of its Nix test group, so local and CI paths share the same
command.

**Tech Stack:** Rust 2024, `wasm-bindgen-test`, `web-sys`, Cargo target runners,
Crane/Nix, Chromium, chromedriver, `cargo xtask`.

**Spec:**
[`2026-08-13-issue-307-wasm-bindgen-tests-spec.md`](2026-08-13-issue-307-wasm-bindgen-tests-spec.md)

## Review header

**Scope — in:** one headless-Chromium localStorage lifecycle test; wasm-target
Cargo test wiring; one hermetic Linux Nix check with explicit browser/driver
paths; xtask check/validate integration and inclusion tests; contributor and
architecture documentation; targeted and full gates.

**Scope — out:** wasm line coverage; Node/Firefox/Safari variants; CSR boot or
Leptos mount tests; migration of every browser primitive; changes to host
coverage, Playwright, ADR-0069, or ADR-0070; a new ADR.

**Tasks:**

1. Add the browser lifecycle test and hermetic Nix runner.
2. Add the Nix check to xtask's shared test group.
3. Document the extension pattern and run the complete gate.

**Key risks/decisions:**

- Force `run_in_browser`; Node cannot satisfy this issue.
- Set `CHROMEDRIVER` and the Chromium capability `binary` to Nix-store paths.
  Browser discovery from the developer host is not accepted.
- Use `--no-sandbox` only inside the already-isolated Nix build sandbox; use
  `--disable-dev-shm-usage` to avoid Chromium's small shared-memory default.
- Capture all storage results, perform final cleanup, then assert. No `?`,
  `unwrap`, or assertion may bypass cleanup.
- Keep pure logic host-tested. This runner is only for irreducible browser APIs.
- The new check is behavioral pass/fail, not wasm coverage.
- No separable concern needs filing and no new architectural decision needs an
  ADR.

## Global constraints

- Preserve ADR-0069: `client` remains raw browser infrastructure with no Jaunder
  domain types and no fake host implementation.
- Preserve ADR-0070: wasm-only gates remain on module wiring, not inside leaf
  production code.
- Keep the root host coverage denominator and reports unchanged.
- `check --no-test` continues to run always-on xtask/tools host tests while
  omitting all Nix test checks.
- Invoke repository tools through `devtool run --`; use the exact commands
  below.
- Before every commit, follow `jaunder-commit`: stage the intended tree, run
  `devtool run -- cargo xtask check`, restage formatter changes, then commit.
- Add no lint suppression and no `Co-Authored-By` trailer.

---

### Task 1: Browser lifecycle test and hermetic Nix runner

**Files:**

- Modify: `client/Cargo.toml` — wasm-target test dependency.
- Modify: `client/src/storage.rs` — inline headless-browser lifecycle test.
- Modify: `.cargo/config.toml` — wasm target test runner.
- Modify: `Cargo.lock` — resolved `wasm-bindgen-test` dependency closure.
- Modify: `flake.nix` — generated WebDriver capabilities and Linux-only check.
- Include in first commit:
  `docs/superpowers/specs/2026-08-13-issue-307-wasm-bindgen-tests.md` and
  `docs/superpowers/plans/2026-08-13-issue-307-wasm-bindgen-tests.md`.

**Interfaces:**

- Cargo runner:

```toml
[target.wasm32-unknown-unknown]
runner = "wasm-bindgen-test-runner"
```

- `client` target-specific development dependency:

```toml
[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
wasm-bindgen-test = "=0.3.71"
```

The exact pin keeps the runner-compatible `0.3.71` release paired with the
locked `wasm-bindgen` `0.2.121`; a caret requirement would select a newer,
schema-incompatible runner.

- Nix check: `checks.x86_64-linux.wasm-tests`; non-Linux check sets do not
  expose it.
- Environment owned by the check:
  `CHROMEDRIVER=${pkgs.chromedriver}/bin/chromedriver` and
  `WASM_BINDGEN_TEST_WEBDRIVER_JSON=<generated-store-path>`.
- Generated `webdriver.json` selects `${pkgs.chromium}/bin/chromium` through
  `goog:chromeOptions.binary` and carries `--no-sandbox` plus
  `--disable-dev-shm-usage`.
- Test command inside the derivation:
  `cargo test -p client --target wasm32-unknown-unknown`.

- [x] **Step 1: Add the failing browser lifecycle test and Cargo wiring**

Add the target runner, target-specific dependency, and an inline test module at
the bottom of `client/src/storage.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{get, remove, set};
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const TEST_KEY: &str = "jaunder-wasm-test-storage-lifecycle";

    #[wasm_bindgen_test]
    fn local_storage_lifecycle() {
        let stale_cleanup = remove(TEST_KEY);
        let initial = get(TEST_KEY);
        let stored = set(TEST_KEY, "stored-value");
        let observed = get(TEST_KEY);
        let removed = remove(TEST_KEY);
        let after_remove = get(TEST_KEY);
        let final_cleanup = remove(TEST_KEY);

        assert!(stale_cleanup.is_ok(), "stale cleanup: {stale_cleanup:?}");
        assert!(matches!(initial, Ok(None)), "initial state: {initial:?}");
        assert!(stored.is_ok(), "store: {stored:?}");
        assert!(matches!(&observed, Ok(Some(value)) if value == "stored-value"));
        assert!(removed.is_ok(), "remove: {removed:?}");
        assert!(
            matches!(after_remove, Ok(None)),
            "state after remove: {after_remove:?}"
        );
        assert!(final_cleanup.is_ok(), "final cleanup: {final_cleanup:?}");
    }
}
```

Do not turn this into a helper abstraction: one test owns one fixed key and one
lifecycle. All browser calls finish before the first assertion, so cleanup does
not depend on wasm panic unwinding.

Run:
`devtool run -- env CHROMEDRIVER=/definitely-missing cargo test -p client --target wasm32-unknown-unknown local_storage_lifecycle`

Expected: **FAIL** before evaluating the test, naming the deliberately missing
driver. This deterministically proves the test artifact reached
`wasm-bindgen-test-runner`, not a Rust compile error or Node execution,
regardless of tools installed on the host.

- [x] **Step 2: Add the explicit WebDriver capability input**

Near the existing version-matched `wasm-bindgen-cli` binding in `flake.nix`,
add:

```nix
wasmTestWebdriverConfig = pkgs.writeText "wasm-bindgen-test-webdriver.json" (
  builtins.toJSON {
    "goog:chromeOptions" = {
      binary = "${pkgs.chromium}/bin/chromium";
      args = [ "--no-sandbox" "--disable-dev-shm-usage" ];
    };
  }
);
```

The file is generated because a committed JSON file cannot name a Nix-store
Chromium path. Keep the browser binary in capabilities rather than relying on
PATH discovery.

- [x] **Step 3: Add the Linux-only Crane test check**

Add `wasm-tests` inside the existing `optionalAttrs pkgs.stdenv.isLinux` check
set, alongside (not inside) the e2e aggregate. Use a target-specific dependency
build so Crane vendors and compiles the wasm test closure:

```nix
wasm-tests = craneLib.cargoTest (
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly (
      commonArgs
      // {
        CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
        cargoExtraArgs = "-p client";
        doCheck = false;
      }
    );
    pname = "jaunder-wasm-tests";
    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
    cargoTestExtraArgs = "-p client";
    nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ wasm-bindgen-cli ];
    CHROMEDRIVER = "${pkgs.chromedriver}/bin/chromedriver";
    CHROMEDRIVER_ARGS = "--verbose";
    WASM_BINDGEN_TEST_WEBDRIVER_JSON = "${wasmTestWebdriverConfig}";
    preCheck = ''
      export XDG_CONFIG_HOME="$TMPDIR/chromium-config"
      mkdir -p "$XDG_CONFIG_HOME"
    '';
  }
);
```

The driver and browser are deliberately absent from `nativeBuildInputs`: their
explicit interpolated paths retain both Nix closures without putting either
executable on PATH. Removing `CHROMEDRIVER` or the capability `binary` therefore
cannot fall back to host/PATH discovery.

The check creates a writable, per-build XDG configuration directory before
starting Chromium; without it, Crashpad exits in the Nix sandbox before a
WebDriver session exists. Verbose driver logging remains silent on success and
preserves the Chromium startup cause on failure.

If Crane requires `doCheck = true` or a build-phase override for the wasm
target, make the smallest API-correct adjustment while preserving the exact
cargo command, target, and explicit binary paths. Do not replace it with
`wasm-pack`, `npx`, or a network installer.

- [x] **Step 4: Prove the hermetic browser check**

Run:
`devtool run -- nix build -L --accept-flake-config .#checks.x86_64-linux.wasm-tests`

Expected: **PASS** — output names one passing wasm test and a headless Chromium
session. Confirm the log identifies the test; a derivation that compiles zero
tests is not green.

Then run:
`devtool run -- nix eval --json .#checks.aarch64-darwin --apply builtins.attrNames`

Expected: **PASS** and output does not contain `wasm-tests`, proving the
platform scope. If the host flake does not expose that system for evaluation,
inspect the Linux `optionalAttrs` placement instead and record that limitation;
do not add a Darwin check.

- [x] **Step 5: Stage, gate, and commit Task 1**

Tick Task 1 steps and stage the five implementation files, the approved spec,
and this plan. Run: `devtool run -- cargo xtask check`

Expected: **PASS** under the pre-change xtask path (the targeted Nix check above
is the Task 1 behavioral proof). If formatting changes files, restage and rerun.
Commit:

```bash
git commit -m "test(wasm): add headless browser unit runner"
```

Expected: pre-commit's `cargo xtask check` passes; commit contains no trailer.

---

### Task 2: Xtask Nix-test orchestration

**Files:**

- Modify: `xtask/src/steps/nix.rs` — shared Nix-test group and wasm check
  adapter.
- Modify: `xtask/src/lib.rs` — route `check` and `validate` through that group.
- Modify: `docs/superpowers/plans/2026-08-13-issue-307-wasm-bindgen-tests.md` —
  progress only.

**Interfaces:**

- Produces `steps::nix::test_checks(result, no_test)`.
- Ordered enabled group: `wasm-tests`, coverage producer/gate/post-processing,
  doctests producer/gate.
- `no_test = true`: performs no Nix builds.
- `no_test = false`: invokes all three in the fixed order above.
- `validate` passes `false` regardless of `--no-e2e`; only e2e remains governed
  by that flag.

- [x] **Step 1: Add failing tests for the Nix-test group**

In `xtask/src/steps/nix.rs`, test a not-yet-defined pure selector:

```rust
#[test]
fn nix_test_check_names_include_wasm_tests() {
    assert!(test_check_names(false).eq(["wasm-tests", "coverage", "doctests"]));
}


#[test]
fn nix_test_check_names_omit_all_for_no_test() {
    assert!(test_check_names(true).next().is_none());
}
```

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml nix_test_check_names`

Expected: **FAIL** — `test_check_names` is undefined.

- [x] **Step 2: Implement one shared Nix-test group**

Add a private enum or fixed descriptor table that is the single source for both
`test_check_names` and execution. Implement public
`test_checks(result: &mut CommandResult, no_test: bool)`:

- return immediately for `no_test`;
- call `build_check("wasm-tests", "wasm-tests")` for the wasm member;
- preserve the existing `coverage(result)` and `doctests(result)` behavior and
  order after it.

The selector used by tests must derive from the same descriptors execution uses;
do not maintain a test-only duplicate list and do not assert over source text.

Replace `lib.rs`'s check-mode `if !no_test { coverage; doctests; }` with
`steps::nix::test_checks(&mut result, no_test)`. Replace validate's two direct
calls with `steps::nix::test_checks(&mut result, false)`.

- [x] **Step 3: Run xtask unit tests**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml nix_test_check_names`

Expected: **PASS** — enabled order and `--no-test` omission are pinned.

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`

Expected: **PASS** — the complete xtask suite accepts the refactor.

- [x] **Step 4: Exercise the static-only command path**

Run: `devtool run -- cargo xtask check --no-test`

Expected: **PASS**. Its result has the always-on `xtask-tests` and `tools-test`
steps but no `wasm-tests`, `nix-coverage`, or `nix-doctests` step.

- [x] **Step 5: Stage, gate, and commit Task 2**

Tick Task 2 and stage `xtask/src/steps/nix.rs`, `xtask/src/lib.rs`, and this
plan. Run: `devtool run -- cargo xtask check`

Expected: **PASS** and includes a successful `wasm-tests` step. If formatting
changes files, restage and rerun. Commit:

```bash
git commit -m "test(xtask): gate wasm browser tests"
```

Expected: pre-commit's `cargo xtask check` passes; commit contains no trailer.

---

### Task 3: Contributor pattern, architecture view, and complete gate

**Files:**

- Modify: `CONTRIBUTING.md` — testing pattern, command table, and Nix-check
  inventory.
- Modify: `docs/ARCHITECTURE.md` — `client` verification and gate composition.
- Modify: `docs/superpowers/plans/2026-08-13-issue-307-wasm-bindgen-tests.md` —
  completion state.

**Documentation contract:**

- `CONTRIBUTING.md` says pure behavior is extracted and host-tested first.
- Inline `wasm_bindgen_test` is reserved for behavior requiring actual browser
  APIs, lives beside its wasm-only owner, and forces `run_in_browser`.
- The targeted proof is
  `nix build -L --accept-flake-config .#checks.x86_64-linux.wasm-tests`; normal
  contributor gates remain `cargo xtask check`/`validate`.
- The command table states that test-enabled `check` and every `validate` run
  include wasm browser tests; `check --no-test` does not.
- The Nix inventory names `checks.x86_64-linux.wasm-tests` and its one Chromium
  runtime.
- `docs/ARCHITECTURE.md` states that `client` browser glue receives focused wasm
  unit tests where appropriate in addition to Playwright flow coverage, while
  host coverage remains unable to instrument wasm.

- [x] **Step 1: Update the contributor testing guide**

Add a compact subsection near the existing unit/e2e layer guidance, then update
the verify-ladder command table and Nix-check inventory. Describe policy and
commands, not the one initial test's implementation.

- [x] **Step 2: Update the architecture view**

Amend the workspace/client paragraph and test-gate composition. Preserve
ADR-0069/0070 boundaries and state explicitly that this is pass/fail execution,
not wasm line coverage.

- [x] **Step 3: Format documentation**

Run:

```bash
devtool run -- prettier -w CONTRIBUTING.md docs/ARCHITECTURE.md docs/superpowers/plans/2026-08-13-issue-307-wasm-bindgen-tests.md
```

Expected: **PASS**; only intended Markdown paragraphs/tables reflow.

- [x] **Step 4: Prove the CI-equivalent and complete gates**

Run: `devtool run -- cargo xtask validate --no-e2e --allow-dirty`

Expected: **PASS** with a successful named `wasm-tests` step and no e2e step,
proving the exact Validate (no e2e) CI path. `clean-tree` is skipped only
because the documentation task is not committed yet.

Then run: `devtool run -- cargo xtask validate --allow-dirty`

Expected: **PASS** — verify-only static checks, headless Chromium wasm unit
test, host coverage/doctests, and all four Playwright e2e combinations pass.

- [x] **Step 5: Stage, gate, and commit Task 3**

Tick Task 3 and stage `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, and this plan.
Run: `devtool run -- cargo xtask check`

Expected: **PASS**. If formatting changes files, restage and rerun. Commit:

```bash
git commit -m "docs(testing): document wasm browser unit tests"
```

Expected: pre-commit's `cargo xtask check` passes; commit contains no trailer.
