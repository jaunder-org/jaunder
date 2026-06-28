# E2E {backend}×{browser} Matrix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop running chromium and firefox serially inside each e2e VM; run every `{backend}×{browser}` combo as its own derivation, fanned out across CI runners, to cut e2e wall-clock from ~17.3 min toward ~the slowest single combo (~10.6 min) with crisp per-combo failure reporting.

**Architecture:** Parameterize the two NixOS-VM e2e check builders by browser so each produces one single-browser VM; generate the 4 warm checks + 4 cold diagnostic packages from one combo list; add a thin `cargo xtask e2e <backend> <browser>` subcommand that builds one combo through xtask's diagnostic-preserving wrapper; restructure CI into `validate-no-e2e` + a 4-way e2e matrix + an `e2e-gate` aggregator. Local `cargo xtask validate` still builds the `e2e-checks` aggregate (now auto-including all 4 combos), parallel by Nix concurrency.

**Tech Stack:** Nix flakes + crane, `pkgs.testers.nixosTest`, Playwright (chromium/firefox projects), Rust (xtask, clap), GitHub Actions, Cachix.

## Global Constraints

- **Backend parity:** the change must keep both sqlite and postgres covered by both browsers (full 2×2). Copied from spec: "Preserve today's exact e2e coverage."
- **Diagnostics invariant (ADR-0032 / #48):** a server panic must fail the e2e derivation, and the journal + build log must be recoverable on failure. Matrix jobs must go through xtask's `build_check` + `rescue_diagnostics`, never a raw `nix build`.
- **xtask is host-only:** Nix derivations never invoke `cargo xtask`; xtask invokes `nix build`. The new `e2e` subcommand runs on the host/runner only.
- **No Co-Authored-By trailers** in commits (project override).
- **Trace ids:** preserve the historical per-combo OTel trace ids (sqlite-chromium=all-`1`, sqlite-firefox=all-`2`, postgres-chromium=all-`3`, postgres-firefox=all-`4`).
- **Cachix pushFilter** stays `jaunder-coverage|jaunder-e2e` (e2e derivations never cached green).
- Per-task gate while iterating: `cargo xtask check --no-test` (clippy + fmt). Flake/CI tasks add explicit `nix eval` / `actionlint` checks since check --no-test doesn't cover them. Final gate before ship: `cargo xtask validate`.

---

### Task 1: Flake — parameterize e2e checks by browser; generate 4 warm checks + 4 cold packages

**Files:**
- Modify: `flake.nix` — `mkE2eSqliteCheck` (~549-658), `mkE2ePostgresCheck` (~660-802), the `packages` block (~806-830), the `checks` block (~846-867).

**Interfaces:**
- Produces (consumed by Tasks 2-4): flake check attrs `checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}` and package attrs `packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-cold`. The `checks.x86_64-linux.e2e` aggregate auto-includes all four warm checks (its `hasPrefix "e2e-"` filter already does this).

- [x] **Step 1: Add `browser`, `traceId`, `traceParent` params and collapse `mkE2eSqliteCheck` to a single run**

In `mkE2eSqliteCheck`, change the argument set and replace the two browser blocks (lines ~619-650) with one. New signature:

```nix
        mkE2eSqliteCheck =
          {
            checkName,
            browser,
            traceId,
            traceParent,
            warmupEnv ? "",
          }:
          pkgs.testers.nixosTest {
            name = checkName;
            # nodes.machine unchanged …
```

Replace the comment + the two `seed_db()` + `machine.succeed(...)` blocks (the chromium block and the firefox block) with a single run:

```nix
              # Seed a fresh DB and run the one browser this derivation targets.
              # Browsers run as separate derivations (one VM each) so their state
              # mutations cannot interfere; that also lets CI fan them out.
              seed_db()
              machine.succeed(
                "cd /tmp/e2e"
                + " && PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}"
                + " PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"
                + "${warmupEnv}"
                + " JAUNDER_MAIL_CAPTURE_FILE=/var/lib/jaunder/mail.jsonl"
                + " JAUNDER_WEBSUB_CAPTURE_FILE=/var/lib/jaunder/websub.jsonl"
                + " JAUNDER_E2E_TRACE_ID=${traceId}"
                + " JAUNDER_E2E_TRACEPARENT=${traceParent}"
                + " JAUNDER_E2E_OTLP_HTTP_ENDPOINT=http://127.0.0.1:4318/v1/traces"
                + " ${pkgs.nodejs}/bin/node node_modules/.bin/playwright test"
                + " --config playwright.nix.config.js --project ${browser}"
              )
```

Leave the otel-collector stop, `copy_from_vm(... "otel-traces-sqlite.jsonl")`, and `${e2ePanicGate "sqlite"}` lines as-is.

- [x] **Step 2: Same collapse for `mkE2ePostgresCheck`**

Identical change to `mkE2ePostgresCheck`: add `browser, traceId, traceParent` to the args; keep the `create-pg-db` prologue and the `seed_db()` definition (TRUNCATE form); replace the two browser blocks (lines ~763-794) with the single run block above but using the postgres `seed_db()` (already in scope) and leaving `copy_from_vm(... "otel-traces-postgres.jsonl")` + `${e2ePanicGate "postgres"}`.

- [x] **Step 3: Add the combo list + generators in the top-level `let`**

Immediately after `mkE2ePostgresCheck` (before the `in` at ~804), add:

```nix
        # All e2e {backend}×{browser} combos. backend picks the VM builder;
        # browser picks the Playwright --project; traceDigit gives each combo a
        # distinct OTel trace id (the 1/2/3/4 mapping preserves the historical
        # per-combo ids). Add a row here and the warm checks, the cold diagnostic
        # packages, and the `e2e-checks` aggregate all extend automatically.
        e2eCombos = [
          { backend = "sqlite";   browser = "chromium"; traceDigit = "1"; }
          { backend = "sqlite";   browser = "firefox";  traceDigit = "2"; }
          { backend = "postgres"; browser = "chromium"; traceDigit = "3"; }
          { backend = "postgres"; browser = "firefox";  traceDigit = "4"; }
        ];

        mkE2eCombo =
          {
            backend,
            browser,
            traceDigit,
            nameSuffix ? "",
            warmupEnv ? "",
          }:
          let
            mk = if backend == "sqlite" then mkE2eSqliteCheck else mkE2ePostgresCheck;
            traceId = pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 32);
            traceParent =
              "00-${traceId}-${pkgs.lib.concatStrings (pkgs.lib.genList (_: traceDigit) 16)}-01";
          in
          mk {
            checkName = "jaunder-e2e-${backend}-${browser}${nameSuffix}";
            inherit browser traceId traceParent warmupEnv;
          };

        # attr name -> warm check, e.g. { "e2e-sqlite-chromium" = <drv>; ... }
        e2eWarmChecks = pkgs.lib.listToAttrs (
          map (c: {
            name = "e2e-${c.backend}-${c.browser}";
            value = mkE2eCombo (c // { warmupEnv = " JAUNDER_E2E_WARMUP=1"; });
          }) e2eCombos
        );

        # Cold-cache variants (no warmup): same combos as the warm checks but the
        # first navigation of each test pays the full cold WASM download + init.
        # NOT part of the gate — built on demand by
        # `scripts/run-e2e-trace-analysis --cold` to capture cold-cache OTel
        # navigation traces for performance diagnostics (see docs/observability.md).
        e2eColdPackages = pkgs.lib.listToAttrs (
          map (c: {
            name = "e2e-${c.backend}-${c.browser}-cold";
            value = mkE2eCombo (c // { nameSuffix = "-cold"; });
          }) e2eCombos
        );
```

- [x] **Step 4: Wire the warm checks into `checks` (replace the two named e2e checks)**

Replace lines ~848-856 (`e2e-sqlite = …;` and `e2e-postgres = …;`) so the isLinux checks block reads:

```nix
        checks =
          pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
            e2eWarmChecks
            // {
              # The single e2e gate `cargo xtask validate` builds. `e2e-checks`
              # aggregates every `checks.e2e-*` combo (now 4); they are independent
              # derivations realized in parallel up to the host `max-jobs`.
              e2e = self.packages.${system}.e2e-checks;
            }
          )
          // {
            clippy = craneLib.cargoClippy (
              # … unchanged …
```

- [x] **Step 5: Wire the cold packages into `packages` (replace the two `-cold` defs)**

Replace lines ~811-817 (`e2e-sqlite-cold = …;` and `e2e-postgres-cold = …;`) by merging `e2eColdPackages`. The `packages` block becomes:

```nix
        packages = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux (
          {
            jaunder = jaunderBin;
            site = site;
            devtool = devtoolBin;

            # The e2e aggregate: a symlinkJoin of every `e2e-*` check, exposed as
            # `checks.e2e` and built by `cargo xtask validate`. (unchanged comment)
            e2e-checks = pkgs.symlinkJoin {
              name = "jaunder-e2e-checks";
              paths = builtins.attrValues (
                pkgs.lib.filterAttrs (name: _: pkgs.lib.hasPrefix "e2e-" name) self.checks.${system}
              );
            };
          }
          // e2eColdPackages
        );
```

- [x] **Step 6: Verify the flake evaluates and exposes the 8 attrs**

Run: `nix eval .#checks.x86_64-linux --apply 'cs: builtins.filter (n: builtins.match "e2e-.*" n != null) (builtins.attrNames cs)'`
Expected: `[ "e2e-postgres-chromium" "e2e-postgres-firefox" "e2e-sqlite-chromium" "e2e-sqlite-firefox" ]`

Run: `nix eval .#packages.x86_64-linux --apply 'ps: builtins.filter (n: builtins.match "e2e-.*-cold" n != null) (builtins.attrNames ps)'`
Expected: the 4 `-cold` names.

- [ ] **Step 7: Build one warm combo end-to-end to prove a single-browser VM runs green**

Run: `nix build -L --accept-flake-config .#checks.x86_64-linux.e2e-sqlite-chromium`
Expected: builds green; the log shows exactly one `playwright test --project chromium` run (not two) and the zero-panic gate passes. (~10 min; this is the real proof the collapse works.)

- [ ] **Step 8: Commit**

```bash
git add flake.nix
git commit -m "feat(e2e): parameterize VM checks by browser into a {backend}×{browser} matrix

Collapse the serial two-browser run in each e2e VM to one browser per
derivation; generate the 4 warm checks + 4 cold packages from one combo
list. The e2e-checks aggregate auto-includes all four. Preserves the
historical per-combo OTel trace ids.

Refs #129"
```

---

### Task 2: xtask — add `cargo xtask e2e <backend> <browser>` (diagnostic-preserving single-combo build)

**Files:**
- Modify: `xtask/src/lib.rs` — `Command` enum (~26), `name()` match (~77-80), `run()` dispatch (~87-123), add a parse test (~330).
- Modify: `xtask/src/steps/nix.rs` — add `e2e_combo` (near `e2e`, ~68-87).

**Interfaces:**
- Consumes (from Task 1): the `checks.x86_64-linux.e2e-<backend>-<browser>` attrs.
- Produces (consumed by Task 4): CLI `cargo xtask e2e <backend> <browser>` building one combo through `build_check` (so `.xtask/diagnostics/e2e-<backend>-<browser>/build.log` + rescued journal exist on failure) and writing `.xtask/last-result.json`.

- [ ] **Step 1: Write the failing parse test**

Add to the `#[cfg(test)] mod tests` in `xtask/src/lib.rs`:

```rust
    #[test]
    fn e2e_combo_parses_backend_and_browser() {
        let cli = Cli::try_parse_from(["xtask", "e2e", "postgres", "firefox"]).unwrap();
        match cli.command {
            Command::E2e { backend, browser } => {
                assert_eq!(backend, E2eBackend::Postgres);
                assert_eq!(browser, E2eBrowser::Firefox);
            }
            _ => panic!("expected e2e"),
        }
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p xtask e2e_combo_parses_backend_and_browser`
Expected: FAIL to compile — `Command::E2e` / `E2eBackend` / `E2eBrowser` undefined.

- [ ] **Step 3: Add the enums + command variant**

In `xtask/src/lib.rs`, add value enums and the variant. Use clap `ValueEnum` so invalid values are rejected:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBrowser {
    Chromium,
    Firefox,
}

impl E2eBackend {
    fn as_str(self) -> &'static str {
        match self {
            E2eBackend::Sqlite => "sqlite",
            E2eBackend::Postgres => "postgres",
        }
    }
}

