# Issue #1949 — update ammonia for RUSTSEC-2026-0533

## Outcome

Every Cargo lockfile in the repo resolves `ammonia` to a version patched for
RUSTSEC-2026-0533 (`>=4.2.5`), so `cargo-deny` advisories pass and the enforced
pre-commit gate accepts commits again. No sanitizer behavior changes.

## Load-bearing decisions

- **Raise the manifest floor, not only the lockfiles.** The root workspace
  `Cargo.toml` requirement moves from `4.2.4` to `4.2.5`. `host` and `common`
  inherit it via `ammonia.workspace = true`; `xtask/` and `tools/` reach ammonia
  only through path deps on those crates, so the one floor governs all three
  lockfiles and a future `cargo update`/lock regeneration cannot fall back to a
  vulnerable release.
- **Update all three lockfiles** (`Cargo.lock`, `xtask/Cargo.lock`,
  `tools/Cargo.lock`) with a targeted `cargo update -p ammonia`; no unrelated
  dependency moves.
- **Exposure confirmed low, no policy change.** The body policy
  (`Builder::default()` + `audio`/`video`/`source`/`track`) admits neither
  `math` nor `svg`; the title policy (`Builder::empty()` + inline tags) lists
  `math`, `svg`, `script`, `style` as clean-content tags; the `Builder::empty()`
  text strippers (`host/src/render.rs`, `host/src/code_highlight.rs`) admit no
  tags. None admits the MathML integration points or `foreignObject` the exploit
  needs. The upgrade is defense in depth, not a remediation of a live hole.

## Acceptance

- `grep -A1 'name = "ammonia"'` shows `4.2.5` (or later) in all three lockfiles;
  root `Cargo.toml` requires `4.2.5`.
- The diff touches only the manifest line and the `ammonia` lockfile entries.
- `just precommit-broad-host` (including `cargo-deny`) passes.
- Existing sanitizer tests in `common` and `host` pass unchanged.

## Boundaries

- No changes to sanitizer allowlists or new MathML/SVG regression tests — the
  policies never admit those namespaces.
- No other dependency upgrades or advisory-ignore entries.
