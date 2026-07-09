# Spec — Extend `JAUNDER_CAPTURE_DIR` to the collector-written otel trace file (#332)

**Status:** approved
**Issue:** jaunder-org/jaunder#332
**Date:** 2026-07-09

## Problem

#227 consolidated the app's per-stream capture files into a single `JAUNDER_CAPTURE_DIR`
contract (`/var/lib/jaunder/capture/{mail,websub}.jsonl`, `diag.log`; ADR-0057), but
deliberately left one exfiltrated e2e diagnostic outside it: the OpenTelemetry trace file.

`otel-traces.jsonl` is written by the **otel-collector** service (`otelcol-contrib`, runs
as root), not the jaunder app — via a static YAML `file` exporter with a hardcoded
`path: /var/lib/jaunder/otel-traces.jsonl` (`flake.nix:525`). It sits *beside* the capture
dir, is copied out on its own (`flake.nix:670-671`), and is consumed on the success path
by `cargo xtask traces run` via a directory layout
(`$out/otel-traces-<backend>.jsonl/otel-traces.jsonl`, `traces/run.rs:33-37`).

So the e2e harness still has per-file otel plumbing: a separate exporter path, a separate
`copy_from_vm`, and a separate `nix.rs` lift filter.

## Goal

Fold the collector-written otel trace into the `JAUNDER_CAPTURE_DIR` contract: the
collector writes `<dir>/otel-traces.jsonl`, it rides the existing `capture-<backend>.tar.gz`
bundle, and the per-file otel copy-out/lift plumbing is deleted. VM-only change (the host
`cargo xtask e2e-local` driver runs no collector).

## Decisions (settled in design)

1. **Env-template the collector's output path.** Set `JAUNDER_CAPTURE_DIR` in the
   otel-collector systemd service environment (reusing the capture-dir binding — renamed
   this cycle from the now-misleading `mailCaptureEnv` to `captureEnv` — so the dir is
   single-sourced) and change the exporter to
   `path: ${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl` (`otelcol-contrib` supports
   `${env:…}` expansion). The collector honors the same contract var as the app. The
   `otel-traces.jsonl` *filename* still lives in the YAML — inherent, since the collector is
   not Rust and cannot call the `host` helper.
2. **A `systemd.tmpfiles` rule creates `capture/` before any service.** The collector is
   ordered *before* jaunder (`jaunder.after/requires = otel-collector.service`), so it
   cannot rely on the server's startup `create_dir_all`. Add
   `d /var/lib/jaunder/capture 0755 jaunder jaunder -` — created at boot, owned by `jaunder`
   (the server writes mail/websub/diag there), and root (the collector) writes too. The
   server's startup `create_dir_all` becomes a harmless no-op.
3. **Fold the lift fully into one bundle.** `otel-traces.jsonl` now lives under `capture/`,
   so it rides `capture-<backend>.tar.gz`. **Delete** the separate otel `copy_from_vm`
   (`flake.nix:670-671`) and the `nix.rs` `otel-traces-*` lift match. **Rework
   `cargo xtask traces run`** to extract the trace from the capture tarball
   (`capture-<backend>.tar.gz` → `capture/otel-traces.jsonl`) instead of the old
   `otel-traces-<backend>.jsonl/otel-traces.jsonl` directory layout.

**No `host::capture::Stream::OtelTraces` variant.** No `host`-linking Rust code writes or
reads the trace (collector = YAML; `xtask` is a separate workspace that does not link
`host`), so a variant would be dead public API — which the repo's dead-pub-API stance
disallows. The filename is collector-domain and restated in the YAML + `traces run`.

## Design

### `flake.nix`

- **Collector config** (`e2eOtelCollectorConfig`, shared `writeText`, `:513-532`): change
  the `file` exporter `path` to `${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl`. One edit
  (the config is shared by both VM blocks).
- **Collector service env** (both VM blocks — `:734` and `:829`): add
  `systemd.services.otel-collector.environment = captureEnv;` (the binding renamed from
  `mailCaptureEnv`) so `${env:JAUNDER_CAPTURE_DIR}` expands to `/var/lib/jaunder/capture`.
- **tmpfiles rule** (both VM blocks): `systemd.tmpfiles.rules = [ "d /var/lib/jaunder/capture 0755 jaunder jaunder -" ];`.
- **Copy-out** (`:665-671`): delete the separate otel `copy_from_vm` + its comment. The
  capture tarball (`:687`, `tar czf … -C /var/lib/jaunder capture`) already sweeps
  `capture/otel-traces.jsonl` in.

### `xtask/src/steps/nix.rs`

