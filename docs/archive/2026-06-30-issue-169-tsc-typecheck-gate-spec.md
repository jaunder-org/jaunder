# Spec — Issue #169: TypeScript typecheck gate for `end2end/`

**Status:** approved (2026-06-30)
**Issue:** [#169](https://github.com/jaunder-org/jaunder/issues/169) (milestone: E2E test suite, label: dx)

## Problem

The e2e TypeScript under `end2end/` is never type-checked. The only TS gate is
**prettier** (formatting). `playwright test` transpiles each spec with esbuild — it
strips types and runs, it does **not** type-check the project; `end2end/tsconfig.json`
exists only for editor IntelliSense. So type errors accumulate silently. Running
`tsc -p end2end/tsconfig.json --noEmit` today surfaces **14 pre-existing errors** in
`end2end/tests/fixtures.ts`.

> **Implementation note:** the devShell pins `tsc` 5.9.3, which surfaces a **15th** error
> in `end2end/tests/otel.ts` (`randomBytes()`'s `Buffer` not assignable to `Uint8Array`)
> not seen by the older `tsc` this count was measured against — TS ≥5.7 + modern
> `@types/node` type `Buffer` as `Buffer<ArrayBufferLike>`. All 15 are fixed (see plan
> Task 2).

## Gate architecture (verified)

- `cargo xtask check` / `validate` run the **host** `static_checks` step list
  (`xtask/src/steps/static_checks.rs`) — `prettier`, `cargo fmt`, `leptosfmt`,
  `clippy`, `ert`, `elisp-fmt`, … — plus the nix `coverage` (and, for `validate`,
  `e2e`) checks. The static-check tools come from the devShell PATH.
- CI (`.github/workflows/ci.yml`) runs `nix develop .#ci -c cargo xtask validate
  --no-e2e` + the e2e matrix. It does **not** run `nix flake check`. Therefore the
  standalone nix `*-check` derivations (`prettier-check`, `ert-check`,
  `leptosfmt-check`, `elisp-fmt-check`) are **orphans** — never gated. The real
  static-check gate is the **host** `static_checks.rs`.

⇒ The typecheck belongs in the host `static_checks.rs` (the issue's ask), where it
gates on every `cargo xtask check/validate` and in CI's `Validate (no e2e)` job.

## Design

### 1. Compiler — `tsc` in the devShell

Add `pkgs.typescript` (nixpkgs 5.9.3) to `ciInputs` in `flake.nix`. `tsc` lands on
PATH for the host gate and CI's `.#ci` shell, exactly like `prettier`/`leptosfmt`/
`emacs` — nix-pinned, no `npm` for the compiler. (Project still pins
`typescript@^5.4.5` in `package.json` for IDE/standalone use; the gate uses the
devShell `tsc`. Minor version skew between the two is irrelevant for `--noEmit`.)

### 2. Type deps — reproducible, no `npm ci`

`tsc --noEmit` needs two packages resolvable under `end2end/node_modules`:

- `@playwright/test` — **already** symlinked by the devShell shellHook from
  `pkgs.playwright-test` (1.60.0; in the lean CI closure today). Unchanged.
- `@types/node` — genuinely required: `process.env`, `Buffer`, `crypto` (`randomBytes`),
  `fs` are used across `otel.ts`, `perf.ts`, `atompub.spec.ts`, `websub.ts`,
  `media.spec.ts`, `mail.ts`, `fixtures.ts`. **Not** carried by `pkgs.playwright-test`.

Extend the shellHook to **also** symlink `@types/node` into
`end2end/node_modules/@types/node`, sourced from the nix `e2ePackage` (the existing
`buildNpmPackage` that already pins `@types/node@20.12.12` — single source of truth,
offline, reproducible). No `npm ci` at gate time; zero network dependency added to the
static gate.

**Open tradeoff (accepted):** `e2ePackage` is named `jaunder-e2e`, which the cachix
pushFilter excludes — so the lean `Validate (no e2e)` CI job would build it once
(small: 3 JS packages, no browsers). Primary choice is `e2ePackage` for DRY (no second
version pin). Measure the CI cost during implementation; if material, fall back to a
dedicated tiny `fetchurl`-based `@types/node` derivation (which *would* be cached, since
its name doesn't match the pushFilter exclusion).

### 3. Gate step — host `static_checks.rs`

Add a `tsc` `StepSpec` immediately **after** `prettier`:

```
name: "tsc", program: "tsc", args: ["--noEmit", "-p", "end2end/tsconfig.json"]
```

It is **verify-only** (tsc has no autofix), so the args are identical in `Check`/`Fix`
mode — unlike the formatters. Update the `step_order_is_locked` test (insert `"tsc"`
after `"prettier"`) and add a focused unit test asserting the new spec's program/args
in both modes.

`tsconfig.json` has no `include`/`files`, so tsc compiles all `.ts` under its dir
(`end2end/tests/*.ts`), excluding `node_modules` by default. `strict: true`,
`skipLibCheck: true`. Default `lib` includes DOM, so `document`/`performance`/
`MutationObserver`/`location`/`globalThis` resolve without extra config.

### 4. Fix the 14 latent `fixtures.ts` errors

- **1 × `cacheWarmth` widening.** The `navigationSummary` `.map()` callback (line ~645)
  has no contextual return type, so the object-literal property
  `cacheWarmth: navigation.id === 1 ? "cold" : "warm"` widens its literals to `string`,
  which fails assignment to `NavigationSummary["cacheWarmth"]` (`"cold" | "warm"`).
  **Fix:** annotate the callback return — `.map((navigation): NavigationSummary => { … })`
  — so the literals are contextually typed and don't widen.
- **13 × `OtlpAttribute | null` not assignable to `OtlpAttribute`.** `otlpAttribute()`
  always returns `OtlpAttribute | null`. The `navigationEvents` builder (line ~774) is
  the one builder whose attribute array is **not** `.filter()`ed (its siblings
  `requestEvents`/`actionEvents` already are); all 13 `otlpAttribute` calls in it are
  flagged. **Fix:** add the same null-guard the siblings use —
  `.filter((attribute): attribute is NonNullable<typeof attribute> => attribute !== null)`.

These are harmless at runtime in the currently-exercised paths but are genuine type
holes the gate could not see.

## Out of scope

- The orphan nix `*-check` derivations (`prettier-check`, etc.) being ungated is a
  pre-existing observation, **not** acted on here.
- No `npm ci`-at-gate-time; no parallelism changes (#61); no Playwright config dedupe
  (#153).

## Verification

1. Prove the gate red→green: introduce a temporary type error in `fixtures.ts`, confirm
   `cargo xtask check` fails on the `tsc` step, revert.
2. `cargo xtask check` green (tsc runs, 14 errors fixed).
3. `cargo xtask validate` green (full local gate).
4. Coverage: the change touches xtask Rust (`static_checks.rs`); the new spec entry +
   unit test keep it covered. `fixtures.ts` is TypeScript (not Rust-coverage-instrumented).

## ADR

None — a conventional gate addition, no novel architectural decision.

## Acceptance (from the issue)

- [x] `cargo xtask check` runs a TypeScript type-check over `end2end/` and fails on type
      errors. (Proven red→green: a canary type error made the gate `[FAIL] tsc … exit=1`.)
- [x] The existing `fixtures.ts` errors (14) — plus a 15th in `otel.ts` surfaced by the
      pinned `tsc` 5.9.3 — are fixed; the typecheck is green.
