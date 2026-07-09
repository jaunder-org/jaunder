# Otel Trace → Capture-Dir (#332) Implementation Plan

> **For agentic workers:** Execute task-by-task with **jaunder-iterate** (delegating a task to a subagent via **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Fold the collector-written `otel-traces.jsonl` into the `JAUNDER_CAPTURE_DIR` contract — collector writes `<dir>/otel-traces.jsonl`, it rides `capture-<backend>.tar.gz`, and the per-file otel copy-out/lift is deleted (VM-only).

**Spec:** [`docs/superpowers/specs/2026-07-09-issue-332-otel-capture-dir.md`](../specs/2026-07-09-issue-332-otel-capture-dir.md) — the "what/why". This plan is the "how".

## Scope

- **In:** `flake.nix` (collector config path, collector env, tmpfiles, delete otel copy-out); `xtask/src/steps/nix.rs` (drop otel lift + dead `copy_tree`); `xtask/src/traces/run.rs` (untar the trace from the capture tarball); docs + `lib.rs` help; an ADR-0057 forward note.
- **Out:** the `host` crate (no `OtelTraces` stream variant — nothing host-linking uses it); the host `e2e-local` driver (no collector); the trace format / `traces analyze` logic.

## Tasks

1. **flake.nix** — env-template the collector path, add collector env + tmpfiles rule, delete the separate otel copy-out.
2. **nix.rs** — drop the `otel-traces-*` lift match + the now-dead `copy_tree`/`is_dir` branch; update its unit test.
3. **traces/run.rs** — rework `collect_trace_files` to untar `capture/otel-traces.jsonl` from `capture-<backend>.tar.gz` (per-combo temp dir); swap the pure path helper + its test; update the `lib.rs` caller.
4. **docs + help + ADR note** — `lib.rs:192` help example, `observability.md`, `ARCHITECTURE.md`, `CONTRIBUTING.md`, and a forward note on ADR-0057.
5. **Full-gate acceptance** — `cargo xtask validate` (matrix) + the scoped clean-break sweep.

## Key risks / decisions

- **tmpfiles ↔ `StateDirectory=jaunder` ownership** on `/var/lib/jaunder` — the one spot to verify at the matrix run (spec Risk).
- **otelcol `${env:VAR}` expansion** must match the pinned collector — verified when the trace appears at the templated path in Task 5.
- **Extraction** shells out `tar` (no `tar` crate in `xtask`; `flate2` is gzip-only) via the already-present `xshell`; per-combo temp dirs avoid the `capture/otel-traces.jsonl` name collision across backends.

## Global Constraints

- **Per-commit gate:** run `cargo xtask check` clean before each commit (**jaunder-commit**); it runs host static + clippy + Nix coverage/tests, **not** the full e2e matrix — so intermediate commits stay green while the flake wiring is mid-change; the matrix is Task 5.
- **No `Co-Authored-By` trailer.** Commit subjects reference `(#332)`.
- Follow `CONTRIBUTING.md` (coverage ADR-0050, no lint suppression without approval). No new storage/dual-backend tests.
- `nix.rs`/`run.rs` are in `xtask` (a workspace excluded from the root) — run their tests with `--manifest-path xtask/Cargo.toml`.

---

## Task 1: `flake.nix` — collector writes under the capture dir

**Files:** Modify `flake.nix` — the `mailCaptureEnv` binding (`:52`) + its three consumers (`:180`, `:749`, `:858`); collector config (`:513-532`), both `systemd.services.otel-collector` blocks (`:734`, `:829`), both VM blocks' systemd config (for tmpfiles), the otel copy-out (`:665-671`).

- [ ] **Step 1: Env-template the exporter path.** In `e2eOtelCollectorConfig` (`:524-525`), change:

```yaml
          exporters:
            file:
              path: ${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl
```

- [ ] **Step 2: Rename the binding, then give the collector the dir var.** The `mailCaptureEnv` binding (`:52`) is now misleading — it holds only `{ JAUNDER_CAPTURE_DIR = "/var/lib/jaunder/capture"; }`. Rename it to **`captureEnv`** and update its three `systemd.services.jaunder.environment` consumers (`:180`, `:749`, `:858`) + its comment. Then add `environment = captureEnv;` to **both** `systemd.services.otel-collector` blocks (`:734`, `:829`), so `${env:JAUNDER_CAPTURE_DIR}` expands:

```nix
                systemd.services.otel-collector = {
                  description = "Jaunder e2e OTel Collector";
                  wantedBy = [ "multi-user.target" ];
                  after = [ "network.target" ];
                  environment = captureEnv;
                  serviceConfig = { ... };
                };
```

- [ ] **Step 3: Create `capture/` before any service.** In **both** VM blocks (alongside each `systemd.services.otel-collector`), add:

```nix
                systemd.tmpfiles.rules = [ "d /var/lib/jaunder/capture 0755 jaunder jaunder -" ];
```

(Boot-time, jaunder-owned so the server writes mail/websub/diag; root — the collector — writes too. The server's startup `create_dir_all` becomes a no-op.)

- [ ] **Step 4: Delete the separate otel copy-out.** Remove `flake.nix:665-671` (the `# OTel trace keeps its directory layout` comment + the guarded `copy_from_vm("/var/lib/jaunder/otel-traces.jsonl", "otel-traces-${backend}.jsonl")`). The capture tarball (`:687`, `tar czf … -C /var/lib/jaunder capture`) now sweeps `capture/otel-traces.jsonl` in.

- [ ] **Step 5: Evaluate the flake, verify it parses.** Behavioral verification is Task 5 (the matrix). A flake syntax error fails the commit gate fast (the coverage nix build evals the flake).

- [ ] **Step 6: Commit**

```bash
git add flake.nix
git commit -m "refactor(e2e): otel collector writes under JAUNDER_CAPTURE_DIR; rename mailCaptureEnv->captureEnv (#332)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

## Task 2: `xtask/src/steps/nix.rs` — drop the otel lift + dead code

**Files:** Modify `xtask/src/steps/nix.rs` — `wanted` filter (`:142`), the copy dispatch (`:160-167`), `copy_tree` (`:178-190`), doc comment (`:128-134`), unit test (`:616-640`).

**Interfaces:** `copy_e2e_diagnostics_between` unchanged in signature.

- [ ] **Step 1: Update the failing unit test.** In `copy_e2e_diagnostics_between_copies_journal_otel_and_playwright` (`:598`): remove the `otel-traces-sqlite.jsonl` directory-artifact creation (`:616-623`) and its lifted-dir assertion (`:636-639`); drop the count for it (the OTel dir is no longer a separately-lifted artifact — it rides `capture-*.tar.gz`, which the test already covers via the `capture-sqlite.tar.gz` case). Adjust the expected count accordingly.

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml copy_e2e_diagnostics_between`
Expected: FAIL — the filter still lifts `otel-traces-*`.

- [ ] **Step 3: Implement.** In `wanted` (`:142`), delete the `otel-traces-` line. Then the dir-copy path is dead: replace the `let ok = if from.is_dir() { copy_tree(&from, &to).is_ok() } else { std::fs::copy(&from, &to).is_ok() };` (`:163-167`) with the flat-file copy only:

```rust
        let ok = std::fs::copy(&from, &to).is_ok();
```

Delete `copy_tree` (`:178-190`) and update the `:160-162` comment (all remaining lifted artifacts are flat files) and the module doc (`:128-134`) — drop the OTEL-traces mention.

- [ ] **Step 4: Run, verify pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml copy_e2e_diagnostics_between`
Expected: PASS. Also `cargo clippy --manifest-path xtask/Cargo.toml` clean (no `dead_code`).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "refactor(xtask): drop the separate otel-traces lift; it rides capture-*.tar.gz (#332)"
```

---

## Task 3: `xtask/src/traces/run.rs` — untar the trace from the capture tarball

**Files:** Modify `xtask/src/traces/run.rs` (`trace_file_path`→helper `:33-38`, `collect_trace_files` `:52-65`, module doc `:5`, tests `:83-89`); `xtask/src/lib.rs` (the `collect_trace_files` caller, ~`:421`).

**Interfaces:**
- Produces: `capture_tarball_path(out: &str, backend: E2eBackend) -> PathBuf` = `<out>/capture-<backend>.tar.gz` (replaces `trace_file_path`).
- Changes: `collect_trace_files(cold, browser) -> Result<(tempfile::TempDir, Vec<PathBuf>)>` — the `TempDir` guards the extracted files; the caller binds it to keep them alive through `analyze`.

- [ ] **Step 1: Rewrite the failing pure-path test.** Replace `trace_file_path_shape` (`:83-89`) with:

```rust
    #[test]
    fn capture_tarball_path_shape() {
        assert_eq!(
            capture_tarball_path("/nix/store/x", E2eBackend::Sqlite),
            PathBuf::from("/nix/store/x/capture-sqlite.tar.gz")
        );
    }
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml capture_tarball_path_shape`
Expected: FAIL — `capture_tarball_path` not defined.

- [ ] **Step 3: Implement.** Replace `trace_file_path` with the tarball helper, and rework `collect_trace_files` to extract each combo's trace into a per-combo subdir of one `TempDir` (avoids the shared `capture/otel-traces.jsonl` name colliding across backends):

```rust
use xshell::{cmd, Shell};
use tempfile::TempDir;

/// The capture bundle inside a built e2e check's store path: `<out>/capture-<backend>.tar.gz`.
pub fn capture_tarball_path(out: &str, backend: E2eBackend) -> PathBuf {
    PathBuf::from(out).join(format!("capture-{}.tar.gz", backend.as_str()))
}

/// Build every combo, extract each `capture/otel-traces.jsonl` from its capture tarball
/// into a per-combo subdir of `tmp`, and return the extracted files. The returned
/// `TempDir` must outlive analysis (the caller binds it). nix/filesystem I/O — not
/// unit-tested; exercised by a manual run.
pub fn collect_trace_files(
    cold: bool,
    browser: Option<E2eBrowser>,
) -> Result<(TempDir, Vec<PathBuf>)> {
    let sh = Shell::new()?;
    let tmp = TempDir::new()?;
    let browsers = browsers(browser);
    let mut files = Vec::new();
    for backend in BACKENDS {
        for &browser in &browsers {
            let attr = e2e_attr(backend, browser, cold);
            let out = build_out_path(&attr)?;
            let tarball = capture_tarball_path(&out, backend);
            ensure!(tarball.exists(), "capture tarball not found: {}", tarball.display());
            let dest = tmp.path().join(format!("{}-{}", backend.as_str(), browser.as_str()));
            std::fs::create_dir_all(&dest)?;
            // Extract just the one member; per-combo dest avoids the shared inner-path collision.
            cmd!(sh, "tar -xzf {tarball} -C {dest} capture/otel-traces.jsonl").run()?;
            let file = dest.join("capture/otel-traces.jsonl");
            ensure!(file.exists(), "otel trace missing from {}", tarball.display());
            files.push(file);
        }
    }
    Ok((tmp, files))
}
```

Update the module doc (`:5`) to describe untarring from `capture-<backend>.tar.gz`. Update the `lib.rs` caller (~`:421`) from `let files = collect_trace_files(...)?;` to `let (_tmp, files) = collect_trace_files(...)?;` (bind `_tmp` so the extracted files survive until `analyze`/`render` finish).

- [ ] **Step 4: Run, verify pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml -E 'package(xtask)'`
Expected: PASS (the path test + the rest of xtask's suite compile & pass; the tar I/O in `collect_trace_files` is manual, verified in Task 5).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/traces/run.rs xtask/src/lib.rs
git commit -m "refactor(xtask): traces run extracts otel-traces from capture-*.tar.gz (#332)"
```

---

## Task 4: docs, help text, and the ADR-0057 note

**Files:** `xtask/src/lib.rs` (`:192` help example), `docs/observability.md` (`:11-13`, `:82-83`), `docs/ARCHITECTURE.md` (`:119-121`), `CONTRIBUTING.md` (`:283-284`), `docs/adr/0057-e2e-capture-dir-contract.md` (forward note).

- [ ] **Step 1: Help example** — `xtask/src/lib.rs:192` (`traces analyze` `after_help` EXAMPLES): update the embedded `otel-traces-<backend>.jsonl/otel-traces.jsonl` path to the delivered flow (analyze accepts extracted `otel-traces.jsonl` files; reference `capture-<backend>.tar.gz` as the bundle they come from). Leave `testdata/*-sample.jsonl` fixture names untouched.
- [ ] **Step 2: `docs/observability.md`** (`:11-13`, `:82-83`) — the trace is now at `/var/lib/jaunder/capture/otel-traces.jsonl` (VM) and lifted inside `capture-<backend>.tar.gz`.
- [ ] **Step 3: `docs/ARCHITECTURE.md`** (`:119-121`) and **`CONTRIBUTING.md`** (`:283-284`) — same path/layout update.
- [ ] **Step 4: ADR-0057 forward note** — add a short note (Consequences or top, per the annotate-not-rewrite convention) that #332 extended the contract to the collector-written otel trace, via `${env:JAUNDER_CAPTURE_DIR}` on the collector + a `systemd.tmpfiles` rule (no new ADR — this applies the existing decision, not a novel one).
- [ ] **Step 5: Prettier-format** the edited Markdown before staging (repo pre-commit runs `prettier -w`).
- [ ] **Step 6: Commit**

```bash
git add xtask/src/lib.rs docs/observability.md docs/ARCHITECTURE.md CONTRIBUTING.md docs/adr/0057-e2e-capture-dir-contract.md
git commit -m "docs(e2e): otel trace lands under JAUNDER_CAPTURE_DIR; note on ADR-0057 (#332)"
```

---

## Task 5: Full-gate acceptance

**Files:** none (verification only).

- [ ] **Step 1: Scoped clean-break sweep (AC3).**

Run: `rg 'otel-traces-' flake.nix xtask/src -g '!*-sample.jsonl'`
Expected: **no matches** — the old `otel-traces-<backend>.jsonl/…` lift/copy-out/help layout is fully gone. (The reworked `collect_trace_files` names its per-combo temp subdirs `<backend>-<browser>` and the extracted member is `capture/otel-traces.jsonl`, so neither carries the `otel-traces-` string.)

- [ ] **Step 2: Full local gate + matrix (AC1, AC2, AC5, AC6, and AC4 end-to-end).**

Run: `devtool run -- cargo xtask validate` (background — long). Confirm the `xtask-done: … ok=true` sentinel and green across all four combos. This exercises: collector writing to `capture/otel-traces.jsonl` (AC1/AC2 — non-empty trace ⇒ tmpfiles + `${env:}` worked), the trace riding the tarball (AC3), and the zero-panic gate still reading `capture/diag.log` (AC6).

- [ ] **Step 3: Trace-tooling end-to-end (AC4).** After the matrix is warm, `devtool run -- cargo xtask traces run --top 5` (background — rebuilds/reuses the combos) confirms `collect_trace_files` untars and analyzes successfully. `--cold` shares the identical extraction path (only the nix attr differs — `packages…-cold` vs `checks…`, same `capture-<backend>.tar.gz` artifact), so warm suffices; run `--cold` too only if verifying cold specifically. (Heavy; the pure tarball-path resolution is otherwise covered by Task 3's unit test.)

- [ ] **Step 4:** If green, ready for **jaunder-ship**.

---

## Self-review notes

- **Spec coverage:** AC1→Task 1 (Steps 1-2); AC2→Task 1 (Step 3) + Task 5 (Step 2, non-empty-trace demo); AC3→Tasks 1(4)/2/3/4 + Task 5 (Step 1); AC4→Task 3 (the pure `capture_tarball_path` *source* resolution is unit-tested; the "per-backend temp-path" collision-avoidance is structural — a separate per-combo `TempDir` subdir, not a unit assertion) + Task 5 (Step 3, end-to-end); AC5→Task 5 (Step 2); AC6→Task 5 (Step 2). No `host`/e2e-local change (out of scope) — correctly absent.
- **Type consistency:** `capture_tarball_path(out, backend)` produced in Task 3 and used by the reworked `collect_trace_files`; the `(TempDir, Vec<PathBuf>)` return threads to the `lib.rs` caller.
- **Ordering:** Tasks 1-4 are independent edits (flake / nix.rs / run.rs+lib.rs / docs), each committable green under `cargo xtask check`; the flake↔traces-run consistency (trace under `capture/` → tarball → untar) is proven only at Task 5's matrix.