- Drop the `otel-traces-*` match from the lift filter (`:142`) and its dedicated
  directory-copy unit-test hunk (`:617-639`). The trace ships inside `capture-*.tar.gz`
  (already lifted by the `capture-` match added in #227). Update the doc comment.
- **Remove the now-dead directory-copy path.** `otel-traces-*` was the *only* artifact
  lifted as a directory, so once its match is gone, `copy_tree` (`:178`) and the
  `from.is_dir()` copy branch (`:163-164`) become dead private code → `dead_code` under
  deny-warnings. Delete them; every remaining lifted artifact is a flat `std::fs::copy`.

### `xtask/src/traces/run.rs`

- Rework `collect_trace_files` (`:52-65`) / `trace_file_path` (`:33-37`): instead of
  resolving `<out>/otel-traces-<backend>.jsonl/otel-traces.jsonl`, locate
  `<out>/capture-<backend>.tar.gz`, extract its `capture/otel-traces.jsonl` member, and hand
  the extracted file to `traces::analyze`. Update the module doc (`:5`, `:33`) and the path
  unit test (`:83-89`).
- **Per-backend temp paths (collision guard):** the tarball's inner member is
  `capture/otel-traces.jsonl` — *identical* across backends — so extraction MUST land at a
  distinct per-backend temp path (e.g. `<tmp>/otel-traces-<backend>.jsonl`) or collecting
  both backends clobbers.
- **Extraction mechanism:** `flate2` (already an xtask dep) covers only the gzip layer, not
  tar. The plan adds the `tar` crate **or** shells out `tar xzf` via `xshell` (already
  available) — decide in the plan.

### Docs & help text

- `xtask/src/lib.rs:192` — the `traces analyze` `after_help` EXAMPLES string embeds the old
  `otel-traces-<backend>.jsonl/otel-traces.jsonl` layout; update it (else it contradicts
  AC3). (The `testdata/*-sample.jsonl` trace fixtures keep their names — they are input
  data, not the lift layout.)
- Live docs referencing the old path/layout, to update: `docs/observability.md`
  (`:11-13`, `:82-83`), `docs/ARCHITECTURE.md` (`:119-121`), `CONTRIBUTING.md`
  (`:283-284`). Archive docs under `docs/archive/` are historical (out of scope).

## Acceptance criteria (observable)

1. **Collector writes under the dir.** After an e2e run, the trace is at
   `/var/lib/jaunder/capture/otel-traces.jsonl` in the VM (not the old
   `/var/lib/jaunder/otel-traces.jsonl`); `rg 'otel-traces\.jsonl' flake.nix` shows the path
   only as `${env:JAUNDER_CAPTURE_DIR}/otel-traces.jsonl`, and the collector service env
   carries `JAUNDER_CAPTURE_DIR`.
2. **Dir exists before the collector.** The `systemd.tmpfiles` rule is present in both VM
   blocks; the collector (root, ordered before jaunder) writes the trace with no failure —
   demonstrated by the trace being non-empty in the run.
3. **One bundle, no per-file otel plumbing.** The separate otel `copy_from_vm`
   (`flake.nix`) and the `nix.rs` `otel-traces-*` lift match (plus the now-dead `copy_tree`
   path) are gone; `otel-traces.jsonl` rides inside `capture-<backend>.tar.gz`. A scoped
   sweep — `rg 'otel-traces-' flake.nix xtask/src` **excluding the trace fixtures**
   (`testdata/*-sample.jsonl`) — leaves only `traces run`'s per-backend temp path; the old
   layout is gone from the flake, the lift filter, and the `lib.rs` help example.
4. **Trace tooling still works.** The `traces run` path/extraction unit test asserts the
   new per-backend temp-path resolution. End-to-end, `cargo xtask traces run` (and `--cold`)
   succeeds against the reworked source (extracts `capture/otel-traces.jsonl` from the
   tarball → analysis) — but that builds the full nix matrix, so it is verified **together
   with AC5's `validate`**, not in a fast unit gate.
5. **e2e matrix green.** `cargo xtask validate` passes — the full
   `{sqlite,postgres}×{chromium,firefox}` matrix runs with the collector writing under
   `capture/` and the tarball including the trace.
6. **App capture unaffected.** mail/websub/diag still land in `capture/` and the zero-panic
   gate still reads `capture/diag.log` — identical to post-#227.

## Out of scope

- The `host` crate is untouched (no `OtelTraces` stream variant — see Decisions).
- The host `cargo xtask e2e-local` driver — it runs no otel-collector, so no change.
- The otel trace *format* / the `traces analyze` logic — only the *source path* changes.
- ADR: this **extends** ADR-0057's contract rather than making a new architectural
  decision; record it as a short amendment/note on ADR-0057 (the contract now covers the
  collector-written trace, via `${env:}` + tmpfiles) — decide new-ADR-vs-note at plan time.

## Risks / notes

- **tmpfiles ↔ StateDirectory ownership.** `systemd.tmpfiles` creates
  `/var/lib/jaunder/capture` at boot (auto-creating `/var/lib/jaunder` as parent);
  `StateDirectory = "jaunder"` (`flake.nix:128`) also manages `/var/lib/jaunder` ownership
  at jaunder-start. These should reconcile (jaunder-owned), but it is the one spot to
  verify at the full-matrix run — a permission mismatch would surface as the server failing
  to write `capture/` or the collector failing to write the trace.
- **otelcol `${env:}` expansion** must be the correct syntax for the pinned
  `opentelemetry-collector-contrib`; verify the trace file appears at the templated path in
  the matrix run.
- Keep the `issue-332` token in the branch, worktree, spec, and plan filenames.
