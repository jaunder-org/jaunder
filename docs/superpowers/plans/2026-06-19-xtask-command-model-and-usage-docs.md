# xtask Command Model, Cache Isolation & Usage Docs — Implementation Plan (Plan B1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finalize the `cargo xtask` command surface (`check [--no-test]` / `validate [--no-e2e]`), stop xtask edits from busting the Nix check caches, and document the usage contract so it's used correctly and consistently.

**Architecture:** `check` is the dev-facing auto-fixing family (`--no-test` = static+clippy only; default adds the Nix `coverage` check); `validate` is the strict never-mutating gate (`--no-e2e` = static+coverage; default adds the e2e VMs). The `--full`/`--no-fix` flags are removed — the auto-fix-vs-not and the e2e-or-not distinctions are now encoded in `check`-vs-`validate` and `--no-e2e`. The flake's whole-repo coverage source is filtered to exclude `xtask/` so editing the driver doesn't rebuild the instrumented suite. Usage canon lives in `CLAUDE.md` (agent) and `CONTRIBUTING.md` (human).

**Tech Stack:** Rust (`clap`, `xshell`), Nix (crane flake), markdown docs.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`. This plan (B1) implements the command-model + cache-isolation + usage-docs slice. The coverage post-processing engine (B2), the `postgres-integration` collapse (B3), and the CI/retirement cutover (B4) are separate plans.
- **Final command surface (verbatim):**
  | Command | Runs | Formatting |
  |---|---|---|
  | `cargo xtask check --no-test` | host static + clippy | auto-fix (`Mode::Fix`) |
  | `cargo xtask check` | static + clippy + Nix `coverage` check | auto-fix (`Mode::Fix`) |
  | `cargo xtask validate --no-e2e` | static + clippy + Nix `coverage` check | verify-only (`Mode::Check`) |
  | `cargo xtask validate` | static + clippy + `coverage` + `e2e-sqlite` + `e2e-postgres` | verify-only (`Mode::Check`) |
- **`postgres-integration` is NOT dispatched** by any xtask command (its tests already run under the `coverage` check via `-p jaunder --run-ignored all`; the VM check is collapsed in B3). `validate` runs `coverage` + `e2e-sqlite` + `e2e-postgres` only.
- **Every Nix invocation passes `--accept-flake-config`** and `--out-link .xtask/gcroots/<check>` (existing `build_check` behavior — do not change it).
- **The `xtask/` crate is committed** — it is a separate workspace, excluded from the root workspace (`exclude = ["xtask"]`); do not add it to the root workspace. Only the runtime dir `/.xtask/` (the `last-result.json` sidecar + `gcroots/` symlinks) and `xtask/target/` are gitignored.
- **Commit after every task**, on branch `testing-coverage-orchestration` (never `main`). The `.beads/issues.jsonl` file may ride along if pre-staged — that's fine; do not `git add` unrelated files.
- **Environment:** the Bash tool blocks `sed`/`grep`/`head`/`tail`/`awk` and complex compound commands — use `rg`, `jq`, the Read tool, and simple commands; run xtask via `cargo xtask …` (the alias), never `cargo run -- …`. For pass/fail gating, run the bare command through context-mode and rely on the exit code.

---

## File structure

- `flake.nix` — filter `xtask/` out of the shared `src` and the `coverage`/`coverage-update` whole-repo `src`.
- `xtask/src/lib.rs` — restructure the `Command` enum + `run` arms + `command_name`.
- `xtask/src/steps/nix.rs` — split the dispatch into `coverage()` and `e2e()`; drop `postgres-integration`.
- `CLAUDE.md` (project root) — add the agent-facing xtask usage contract.
- `CONTRIBUTING.md` — update the existing "Incoming: cargo xtask commands" note to the final surface.

---

## Task 1: Exclude `xtask/` from the flake source so xtask edits don't bust the check caches

Editing `xtask/*.rs` currently changes the input hash of every commonArgs-based derivation (its `src` includes `xtask/*.rs` via `filterCargoSources`) **and** the `coverage`/`coverage-update` derivations (which set `src = ./.`, the whole repo). Since `xtask/` is a separate workspace never built by those derivations, exclude it.

**Files:**
- Modify: `flake.nix` (the `src` block at ~261–269, and the `src = ./.;` lines in `coverage-update` ~901 and `coverage` ~1020)

- [ ] **Step 1: Record the current coverage drvPath (baseline for the proof)**

Run: `nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath`
Note the printed `/nix/store/<hash>-jaunder-coverage-0.1.0.drv` path.

- [ ] **Step 2: Filter `xtask/` out of the shared `src`**

In `flake.nix`, the shared source filter currently reads:

```nix
        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter =
            path: type:
            (pkgs.lib.hasSuffix ".sql" path)
            || (pkgs.lib.hasSuffix ".css" path)
            || (builtins.match "scripts/.*" path != null)
            || (craneLib.filterCargoSources path type);
        };
```

Add a leading exclusion so nothing under `xtask/` is ever included:

```nix
        src = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./.;
          filter =
            path: type:
            (!pkgs.lib.hasInfix "/xtask/" path)
            && (
              (pkgs.lib.hasSuffix ".sql" path)
              || (pkgs.lib.hasSuffix ".css" path)
              || (builtins.match "scripts/.*" path != null)
              || (craneLib.filterCargoSources path type)
            );
        };
```

- [ ] **Step 3: Filter `xtask/` out of the two whole-repo coverage sources**

The `coverage-update` (~901) and `coverage` (~1020) derivations each set `src = ./.;`. Replace **both** occurrences with a cleaned source that drops `xtask/`:

```nix
                src = pkgs.lib.cleanSourceWith {
                  src = ./.;
                  filter = path: _type: !(pkgs.lib.hasInfix "/xtask/" path);
                };
```

(Match the indentation of each site. Confirm there are exactly these two `src = ./.;` lines via `rg -n 'src = \./\.;' flake.nix` before editing; if e2e checks also use `src = ./.;`, leave them — this task targets the coverage caches.)

- [ ] **Step 4: Prove the coverage drvPath is now stable across an xtask edit**

```bash
nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath
```
Note it (it changed from Step 1 because the filter changed — expected). Now perturb xtask and confirm the drvPath does NOT change:

```bash
printf '\n// cache-isolation probe\n' >> xtask/src/main.rs
nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath
git checkout -- xtask/src/main.rs
```
Expected: the two drvPaths from this step are **identical** (the xtask edit did not change the coverage derivation). If they differ, the filter isn't catching `xtask/` — fix before committing.

- [ ] **Step 4b: Prove an APP edit STILL busts the coverage cache (no over-exclusion / staleness)**

The exclusion must drop ONLY `xtask/`, never app source. Confirm an app-file edit still changes the coverage drvPath:

```bash
nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath   # note A
printf '\n// app-source probe\n' >> server/src/lib.rs
nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath   # note B
git checkout -- server/src/lib.rs
```
Expected: A and B **differ** — touching app source still invalidates the coverage derivation (so coverage is never stale on real changes). If they're identical, the filter is over-excluding app code — fix before committing.

- [ ] **Step 5: Confirm the flake still evaluates and coverage still builds-from-cache**

Run: `nix build --dry-run --accept-flake-config .#checks.x86_64-linux.coverage`
Expected: resolves cleanly (the `jaunder-coverage` derivation listed), no eval errors.

- [ ] **Step 6: Commit**

```bash
git add flake.nix
git commit -m "build(flake): exclude xtask/ from coverage source so driver edits don't bust the cache"
```

---

## Task 2: Restructure the xtask command surface to `check [--no-test]` / `validate [--no-e2e]`

**Files:**
- Modify: `xtask/src/lib.rs`, `xtask/src/steps/nix.rs`

**Interfaces:**
- Produces:
  - `Command::Check { no_test: bool }`, `Command::Validate { no_e2e: bool }`
  - `Cli::command_name(&self) -> &'static str`
  - `steps::nix::coverage(result: &mut CommandResult)` — appends the `nix-coverage` step.
  - `steps::nix::e2e(result: &mut CommandResult)` — appends `nix-e2e-sqlite` + `nix-e2e-postgres`.
- Removed: `Command::Validate.full`, `Command::Validate.no_fix`, `steps::nix::run`, and any dispatch of `postgres-integration`.

- [ ] **Step 1: Rewrite the `Command` enum + `command_name` + `run` in `lib.rs`**

Replace the existing `Command` enum, the `command_name` method, and the `run` match arms with:

```rust
#[derive(Subcommand)]
pub enum Command {
    /// Inner loop (auto-fixes formatting): host static checks + clippy, then the
    /// Nix coverage check (instrumented test suite + coverage). `--no-test` runs
    /// static + clippy only.
    Check {
        /// Skip the Nix coverage check — static checks + clippy only.
        #[arg(long)]
        no_test: bool,
    },
    /// Full gate (never mutates the tree): static + clippy (verify-only) + the Nix
    /// coverage check + the e2e VMs. `--no-e2e` skips the e2e VMs.
    Validate {
        /// Skip the e2e VM checks — static + coverage only.
        #[arg(long)]
        no_e2e: bool,
    },
}

impl Cli {
    pub fn command_name(&self) -> &'static str {
        match self.command {
            Command::Check { .. } => "check",
            Command::Validate { .. } => "validate",
        }
    }
}

pub fn run(cli: Cli) -> anyhow::Result<CommandResult> {
    match cli.command {
        Command::Check { no_test } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("check");
            steps::static_checks::run(&sh, Mode::Fix, &mut result);
            if !no_test {
                steps::nix::coverage(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
        Command::Validate { no_e2e } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::nix::coverage(&mut result);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
    }
}

fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
```

(Keep the existing `use`/`mod`/`Cli` struct declarations. The `Cli` struct already has the global `--json` flag and `#[command(subcommand)] pub command: Command`.)

- [ ] **Step 2: Split `nix.rs` into `coverage` + `e2e`**

In `xtask/src/steps/nix.rs`, replace the `pub fn run(full: bool, …)` function with:

```rust
/// The Nix coverage check: the instrumented test suite (SQLite + ephemeral
/// PostgreSQL via `--run-ignored all`) plus the coverage gate.
pub fn coverage(result: &mut CommandResult) {
    result.push(build_check("nix-coverage", "coverage"));
}

/// The e2e VM checks (both backends). `postgres-integration` is deliberately
/// not dispatched — its tests already run under the coverage check.
pub fn e2e(result: &mut CommandResult) {
    result.push(build_check("nix-e2e-sqlite", "e2e-sqlite"));
    result.push(build_check("nix-e2e-postgres", "e2e-postgres"));
}
```

(Leave `SYSTEM`, `build_check`, and the `use` lines unchanged.)

- [ ] **Step 3: Build + unit tests**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Expected: the existing `result::tests` pass; no compile errors.
Run: `cargo build --manifest-path xtask/Cargo.toml`
Expected: clean, no warnings.

- [ ] **Step 4: Verify the CLI surface**

Run: `cargo xtask check --help`
Expected: shows `--no-test`.
Run: `cargo xtask validate --help`
Expected: shows `--no-e2e` and NO `--full`/`--no-fix`.

- [ ] **Step 5: Verify the fast (no-Nix) path end-to-end**

Run: `cargo xtask check --no-test`
Expected: green, steps `fmt`/`leptosfmt`/`prettier`/`cargo-deny`/`clippy` only (no `nix-coverage`); tree unmodified (Fix mode no-op on a clean tree).
Run: `jq '{command, ok, steps:[.steps[].name]}' .xtask/last-result.json`
Expected: `command` = `"check"`, `steps` has no `nix-*` entries.

> Do NOT run a full `cargo xtask validate` here (it builds the e2e VMs — slow); the coverage path is exercised in Step 6.

- [ ] **Step 6: Verify the coverage path hits the cache (fast, thanks to Task 1)**

Run: `cargo xtask check`
Expected: green; steps include `nix-coverage`; because Task 1 excluded `xtask/` from the coverage source, this is a cache hit (the `coverage` derivation was built earlier and GC-rooted) and returns quickly rather than rebuilding. `jq '.steps[].name' .xtask/last-result.json` includes `nix-coverage`.

- [ ] **Step 7: Commit**

```bash
git add xtask/src/lib.rs xtask/src/steps/nix.rs
git commit -m "feat(xtask): check [--no-test] / validate [--no-e2e] command model"
```

---

## Task 3: Agent usage contract in `CLAUDE.md` + host-only invariant comment

Document how to invoke and observe xtask so it's used correctly and consistently (the canonical form, not `cargo run --`, not defensive `echo`), and record the host-only invariant in both the docs and the flake.

**Files:**
- Modify: `CLAUDE.md` (project root) — add a new section.
- Modify: `flake.nix` — add an explanatory comment at the `xtask/` source-exclusion site.

- [ ] **Step 1: Add the usage-contract section**

Append this section to `CLAUDE.md` (after the existing context-mode notes):

```markdown
# xtask — how to run and observe it

`cargo xtask` is the dev/CI driver. Invoke it correctly and consistently:

- **Invoke via the alias, bare, through context-mode:** `ctx_execute(language:"shell", code:"cargo xtask validate")`. Not `cargo run --manifest-path …`, not raw Bash, and never with `2>&1` / `| tee` / `; echo` appended — a trailing command replaces the exit status and defeats the gate.
- **Pass/fail is the exit code** (context-mode's `isError`). Do not scrape stdout to decide whether it passed. A non-zero exit — including death by signal — means it failed.
- **Detail is the JSON sidecar, read separately:** a second call, e.g. `ctx_execute(shell, "jq '.steps' .xtask/last-result.json")`. The sidecar (`.xtask/last-result.json`, gitignored) is rewritten every run and carries `command`, `ok`, `duration_ms`, `finished_at_unix`, and `steps[]` (each `name`/`ok`/`detail`).
- **Completion proof is the `xtask-done:` sentinel** on stderr (`xtask-done: command=… ok=… exit=… duration_ms=…`), printed on every exit path. If you are reading a captured/truncated log rather than a live exit code, its presence proves the process finished; its absence means it died (panic/OOM/kill) — don't trust the sidecar then.

## Commands

| Command | Runs | Formatting |
|---|---|---|
| `cargo xtask check --no-test` | host static checks + clippy | auto-fixes |
| `cargo xtask check` | static + clippy + Nix coverage check (instrumented tests incl. PostgreSQL + coverage) | auto-fixes |
| `cargo xtask validate --no-e2e` | static + clippy + coverage (the pre-push-style gate) | verify-only, never mutates |
| `cargo xtask validate` | static + coverage + e2e (sqlite + postgres) — the full CI-faithful gate | verify-only, never mutates |

Use `check`/`check --no-test` while iterating; `validate` is what CI runs and what "green → you may move on" means. The Nix checks are cachix-pulled and GC-rooted, so an unchanged re-run reuses the cached build.

## Invariant: xtask is host-only

xtask runs **only on the host** (your dev box or the CI runner — both have the full checkout), where `cargo xtask` always rebuilds it from the live working tree. **Nix derivations never invoke xtask** — the checks run the raw tooling directly, and the flake deliberately excludes `xtask/` from its source, so an accidental `cargo xtask` inside a derivation fails loudly (missing crate) rather than running a stale copy. The flow is strictly one-directional: host `cargo xtask` → `nix build`; Nix never calls back. So xtask can never run stale.
```

- [ ] **Step 2: Comment the flake exclusion site**

In `flake.nix`, the shared source filter (~261) has the `xtask/` guard line `(!pkgs.lib.hasInfix "/xtask/" path)`. Add an explanatory comment immediately above it (match the indentation):

```nix
            # xtask/ is the host-only dev driver (a separate workspace these
            # derivations never build). Excluding it keeps driver edits from
            # busting the app caches AND guarantees a derivation can never run a
            # stale xtask: it is not in the sandbox, so an accidental
            # `cargo xtask` fails loudly rather than running stale. xtask runs
            # only on the host (dev box / CI runner).
            (!pkgs.lib.hasInfix "/xtask/" path)
```

Then confirm the flake still evaluates: `nix eval --accept-flake-config --raw .#checks.x86_64-linux.coverage.drvPath` resolves and the hash is **unchanged** from before this comment (comments don't affect `craneLib.path` source hashing, but confirm to be safe — if it changed, the comment landed inside the hashed source somehow; revert and place it outside the `cleanSourceWith`).

- [ ] **Step 3: Verify**

Run: `rg -n 'xtask-done|cargo xtask validate|--no-test|--no-e2e|host-only' CLAUDE.md`
Expected: the new section's key lines and the invariant are present.
Run: `rg -n 'host-only dev driver' flake.nix`
Expected: the comment is present at the exclusion site.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md flake.nix
git commit -m "docs(claude): xtask usage contract + host-only invariant (with flake comment)"
```

---

## Task 4: Update the human-facing `CONTRIBUTING.md` note to the final surface

The existing additive note (line ~104, "Incoming: `cargo xtask` commands (Plan A preview)") describes the old `check`/`validate`/`validate --full` surface. Update it to the final one. Still additive — the verify ladder stays authoritative until the B4 cutover.

**Files:**
- Modify: `CONTRIBUTING.md` (the "Incoming: `cargo xtask` commands" paragraph)

- [ ] **Step 1: Read the current note**

Run: `rg -n -A6 'Incoming: ' CONTRIBUTING.md`
Confirm the paragraph to replace.

- [ ] **Step 2: Replace the paragraph**

Replace the "Incoming: `cargo xtask` commands …" paragraph with:

```markdown
**Incoming: `cargo xtask` commands (preview).** `cargo xtask check` and `cargo xtask validate` are being introduced alongside the verify ladder. `check` runs the static checks + clippy on the host and **auto-fixes** formatting; by default it also runs the Nix `coverage` check (the instrumented test suite — including the ephemeral-PostgreSQL pass — plus the coverage gate), and `check --no-test` runs the static checks alone. `validate` is the strict, never-mutating gate: it runs the static checks (verify-only), the coverage check, and the e2e VM checks; `validate --no-e2e` skips the e2e VMs (the intended pre-push gate). All tests/coverage/e2e run via the Nix checks that match CI. Both commands write a machine-readable result to `.xtask/last-result.json` and a `xtask-done:` completion line to stderr. These will replace the verify ladder once the coverage post-processing and CI wiring land; until then the scripts remain authoritative and are not retired.
```

(Confirm the verify-ladder description elsewhere in the file is left intact.)

- [ ] **Step 3: Verify**

Run: `rg -n '\-\-no-test|\-\-no-e2e|verify ladder' CONTRIBUTING.md`
Expected: the updated note mentions `--no-test`/`--no-e2e`, and the verify-ladder lines still exist.

- [ ] **Step 4: Commit**

```bash
git add CONTRIBUTING.md
git commit -m "docs(contributing): update xtask preview to the check/validate command model"
```

---

## Task 5: Sync the spec's command surface to the final model

The spec (`docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`) still describes the superseded `check`/`validate --full`/`--no-fix` surface. Update it to match this plan. (The broader test-model + migration spec updates — `postgres-integration` collapse, pre-push hook — ride with B3/B4.)

**Files:**
- Modify: `docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`

- [ ] **Step 1: Update the "Command names" decisions row**

Replace:
```
| Command names | `check`, `validate`, `validate --full` | `check` = host static+clippy (inner loop); `validate` = check + the Nix coverage check (tests+coverage); `--full` adds the Nix e2e + postgres-integration checks. |
```
with:
```
| Command names | `check [--no-test]`, `validate [--no-e2e]` | `check` auto-fixes: static+clippy, plus the Nix coverage check unless `--no-test`. `validate` never mutates: static (verify-only) + coverage, plus the e2e VMs unless `--no-e2e`. No `--full`/`--no-fix` flags. |
```

- [ ] **Step 2: Update the "Formatting" decisions row**

Replace the `| Formatting | … |` row with:
```
| Formatting | `check` auto-fixes (`Mode::Fix`); `validate` verifies (`Mode::Check`) | The auto-fix-vs-not split is encoded in `check` vs `validate` (no `--no-fix` flag): iterate with `check` (fixes and proceeds), gate with `validate` (fails on unformatted, never mutates) — which is what CI runs. No standalone `format`. |
```

- [ ] **Step 3: Update the "Command surface" code block**

Replace the ```` ``` ````-fenced command-surface block with:
```
cargo xtask check --no-test   # host: static checks + clippy only (fast inner loop)
cargo xtask check             # + Nix coverage check (instrumented tests incl. PostgreSQL + coverage); auto-fixes
cargo xtask validate --no-e2e # static (verify-only) + coverage — the pre-push-style gate
cargo xtask validate          # + e2e-sqlite + e2e-postgres — the full CI-faithful gate
# --json available on every command; .xtask/last-result.json always written; xtask-done: sentinel on stderr
```

- [ ] **Step 4: Verify**

Run: `rg -n '\-\-no-test|\-\-no-e2e|\-\-full|\-\-no-fix' docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md`
Expected: `--no-test`/`--no-e2e` present; no remaining `--full`/`--no-fix` in the command-surface or decisions rows.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-06-18-testing-coverage-orchestration-design.md
git commit -m "docs(spec): sync command surface to check [--no-test] / validate [--no-e2e]"
```

---

## Self-review notes (for the implementer)

- **Order matters:** Task 1 (flake src exclusion) lands first so the command-restructure edits in Task 2 don't bust the coverage cache, keeping Step 6's verification a fast cache hit.
- **Scope:** B1 stops before the coverage post-processing engine (B2), the `postgres-integration` collapse (B3), and the CI/retirement cutover (B4). No script is retired and no CI file changes here.
- **Type consistency:** `Command::Check { no_test }` / `Command::Validate { no_e2e }`, `command_name`, `steps::nix::coverage`/`steps::nix::e2e`, and the `finalize` helper are the names later plans depend on.
- **Don't run full `validate`** during verification — the e2e VMs are slow; `check --no-test` and `check` (cached coverage) cover the reachable paths.