impl E2eBrowser {
    fn as_str(self) -> &'static str {
        match self {
            E2eBrowser::Chromium => "chromium",
            E2eBrowser::Firefox => "firefox",
        }
    }
}
```

Add to `enum Command`:

```rust
    /// Build ONE e2e VM check (a {backend}×{browser} combo) through the same
    /// diagnostic-preserving wrapper `validate` uses. For CI matrix fan-out;
    /// not part of `check`/`validate`. Runs on the host only.
    E2e {
        #[arg(value_enum)]
        backend: E2eBackend,
        #[arg(value_enum)]
        browser: E2eBrowser,
    },
```

Add to the `name()` match: `Command::E2e { .. } => "e2e",`.

- [ ] **Step 4: Add the dispatch arm in `run()`**

```rust
        Command::E2e { backend, browser } => {
            let label = format!("e2e-{}-{}", backend.as_str(), browser.as_str());
            let mut result = CommandResult::new(&label);
            steps::nix::e2e_combo(&mut result, backend.as_str(), browser.as_str());
            result
        }
```

(Match the surrounding pattern for how other arms build/return `CommandResult` and how the sidecar/`last-result.json` is written — mirror `Command::Validate`.)

- [ ] **Step 5: Add `e2e_combo` in `steps/nix.rs`**

```rust
/// Build a single e2e {backend}×{browser} combo check via `build_check` (so the
/// `nix build -L --keep-failed` log + `rescue_diagnostics` failure bundle land in
/// `.xtask/diagnostics/e2e-<backend>-<browser>/`), then copy that combo's journal
/// into the canonical diagnostics dir. Used by CI's e2e matrix.
pub fn e2e_combo(result: &mut CommandResult, backend: &str, browser: &str) {
    let check = format!("e2e-{backend}-{browser}");
    let step_name = format!("nix-{check}");
    result.push(build_check(&step_name, &check));
    copy_journals_between(
        std::path::Path::new(&format!(".xtask/gcroots/{check}")),
        std::path::Path::new(&format!(".xtask/diagnostics/{check}")),
    );
}
```

- [ ] **Step 6: Run the parse test + the gate**

Run: `cargo test -p xtask e2e_combo_parses_backend_and_browser`
Expected: PASS.
Run: `cargo xtask check --no-test`
Expected: clippy + fmt clean.

- [ ] **Step 7: Smoke-test the command builds the combo with diagnostics**

Run: `cargo xtask e2e sqlite chromium`
Expected: exit 0; `.xtask/diagnostics/e2e-sqlite-chromium/build.log` exists. (Reuses the warm cache from Task 1; ~fast on a warm store.)

- [ ] **Step 8: Commit**

```bash
git add xtask/src/lib.rs xtask/src/steps/nix.rs
git commit -m "feat(xtask): add e2e <backend> <browser> single-combo build for CI fan-out

