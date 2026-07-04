# Plan: issue #234 — reconcile CSR wasm filename + bootstrap URL

Spec:
[`docs/superpowers/specs/2026-07-04-issue-234-csr-wasm-filename-reconcile.md`](../specs/2026-07-04-issue-234-csr-wasm-filename-reconcile.md)
Issue: [#234](https://github.com/jaunder-org/jaunder/issues/234)

## Review header

**Goal.** Make the host `cargo leptos end-to-end` CSR build hydrate: point both
hand-written bootstraps at the wasm filename cargo-leptos actually emits
(`jaunder.wasm`), reconcile the Nix bundle to the same name, and add
drift-guards so it can't silently re-break. Narrow bug fix; unblocks #153 AC8.

**Scope.**

- _In_: `csr/index.html` + `server/src/projector/mod.rs` bootstraps; `flake.nix`
  `csrWasmBundle` rename; `xtask/src/audit_wasm.rs` (name + tests); two
  drift-guard tests; consistency updates to stray `jaunder_bg.wasm` references.
- _Out_ (filed follow-ups): unifying the build onto cargo-leptos (#236);
  embedding the bundle for single-binary (#237). Do **not** touch the build
  tooling or asset serving here.

**Tasks.**

1. Bootstraps load the wasm by its emitted name + drift-guard tests (`web`,
   `jaunder`).
2. Reconcile the Nix bundle filename (`flake.nix`) + `audit_wasm` + stray refs.
3. Verify host + Nix e2e green and `audit-wasm` passes (the ACs).

**Key risks / decisions.**

- Canonicalize on `jaunder.wasm` (cargo-leptos's natural output) so the host
  needs no post-build reconciliation; Nix moves to match.
- Tasks 1 and 2 must both land before merge — after Task 1 alone, Nix still
  emits `jaunder_bg.wasm` and its e2e would 404. Unit tests pass after Task 1;
  Nix e2e is verified after Task 2 (Task 3).
- The explicit `init("/pkg/jaunder.wasm")` URL overrides wasm-bindgen's internal
  default, so the host's unrenamed internal reference is harmless.

**For agentic workers.** Drive with `jaunder-iterate`; delegate a task via
`jaunder-dispatch` when useful. Tick checkboxes in real time.

## Global constraints

- Worktree: `.claude/worktrees/issue-234-host-csr-wasm-name` (already created).
- Gate before every commit: `cargo xtask check` must pass clean (the pre-commit
  hook runs it); commit per `jaunder-commit`. **No `Co-Authored-By` trailer.**
- No storage changes → no backend-parity/dialect concerns here.
- Crate names: web = `web`, server = `jaunder`, xtask = `xtask`.
- Leave `docs/archive/**` untouched (historical).

---

## Task 1 — Bootstraps load the wasm by its emitted name (+ drift-guards)

Both bootstraps currently call wasm-bindgen's `init()` with no argument, falling
back to the non-existent `jaunder_bg.wasm`. Fix both to load `/pkg/jaunder.wasm`
explicitly, guarded by tests. TDD: guard first (RED), then the edit (GREEN).

### Files

- `web/src/render/mod.rs` — add drift-guard test (in the existing `#[cfg(test)]`
  module, beside `index_html_shell_contains_the_prepaint_script`).
- `csr/index.html` — line 16 bootstrap.
- `server/src/projector/mod.rs` — add drift-guard test (in the `#[cfg(test)]`
  module at line 282, beside `document_head_starts_with_the_prepaint_script`);
  edit the bootstrap literal at line 80.

### Step 1.1 — SPA shell (`csr/index.html`)

Add to `web/src/render/mod.rs` tests:

```rust
#[test]
fn csr_index_html_boots_wasm_by_emitted_filename() {
    // Drift guard (#234): the SPA shell must load the wasm by the filename
    // cargo-leptos emits (`jaunder.wasm`), NOT wasm-bindgen's arg-less
    // `jaunder_bg.wasm` default. If these drift, hydration 404s silently.
    let index = include_str!("../../../csr/index.html");
    assert!(
        index.contains(r#"init("/pkg/jaunder.wasm")"#),
        "csr/index.html must boot via init(\"/pkg/jaunder.wasm\") (drift guard #234)"
    );
}
```

- Run (RED):
  `cargo nextest run -p web csr_index_html_boots_wasm_by_emitted_filename` →
  **FAIL** (shell still calls `init()`).
- Edit `csr/index.html:16` from `init();` to `init("/pkg/jaunder.wasm");`.
- Run (GREEN): same command → **PASS**.

### Step 1.2 — Projector anonymous HTML (`server/src/projector/mod.rs`)

Add to the `#[cfg(test)]` module (mirror the seed built by
`document_head_starts_with_the_prepaint_script`):

```rust
#[test]
fn document_boots_wasm_by_emitted_filename() {
    use super::document;
    use web::render::PageSeed;
    // Drift guard (#234): projector anonymous HTML must load the wasm by the
    // filename cargo-leptos emits. Public projector routes use this shell, so an
    // arg-less init() here 404s hydration on those routes even when the SPA shell
    // is fixed.
    let doc = document(&PageSeed::SiteTimeline(web::posts::TimelinePage {
        posts: vec![],
        next_cursor_created_at: None,
        next_cursor_post_id: None,
        has_more: false,
    }));
    assert!(
        doc.contains(r#"init("/pkg/jaunder.wasm")"#),
        "projector document() must boot via init(\"/pkg/jaunder.wasm\") (drift guard #234): {doc}"
    );
}
```

- Run (RED):
  `cargo nextest run -p jaunder document_boots_wasm_by_emitted_filename` →
  **FAIL**.
- Edit `server/src/projector/mod.rs:80` from
  `"<script type=\"module\">import init from \"/pkg/jaunder.js\"; init();</script>",`
  to
  `"<script type=\"module\">import init from \"/pkg/jaunder.js\"; init(\"/pkg/jaunder.wasm\");</script>",`.
- Run (GREEN): same command → **PASS**.

### Verify & commit

- `cargo xtask check` → PASS (fmt + clippy + coverage/tests).
- Commit (`jaunder-commit`):
  `fix(csr): boot wasm by emitted filename to restore host hydration (#234)`.

---

## Task 2 — Reconcile the Nix bundle filename + audit + stray refs

Move the Nix bundle to `jaunder.wasm` so both bootstraps' URL resolves in CI,
and update every consumer of the old name.

### Files

- `flake.nix` — `csrWasmBundle` (lines 500-502).
- `xtask/src/audit_wasm.rs` — `run()` names (133) + tests (202, 220, 222, 231
  comment, 239). **Must-fix**: it reads `nix build .#site` and errors if the
  named artifact is absent.
- Consistency (self-contained; update for accuracy, non-blocking to hydration):
  `xtask/src/lib.rs` doc comment (~87), `xtask/src/result.rs` test data (~176),
  `xtask/src/traces/testdata/otel-traces-sample.jsonl` + the assertion in
  `xtask/src/traces/analyze.rs` (~1038) that reads it, `docs/observability.md`.

### Step 2.1 — flake rename

In `flake.nix` `csrWasmBundle`:

```nix
mv $out/csr.js $out/jaunder.js
mv $out/csr_bg.wasm $out/jaunder.wasm
sed -i 's/csr_bg\.wasm/jaunder.wasm/g' $out/jaunder.js
```

(Only lines 501-502 change: the wasm becomes `jaunder.wasm`, and `jaunder.js`'s
internal reference is rewritten to match — keeping the Nix bundle
self-consistent even independent of the explicit `init` URL.)

### Step 2.2 — audit_wasm

- `xtask/src/audit_wasm.rs:133`:
  `let names = ["pkg/jaunder.wasm", "pkg/jaunder.js"];`.
- Update the three tests that hard-code `jaunder_bg.wasm` (the `ArtifactMetrics`
  path + `contains`/`!contains` asserts in `render_table_…`, and the comment +
  `err.contains("jaunder.wasm")` in `run_errors_when_artifact_missing`).
- Run: `cargo nextest run -p xtask` → **PASS**.

### Step 2.3 — consistency stragglers

Update the remaining `jaunder_bg.wasm` occurrences to `jaunder.wasm` in
`xtask/src/lib.rs` (doc comment), `xtask/src/result.rs` (test data),
`xtask/src/traces/testdata/otel-traces-sample.jsonl` **and** the matching
assertion in `xtask/src/traces/analyze.rs` (update both together so the trace
test stays consistent), and `docs/observability.md`. Confirm none remain outside
`docs/archive`:

```
rg -n 'jaunder_bg\.wasm' --glob '!target' --glob '!docs/archive' .
```

(expect: only this plan/spec reference it in prose.)

### Verify & commit

- `cargo nextest run -p xtask` → PASS.
- `cargo xtask audit-wasm` → **PASS**, proving `nix build .#site` now emits
  `pkg/jaunder.wasm` (this exercises the flake rename end-to-end).
- `cargo xtask check` → PASS. Commit:
  `build(nix): emit CSR wasm as jaunder.wasm to match the bootstrap (#234)`.

---

## Task 3 — Verify host + Nix e2e green (the acceptance proof)

No code; this is the AC gate. Heavy runs — use Bash background mode.

- **AC1 (host hydration):** `cargo leptos end-to-end` → e2e suite green;
  `body[data-hydrated]` set (the 68/71 timeouts clear), including on projector
  routes. (Runs `bash run-e2e.sh`: chromium Playwright, workers=1; ensure
  `end2end/` node deps are provisioned — `end2end/provision-node-modules.sh` —
  if the browser/deps are missing.)
- **AC2 (Nix e2e):** `cargo xtask e2e sqlite chromium` → the combo's Nix check
  passes with the renamed bundle. (One combo proves the rename; the four are
  identical w.r.t. the bundle. Optionally the full `cargo xtask validate` before
  ship.)
- **AC6 (audit):** `cargo xtask audit-wasm` → PASS (already run in Task 2;
  re-confirm).

Record outcomes faithfully in the PR (paste the green summaries). If AC1 still
shows any hydration timeout, the likely cause is a _third_ un-fixed bootstrap —
grep `init()` again and extend Task 1 + its drift-guard rather than patching
around it.

---

## Self-review

- Every task is a small, cohesive commit with a RED→GREEN or build→verify shape.
- The two bootstraps (the cold-review gap) are both fixed and both
  drift-guarded.
- No build-tooling or asset-serving changes leak in from #236/#237.
- The flake rename's blast radius (`audit_wasm` + stray refs) is enumerated; a
  final `rg` confirms none remain.
- Standalone: the fix depends on no #153 changes.
