# Migrate `scripts/audit-wasm-bundle` → `cargo xtask audit-wasm`

**Date:** 2026-06-27
**Issue:** #31 (milestone 3, "Devtool migration")
**Status:** Approved design (pre-implementation)
**Related:** [ADR-0028](../../adr/0028-devtool-vs-xtask-boundary.md) (the `devtool`/`xtask` boundary)

## Context

`scripts/audit-wasm-bundle` is a Node.js dev tool that measures the frontend
bundle size — raw, gzip, and brotli — of the deterministic `nix build .#site`
output (`pkg/jaunder_bg.wasm`, `pkg/jaunder.js`). It is invoked **manually** by
developers; it has **no programmatic callers** (referenced only in
`CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, `docs/observability.md`).

Issue #31, like its four siblings, was filed "migrate into `devtool`." But
`audit-wasm-bundle` runs `nix build .#site` — which cannot run inside a Nix build
sandbox — and otherwise does pure host-side analysis. Per **ADR-0028**, its home
is **`xtask`** (host orchestration/analysis), not `devtool` (in-sandbox producer).
This spec implements that placement.

## Goals / non-goals

**Goals**
- Replace the Node.js `scripts/audit-wasm-bundle` with `cargo xtask audit-wasm`.
- Behavior-preserving: same artifacts, same raw/gzip/brotli columns, same
  `--site-path` override, same machine-readable mode.
- The command is **self-documenting**: `cargo xtask audit-wasm --help` states the
  problem it solves, when to run it, and how (flags + examples).
- Pure helpers unit-tested in-crate.

**Non-goals**
- Byte-identical JSON *shape* vs. the old script (no programmatic consumers; the
  report is re-homed inside xtask's existing result envelope).
- A CI/Nix bundle-size **gate** — this stays a manual tool, not wired into
  `check`/`validate`. (A size-regression gate would be a separate, opt-in feature.)
- The other four script migrations (#29, #30, #32, #33) — each its own cycle.

## Design

### Subcommand

`Command::AuditWasm { site_path: Option<String> }` in `xtask/src/lib.rs`.

- Reuses xtask's existing **global `--json`** flag (the old script's `--json` maps
  onto it — no subcommand-local flag, which would collide with the global one).
- `--site-path PATH` overrides the `nix build` resolution (audit a prebuilt store
  path, e.g. in CI or when iterating).
- **Not** added to `check`/`validate` — it is a standalone manual tool.

### Self-documentation (the `--help` contract)

The clap doc comment on `AuditWasm` is the documentation surface, mirroring the
multi-line doc comments already on `Check`/`Validate`:

- First line → short `about`.
- Body (`long_about`) → **the problem it solves and when to use it**: "Measure the
  deterministic frontend WASM/JS bundle size (raw, gzip, brotli) from the Nix
  `.#site` build, to catch bundle-size bloat before it ships and to compare the
  effect of a change on download weight."
- `after_help` → **how to use it**, carrying the old script's `usage()` examples:
  ```
  cargo xtask audit-wasm
  cargo xtask audit-wasm --site-path /nix/store/...-jaunder-site
  cargo xtask --json audit-wasm
  ```

### New module `xtask/src/audit_wasm.rs`

**Types** (Serialize):
- `AuditReport { site_path: String, artifacts: Vec<ArtifactMetrics> }`
- `ArtifactMetrics { path: String, raw_bytes: u64, gzip_bytes: u64, brotli_bytes: u64 }`

**Pure (unit-tested):**
- `format_bytes(u64) -> String` — B/KiB/MiB/GiB, matching the script's rounding
  (0 decimals at ≥10 or for bytes, else 1).
- `parse_store_path(nix_output: &str) -> Option<String>` — last `/nix/store/…`
  line (ports the JS `.split/.filter/.at(-1)`).
- `gzip_size(&[u8]) -> u64` — `flate2` at best compression (level 9 =
  Node's `Z_BEST_COMPRESSION`).
- `brotli_size(&[u8]) -> u64` — `brotli` quality 11 (= the script's
  `BROTLI_PARAM_QUALITY: 11`).
- `render_table(&AuditReport) -> String` — the human table (header + per-artifact
  raw/gzip/brotli columns, repo-relative artifact names).

**I/O:**
- `resolve_site_path(explicit: Option<&str>) -> Result<String>` — returns
  `explicit` if set, else runs `nix build .#site --no-link --print-out-paths` and
  `parse_store_path`s it.
- `run(site_path: Option<&str>) -> Result<AuditReport>` — resolve path, require
  `pkg/jaunder_bg.wasm` + `pkg/jaunder.js`, compute metrics.

### Envelope integration

`CommandResult` already carries a typed domain payload (`coverage:
Option<CoverageReport>`). Mirror it:

- Add `audit: Option<AuditReport>` to `CommandResult` (`#[serde(skip_serializing_if
  = "Option::is_none")]`).
- `print_human` renders `render_table(report)` when `audit` is present.
- `--json` serializes the report inside the envelope (machine-readable mode).

The `lib.rs` handler runs `audit_wasm::run`, attaches the report, and pushes a
`StepResult`: `ok("audit-wasm")` on success, `fail("audit-wasm").detail(...)` when
`nix build` fails or an artifact is missing (→ exit 1). `command_name()` gains the
`"audit-wasm"` arm.

### Dependencies

Add to `xtask/Cargo.toml`: `flate2` (gzip) and `brotli`.

### Cleanup

- Delete `scripts/audit-wasm-bundle`.
- Update the three doc references to `cargo xtask audit-wasm`:
  - `CONTRIBUTING.md:136-138`
  - `docs/ARCHITECTURE.md:103` (re-home from the `scripts/` list to xtask)
  - `docs/observability.md:111-115`

### Docs maintenance (incidental)

The `docs/README.md` ADR table had silently fallen behind (stopped at 0022 while
0023–0025 existed). Restored 0023–0025 and added 0028 as part of landing this
ADR, so the table is whole again.

## Testing

- In-file `#[cfg(test)]` in `audit_wasm.rs` for the pure helpers: `format_bytes`
  boundaries; `parse_store_path` (last-line, no-match, trailing whitespace);
  `gzip_size`/`brotli_size` (compressible input compresses, deterministic);
  `render_table` golden columns. (xtask is host-only with its own unit suite — the
  ADR-0019 dialect-file coverage caveat does not apply.)
- A `result.rs` test that an `AuditReport`-bearing `CommandResult` serializes the
  `audit` field and that `print_human` includes the table (or a focused
  `render_table` assertion).
- Final gate: `cargo xtask validate --no-e2e` green.

## Sequencing

1. Add `flate2`/`brotli` deps; create `audit_wasm.rs` with pure helpers + tests
   (TDD).
2. Add the I/O (`resolve_site_path`, `run`).
3. Wire `Command::AuditWasm` + `audit` envelope field + `print_human` rendering +
   self-documenting help.
4. Delete the script; update the three docs.
5. `cargo xtask validate --no-e2e`.