Builds one e2e check through build_check so the matrix jobs preserve the
ADR-0032/#48 diagnostic-visibility (build.log + rescued journal on
failure) instead of a raw nix build.

Refs #129"
```

---

### Task 3: trace-analysis script + docs — per-browser cold variants

**Files:**
- Modify: `scripts/run-e2e-trace-analysis` (arg parsing ~22-61, attr selection ~120-145).
- Modify: `CONTRIBUTING.md` (~225-236), `docs/observability.md` (~65-75).

**Interfaces:**
- Consumes (from Task 1): `packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-cold` and `checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}`.

- [ ] **Step 1: Add a `--browser` selector to `parseArgs`**

In `scripts/run-e2e-trace-analysis`, add a `browser` option (default `null` = both). Add to the `parseArgs` loop:

```js
        if (arg === "--browser") {
            i += 1;
            if (i >= argv.length) {
                throw new Error("--browser requires a value");
            }
            if (argv[i] !== "chromium" && argv[i] !== "firefox") {
                throw new Error("--browser must be chromium or firefox");
            }
            browser = argv[i];
            continue;
        }
```

Declare `let browser = null;` next to `let cold = false;`, return it in the object, and add `[--browser chromium|firefox]` to `usage()`.

- [ ] **Step 2: Build the per-browser combo attrs**

Replace the `sqliteAttr`/`postgresAttr` block (~120-128) with a loop over the selected browsers and both backends, building each and collecting its trace file:

```js
    const browsers = parsed.browser ? [parsed.browser] : ["chromium", "firefox"];
    const ns = parsed.cold ? "packages" : "checks";
    const suffix = parsed.cold ? "-cold" : "";

    const traceFiles = [];
    for (const backend of ["sqlite", "postgres"]) {
        for (const browser of browsers) {
            const attr = `${ns}.x86_64-linux.e2e-${backend}-${browser}${suffix}`;
            const out = buildCheck(attr);
            const traceFile = path.join(out, `otel-traces-${backend}.jsonl`, "otel-traces.jsonl");
            if (!fs.existsSync(traceFile)) {
                throw new Error(`trace file not found: ${traceFile}`);
            }
            traceFiles.push(traceFile);
        }
    }
```

Then pass `...traceFiles` to the analyzer instead of `sqliteTrace, postgresTrace`:

```js
    analyzerArgs.push(...traceFiles);
```

(Delete the now-unused `sqliteTrace`/`postgresTrace`/`sqliteOut`/`postgresOut` lines and the `for (const traceFile of [...])` existence loop they fed.)

- [ ] **Step 3: Verify arg parsing + attr resolution**

Run: `scripts/run-e2e-trace-analysis --help`
Expected: usage now lists `--browser`.
Run: `node -e "require('child_process')" ` is not needed; instead resolve attrs without building:
Run: `nix eval .#packages.x86_64-linux.e2e-sqlite-firefox-cold.name`
Expected: `"jaunder-e2e-sqlite-firefox-cold"` — confirms the script's constructed attr path exists.

- [ ] **Step 4: Update the docs**

In `CONTRIBUTING.md` replace the two `e2e-*-cold` package bullets (~225-226) and the two `nix build` examples (~235-236) with the 4 per-browser cold packages, and note the `--browser` flag near the `--cold` mention (~141). In `docs/observability.md` (~65-75) update the `--cold` description to say it runs the per-browser cold packages and mention `--browser`.

- [ ] **Step 5: Commit**

```bash
git add scripts/run-e2e-trace-analysis CONTRIBUTING.md docs/observability.md
git commit -m "feat(e2e): per-browser cold trace-analysis variants

run-e2e-trace-analysis gains --browser; --cold now builds the per-browser
cold packages. Docs updated.

Refs #129"
```

---

### Task 4: CI — restructure ci.yml into validate-no-e2e + e2e matrix + e2e-gate

**Files:**
- Modify: `.github/workflows/ci.yml` (full rewrite of `jobs`).

**Interfaces:**
- Consumes: `cargo xtask validate --no-e2e` (existing), `cargo xtask e2e <backend> <browser>` (Task 2).

- [ ] **Step 1: Rewrite `jobs`**

Replace the single `validate` job with three jobs. `validate-no-e2e` keeps the existing steps but runs `cargo xtask validate --no-e2e` and uploads `validate-diagnostics`. The `e2e` matrix job runs `cargo xtask e2e <backend> <browser>` and uploads per-combo diagnostics. `e2e-gate` aggregates.

```yaml
jobs:
  validate-no-e2e:
    name: Validate (no e2e)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: cachix/install-nix-action@v31
        with:
          extra_nix_config: |
            experimental-features = nix-command flakes
          github_access_token: ${{ secrets.GITHUB_TOKEN }}
      - uses: cachix/cachix-action@v17
        with:
          name: jaunder-org
          authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}
          pushFilter: "jaunder-coverage|jaunder-e2e"
      - name: Cache xtask host build
        uses: actions/cache@v6
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            xtask/target
          key: xtask-${{ runner.os }}-${{ hashFiles('xtask/Cargo.lock', 'flake.lock') }}
          restore-keys: |
            xtask-${{ runner.os }}-
      - name: Validate (static + clippy + coverage, via xtask)
        run: nix develop .#ci --accept-flake-config -c cargo xtask validate --no-e2e
      - name: Upload coverage diagnostics
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: validate-diagnostics
          path: |
            .xtask/diagnostics/
            .xtask/gcroots/coverage/status.json
            .xtask/gcroots/coverage/diagnostics/
            .xtask/last-result.json
          if-no-files-found: ignore
          retention-days: 14

  e2e:
    name: e2e (${{ matrix.backend }}/${{ matrix.browser }})
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        backend: [sqlite, postgres]
        browser: [chromium, firefox]
    steps:
      - uses: actions/checkout@v7
      - uses: cachix/install-nix-action@v31
        with:
          extra_nix_config: |
            experimental-features = nix-command flakes
          github_access_token: ${{ secrets.GITHUB_TOKEN }}
      - uses: cachix/cachix-action@v17
        with:
          name: jaunder-org
          authToken: ${{ secrets.CACHIX_AUTH_TOKEN }}
          pushFilter: "jaunder-coverage|jaunder-e2e"
      - name: Cache xtask host build
        uses: actions/cache@v6
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            xtask/target
          key: xtask-${{ runner.os }}-${{ hashFiles('xtask/Cargo.lock', 'flake.lock') }}
          restore-keys: |
            xtask-${{ runner.os }}-
      - name: e2e (${{ matrix.backend }}/${{ matrix.browser }}, via xtask)
        run: nix develop .#ci --accept-flake-config -c cargo xtask e2e ${{ matrix.backend }} ${{ matrix.browser }}
      - name: Upload e2e diagnostics
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: e2e-diagnostics-${{ matrix.backend }}-${{ matrix.browser }}
          path: |
            .xtask/diagnostics/
            .xtask/last-result.json
          if-no-files-found: ignore
          retention-days: 14

  e2e-gate:
    name: e2e gate
    runs-on: ubuntu-latest
    needs: e2e
    if: always()
    steps:
      - name: Require all e2e matrix jobs to pass
        run: |
          result="${{ needs.e2e.result }}"
          echo "e2e matrix result: $result"
          test "$result" = "success"
```

- [ ] **Step 2: Lint the workflow**

Run: `actionlint .github/workflows/ci.yml` (if available; else `nix run nixpkgs#actionlint -- .github/workflows/ci.yml`).
Expected: no errors. If actionlint is unavailable, validate YAML parses: `nix eval --impure --expr 'builtins.fromJSON (builtins.toJSON 0)'` is not a substitute — instead confirm structure by eye against this plan.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: fan e2e out to a {backend}×{browser} matrix + e2e-gate

Split the single validate job into validate-no-e2e + a 4-way e2e matrix
(via cargo xtask e2e) + an e2e-gate aggregator, so browsers run in
parallel across runners. Branch protection must require validate-no-e2e
+ e2e-gate (done at landing).

Refs #129"
```

---

### Task 5: Docs — CLAUDE.md + stale-comment cleanup

**Files:**
- Modify: `CLAUDE.md` (`# xtask` section, the table + invariant describing CI).
- Modify: `flake.nix` / `xtask/src/steps/nix.rs` — any comment still saying CI runs both VM checks in one `nix build` / "via xtask validate".

- [ ] **Step 1: Update CLAUDE.md `# xtask` section**

Adjust the prose/table that states CI runs `nix develop .#ci -c cargo xtask validate` as one command: CI now runs `cargo xtask validate --no-e2e` in one job plus a `{backend}×{browser}` e2e matrix (each job `cargo xtask e2e <backend> <browser>`) aggregated by `e2e-gate`; `cargo xtask validate` remains the full local gate. Reference ADR-0033.

- [ ] **Step 2: Fix stale comments**

Run: `rg -n 'both backend VM checks|both VM checks|via .cargo xtask validate.|runs both' flake.nix xtask/src .github`
Update each hit that now misdescribes the flow (e.g. the ci.yml `max-jobs`/"both VM checks" comment block, the `e2e()`/`e2e-checks` comments) to reflect: local `validate` builds the aggregate (parallel via max-jobs); CI fans out per combo.

- [ ] **Step 3: Verify no stale references remain**

Run: `rg -n 'cargo xtask validate$|both backend VM checks' CLAUDE.md flake.nix .github/workflows/ci.yml`
Expected: only intended mentions (e.g. "local full gate") remain.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md flake.nix xtask/src
git commit -m "docs: describe the fanned-out e2e CI model (ADR-0033)

Refs #129"
```

---

### Task 6 (landing, non-code): branch-protection required checks

Handled in `jaunder-ship` / by the maintainer, not a code commit. After the PR's first CI run produces the new check names, update the repo ruleset so required status checks are **`validate-no-e2e`** + **`e2e gate`** (and remove the old `Validate`). Without this the gate is silently absent. Verify on the PR that merge is blocked until both pass.

---

## Self-Review

**Spec coverage:**
- Flake browser split + 4 warm + 4 cold (one code path) → Task 1. ✓
- Cold packages documented inline → Task 1 Step 3 comment. ✓
- `--browser` selector + docs → Task 3. ✓
- CI `validate-no-e2e` + matrix + `e2e-gate` → Task 4. ✓
- Diagnostics preserved via xtask (ADR-0032/#48) → Task 2 + Task 4 (uses `cargo xtask e2e`). ✓
- Required-checks wiring → Task 6. ✓
- ADR-0033 + CLAUDE.md → ADR already committed; CLAUDE.md Task 5. ✓
- Verification (build a combo; measure CI wall-clock) → Task 1 Step 7; CI wall-clock measured post-merge (noted in ship). ✓

**Placeholder scan:** none — every code step shows the code; commands have expected output.

**Type consistency:** `E2eBackend`/`E2eBrowser` enums + `as_str()` defined Task 2 Step 3, used Step 4/5; `e2e_combo(result, backend, browser)` signature matches its call. Flake `mkE2eCombo` arg names (`backend, browser, traceDigit, nameSuffix, warmupEnv`) consistent across Steps 3-5. Attr names `e2e-<backend>-<browser>[-cold]` consistent across Tasks 1-4.
