# TypeScript Typecheck Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `tsc --noEmit` TypeScript typecheck to the host static-check gate so `end2end/` type errors fail `cargo xtask check/validate`, and fix the 14 pre-existing `fixtures.ts` type errors.

**Architecture:** `tsc` comes from the nix devShell (`pkgs.typescript`), exactly like `prettier`/`leptosfmt`. The devShell shellHook provisions `end2end/node_modules` from the nix `e2ePackage` (full, reproducible dependency closure — `@types/node`, `undici-types`, `typescript`), preserving the existing `@playwright/test`→`pkgs.playwright-test` override. The typecheck is a verify-only `StepSpec` in `xtask/src/steps/static_checks.rs`, which is the real static-check gate (CI runs the host static-checks, not `nix flake check`).

**Tech Stack:** Rust (xtask), Nix (flake.nix), TypeScript (`end2end/tests/*.ts`), Playwright.

## Global Constraints

- No `npm ci`/network at gate time — deps come from nix (`e2ePackage`), offline + reproducible.
- `tsc` is verify-only: identical args in `Mode::Check` and `Mode::Fix` (no autofix).
- Command: `tsc --noEmit -p end2end/tsconfig.json`, placed immediately **after** `prettier` in the step list.
- No `Co-Authored-By` trailers in commits.
- Per-task gate: run via `nix develop .#ci --accept-flake-config -c cargo xtask check` so the shellHook runs (it creates the `node_modules` symlinks the typecheck needs) and the `.#ci` closure is exercised exactly as CI does. `flake.nix` is git-tracked, so nix sees the (dirty) edits.
- `e2ePackage` is named `jaunder-e2e` ⇒ excluded from the cachix pushFilter. Accept a one-time build on the lean `Validate (no e2e)` CI job (small: 3 JS packages, no browsers; npm-deps FOD is content-addressed/cached). **Measure in Task 1**; if material, apply the decoupled-derivation fallback (Task 1, "Fallback" note).

---

### Task 1: devShell — `tsc` on PATH + `end2end/node_modules` from `e2ePackage`

**Files:**
- Modify: `flake.nix` — `ciInputs` list (add `pkgs.typescript`); `shellEnv.shellHook` (replace the `@playwright/test`-only symlink with a full-closure provision).

**Interfaces:**
- Produces: `tsc` on the devShell PATH; `end2end/node_modules/` populated so `tsc --noEmit -p end2end/tsconfig.json` resolves `@playwright/test` + `@types/node` (+ `undici-types`). `@playwright/test` still resolves to `pkgs.playwright-test` (browser-driver parity / IDE intent preserved).

- [x] **Step 1: Add `pkgs.typescript` to `ciInputs`**

In `flake.nix`, inside the `ciInputs = [ … ]` list (the block beginning around line 1108), add `pkgs.typescript` in alphabetical position (between `pkgs.sqlite` and `wasm-bindgen-cli`, or adjacent — match the existing ordering):

```nix
              pkgs.sqlite
              pkgs.typescript
              wasm-bindgen-cli
```

Rationale comment is not required (the list is self-evident), but keep alpha order consistent with neighbors.

- [x] **Step 2: Replace the shellHook node_modules provisioning with the full closure**

> **Deviation (implemented):** the literal snippet below (`ln -sfn ${e2ePackage}/node_modules/* …`)
> is **not idempotent** — `@playwright` becomes a real dir, which a bare `ln -sfn …/*`
> cannot overwrite, so re-entering the shell errors (`ln: … cannot overwrite directory`;
> `rm -f … Is a directory`). CI's clean checkout never hits it, but every local re-entry
> does. The landed hook uses a `rm -rf`-before-`ln` loop over the closure entries plus the
> `@playwright` re-pin, so it is idempotent on re-entry. Verified by running the shellHook
> twice with no errors and `@playwright/test` still resolving to the nix package.

In `flake.nix`, `shellEnv.shellHook` currently is:

```nix
              shellHook = ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

                # Symlink Nix-provided Playwright into node_modules to avoid instance conflict
                # and provide IDE support without redundant disk usage.
                mkdir -p end2end/node_modules/@playwright
                ln -sfn ${pkgs.playwright-test}/lib/node_modules/@playwright/test end2end/node_modules/@playwright/test
              '';
```

Replace the node_modules block (keep the `LD_LIBRARY_PATH` line) with:

```nix
              shellHook = ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl ]}:$LD_LIBRARY_PATH"

                # Provision end2end/node_modules from the nix e2ePackage closure so the
                # devShell `tsc` (pkgs.typescript) can type-check end2end/ offline:
                # @types/node + undici-types + typescript all resolve. Then re-pin
                # @playwright/test to the nix-matched Playwright (browser-driver parity +
                # IDE support, as before) instead of e2ePackage's npm copy.
                mkdir -p end2end/node_modules
                ln -sfn ${e2ePackage}/node_modules/* end2end/node_modules/
                rm -f end2end/node_modules/@playwright
                mkdir -p end2end/node_modules/@playwright
                ln -sfn ${pkgs.playwright-test}/lib/node_modules/@playwright/test end2end/node_modules/@playwright/test
              '';
```

Notes for the implementer:
- The glob `${e2ePackage}/node_modules/*` does not match dotfiles (`.bin`, `.package-lock.json`) — that is fine; the typecheck uses the devShell `tsc`, not `node_modules/.bin/tsc`.
- `@types` is symlinked whole (it contains `node/`), so tsc's automatic `@types/*` inclusion works.
- The final three lines reproduce the prior `@playwright/test`→nix override exactly; `rm -f` clears the whole-`@playwright` symlink created by the glob so the real `@playwright/` dir + child symlink can be created.

- [x] **Step 3: Enter a fresh dev shell and run tsc directly — expect exactly the 14 known errors**

> **Deviation (measured):** the devShell pins `tsc` **5.9.3** (nixpkgs `pkgs.typescript`),
> which reports **15** errors, not 14. The 14 `fixtures.ts` errors are all present, plus a
> 15th in `end2end/tests/otel.ts:59` (`Buffer` not assignable to `Uint8Array<ArrayBufferLike>`).
> This is a TS-version effect: TS ≥5.7 + modern `@types/node` type `Buffer` as
> `Buffer<ArrayBufferLike>` and `Uint8Array` defaults to `Uint8Array<ArrayBuffer>`, so
> `randomBytes()` (a `Buffer`) no longer satisfies `bytesToHex(bytes: Uint8Array)`. The
> plan's "14" was measured against an older `tsc`. **No** module/type-resolution errors
> appeared (closure is correct). Task 2 is extended to fix all 15.

This proves the compiler is on PATH and the full type-dep closure resolves (a module-resolution failure would instead say "Cannot find module '@playwright/test'" / "Cannot find name 'process'").

Run:
```bash
nix develop .#ci --accept-flake-config -c tsc --noEmit -p end2end/tsconfig.json
```
Expected: non-zero exit, output is **only** errors in `end2end/tests/fixtures.ts` — 14 total: 1 × `Type 'string' is not assignable to type '"cold" | "warm"'` at the `navigationSummary` map, and 13 × `Type 'OtlpAttribute | null' is not assignable to type 'OtlpAttribute'` (or `Type 'null' is not assignable …`) in the `navigationEvents` builder. **No** "Cannot find module" / "Cannot find name" / "Cannot find type definition" errors. If any module/type-resolution error appears, the closure symlink is wrong — fix before proceeding.

- [x] **Step 4: Confirm `@playwright/test` still resolves to the nix package**

Run:
```bash
nix develop .#ci --accept-flake-config -c readlink end2end/node_modules/@playwright/test
```
Expected: a `/nix/store/…-playwright-test-1.60.0/lib/node_modules/@playwright/test` path (the nix package), not an `e2ePackage` path. (Preserves the prior IDE/runtime-parity intent.)

- [x] **Step 5: Measure the `e2ePackage` build cost for the lean job (decision point)**

> **Decision (measured):** entering `.#ci` warm (`e2ePackage` realized) takes ~4.9s — the
> realize cost is immaterial. **Keeping `e2ePackage`; the Fallback is not applied.**

Run:
```bash
nix build .#checks.x86_64-linux.coverage --dry-run 2>&1 | true   # sanity: store warm
time nix build --no-link --print-out-paths --accept-flake-config .#packages.x86_64-linux.e2e-checks >/dev/null 2>&1 || true
nix path-info -S "$(nix eval --raw .#packages.x86_64-linux.e2e-checks 2>/dev/null)" 2>/dev/null || true
```
Then build just the deps closure entry the shellHook references and time a *cold-ish* realize:
```bash
time nix develop .#ci --accept-flake-config -c true
```
Decision: if entering the `.#ci` shell (which now realizes `e2ePackage`) adds more than a few seconds on an otherwise-warm machine, apply the **Fallback** below before committing. Otherwise keep `e2ePackage`.

  **Fallback (decoupled + cacheable deps derivation).** Add, next to `e2ePackage` in `flake.nix` (around line 502), a second `buildNpmPackage` whose `src` is only the manifest files (so it does **not** rebuild when test files change) and whose name does **not** match the `jaunder-e2e` pushFilter (so cachix caches it):

```nix
        e2eTypecheckDeps = pkgs.buildNpmPackage {
          name = "e2e-typecheck-deps";
          src = pkgs.lib.cleanSourceWith {
            src = ./end2end;
            filter =
              path: _type:
              let
                b = baseNameOf path;
              in
              b == "package.json" || b == "package-lock.json";
          };
          npmDepsHash = "sha256-k+N5Zf+jX2wT9Q2N1yaPYngjV0qTBFWNRdZMjqeE+t0=";
          dontNpmBuild = true;
          installPhase = ''
            mkdir -p $out
            cp -r node_modules $out/
          '';
        };
```

  Then in the shellHook, replace `${e2ePackage}` with `${e2eTypecheckDeps}`. (Same `npmDepsHash` as `e2ePackage`, so no new hash to compute.) Re-run Steps 3–4.

- [x] **Step 6: Confirm the gate is still green (no typecheck step added yet)**

Run:
```bash
nix develop .#ci --accept-flake-config -c cargo xtask check
```
Expected: `ok: true` (exit 0). The `tsc` step is **not** in the gate yet, so the 14 errors do not fail it. This commit is clean.

- [x] **Step 7: Commit**

```bash
git add flake.nix
git commit -m "build(e2e): put tsc on the devShell PATH and provision end2end/node_modules from nix

Adds pkgs.typescript to ciInputs and provisions end2end/node_modules from the
e2ePackage closure (@types/node + undici-types + typescript) so the devShell tsc
can type-check end2end/ offline. Re-pins @playwright/test to the nix-matched
Playwright as before. The provisioning loop is idempotent on shell re-entry
(rm -rf before each ln) so the @playwright real dir never breaks a re-run.
Prerequisite for the tsc gate step (#169)."
```

---

### Task 2: Fix the 15 latent type errors (`fixtures.ts` ×14 + `otel.ts` ×1)

**Files:**
- Modify: `end2end/tests/fixtures.ts` — the `navigationSummary` `.map()` callback (~line 645) and the `navigationEvents` builder (~line 774).
- Modify: `end2end/tests/otel.ts` — `bytesToHex` parameter type (~line 54).

**Interfaces:**
- Consumes: `tsc` + resolvable `node_modules` from Task 1.
- Produces: `tsc --noEmit -p end2end/tsconfig.json` exits 0 (green).

- [x] **Step 1: Narrow `cacheWarmth` — annotate the map callback return type**

In `fixtures.ts`, the `navigationSummary` builder starts:

```ts
      const navigationSummary: NavigationSummary[] = navigations
        .map((navigation) => {
```

Change the callback signature to annotate its return type so the object-literal
`cacheWarmth: navigation.id === 1 ? "cold" : "warm"` is contextually typed (the
literals stop widening to `string`):

```ts
      const navigationSummary: NavigationSummary[] = navigations
        .map((navigation): NavigationSummary => {
```

(Only the `(navigation)` → `(navigation): NavigationSummary` change; the body is unchanged.)

- [x] **Step 2: Add the null-guard filter to `navigationEvents`**

In `fixtures.ts`, the `navigationEvents` builder currently is:

```ts
      const navigationEvents = topNavigations.map((navigation) =>
        makeEvent("navigation.lifecycle", endMs, [
          otlpAttribute("navigation.id", navigation.id),
          // … 13 otlpAttribute(…) calls …
          otlpAttribute("navigation.request_failed", navigation.requestFailed),
        ]),
      );
```

`otlpAttribute()` returns `OtlpAttribute | null`; `makeEvent`'s 3rd arg is
`OtlpAttribute[]`. Add the same `.filter()` type-guard the sibling `requestEvents`
and `actionEvents` builders already use — change the closing `]),` of the attribute
array to:

```ts
          otlpAttribute("navigation.request_failed", navigation.requestFailed),
        ].filter(
          (attribute): attribute is NonNullable<typeof attribute> =>
            attribute !== null,
        )),
      );
```

(i.e. the array literal `[ … ]` passed to `makeEvent` gains the trailing
`.filter((attribute): attribute is NonNullable<typeof attribute> => attribute !== null)`,
identical to `requestEvents`/`actionEvents`.)

- [x] **Step 2b: Coerce the `Buffer` to a plain `Uint8Array` at the call site (`otel.ts`)**

In `end2end/tests/otel.ts`, `bytesToHex` is called once, from `randomHex`, with
`randomBytes(byteLength)` — a node `Buffer`, typed `Buffer<ArrayBufferLike>` under modern
`@types/node`. The parameter `bytes: Uint8Array` defaults to `Uint8Array<ArrayBuffer>`
(TS ≥5.7), so the `ArrayBufferLike` (incl. `SharedArrayBuffer`) doesn't satisfy
`ArrayBuffer`.

> **Correction:** widening the parameter to `Uint8Array<ArrayBufferLike>` does **not**
> fix it — `Buffer` is still not assignable to `Uint8Array<ArrayBufferLike>` (their
> `slice(...).buffer` return types diverge). Instead coerce at the call site so the
> argument is a plain `Uint8Array<ArrayBuffer>` and `bytesToHex`'s signature stays
> DOM-agnostic:

```ts
return bytesToHex(Uint8Array.from(randomBytes(byteLength)));
```

(`bytesToHex`'s `bytes: Uint8Array` parameter is unchanged; only `randomHex`'s call site
gains the `Uint8Array.from(...)` copy, with an intent comment.)

- [x] **Step 3: Run tsc — expect green**

Run:
```bash
nix develop .#ci --accept-flake-config -c tsc --noEmit -p end2end/tsconfig.json
```
Expected: exit 0, no output.

- [x] **Step 4: Confirm prettier formatting is preserved**

Run:
```bash
nix develop .#ci --accept-flake-config -c prettier --check end2end
```
Expected: "All matched files use Prettier code style!" (exit 0). If it reports the
file, run `prettier -w end2end/tests/fixtures.ts` and re-check.

- [x] **Step 5: Run the gate**

Run:
```bash
nix develop .#ci --accept-flake-config -c cargo xtask check
```
Expected: `ok: true` (exit 0). (Still no `tsc` gate step; this verifies fmt/clippy/etc. unaffected.) — run as the pre-commit hook below; PASSED in 156s.

- [x] **Step 6: Commit** — landed as `cb5cba2`.

```bash
git add end2end/tests/fixtures.ts end2end/tests/otel.ts
git commit -m "fix(e2e): resolve 15 latent type errors surfaced by tsc --noEmit

Annotate the navigationSummary map callback return type so cacheWarmth stays
\"cold\" | \"warm\" instead of widening to string, and add the sibling null-guard
filter to the navigationEvents attribute array (otlpAttribute returns
OtlpAttribute | null) — 14 fixtures.ts errors. Also coerce randomBytes()'s
Buffer to a plain Uint8Array at bytesToHex's call site so it satisfies the
Uint8Array<ArrayBuffer> parameter under TS 5.9 / modern @types/node (otel.ts).
tsc --noEmit is now green. (#169)"
```

---

### Task 3: Add the `tsc` static-check step to the gate

**Files:**
- Modify: `xtask/src/steps/static_checks.rs` — add the `tsc` `StepSpec` after `prettier`; update the `step_order_is_locked` test; add a focused unit test.

**Interfaces:**
- Consumes: the green typecheck from Task 2; `tsc` on PATH + `node_modules` from Task 1.
- Produces: `cargo xtask check/validate` runs `tsc --noEmit -p end2end/tsconfig.json` and fails on type errors.

- [x] **Step 1: Add the failing unit test for the new spec**

In `xtask/src/steps/static_checks.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn tsc_typechecks_in_both_modes() {
        for mode in [Mode::Check, Mode::Fix] {
            let s = specs(mode);
            let tsc = find(&s, "tsc");
            assert_eq!(tsc.program, "tsc");
            assert_eq!(tsc.args, ["--noEmit", "-p", "end2end/tsconfig.json"]);
        }
    }
```

Also update the existing `step_order_is_locked` expected list to insert `"tsc"`
immediately after `"prettier"`:

```rust
        let expected = [
            "fmt",
            "leptosfmt",
            "prettier",
            "tsc",
            "elisp-fmt",
            "ert",
            "cargo-deny",
            "clippy",
            "tools-fmt",
            "tools-clippy",
            "xtask-fmt",
            "xtask-clippy",
        ];
```

- [x] **Step 2: Run the tests — verify they fail**

Run:
```bash
cargo nextest run --manifest-path xtask/Cargo.toml static_checks
```
Expected: FAIL — `tsc_typechecks_in_both_modes` panics with "step present" (no `tsc` step yet), and `step_order_is_locked` fails on the mismatched list.

- [x] **Step 3: Add the `tsc` StepSpec after `prettier`**

In `static_checks.rs`, in the `vec![ … ]` returned by `specs(mode)`, insert a new
`StepSpec` immediately after the `prettier` entry (after the block ending at the
`prettier` `StepSpec { … }`). `tsc` is verify-only, so no `Mode` switch — the same
args in both modes:

```rust
        StepSpec {
            name: "prettier",
            program: "prettier",
            args: prettier_args,
        },
        // tsc — type-check end2end/ (verify-only; no autofix, so identical in both
        // modes). The compiler comes from the devShell (pkgs.typescript) and the
        // type-dep closure from the shellHook-provisioned end2end/node_modules.
        StepSpec {
            name: "tsc",
            program: "tsc",
            args: vec!["--noEmit", "-p", "end2end/tsconfig.json"],
        },
```

- [x] **Step 4: Run the unit tests — verify they pass**

Run:
```bash
cargo nextest run --manifest-path xtask/Cargo.toml static_checks
```
Expected: PASS (all `static_checks` tests, including the two updated/added).

- [x] **Step 5: Run the full gate — verify it runs tsc and is green** (proven by the Step 7 commit's pre-commit gate: all steps `[ ok ]` incl. `tsc`).

Run:
```bash
nix develop .#ci --accept-flake-config -c cargo xtask check
```
Expected: `ok: true`. Confirm `tsc` actually ran:
```bash
jq '.steps[] | select(.name=="tsc")' .xtask/last-result.json
```
Expected: an object with `"name":"tsc"`, `"ok":true`.

- [x] **Step 6: Prove the gate catches a type error (red→green)** (verified: a `const _typecheckCanary: number = "not a number";` in `example.spec.ts` made the gate `[FAIL] tsc — example.spec.ts(4,7): error TS2322`, `ok=false exit=1`; reverted, gate green again.)

Temporarily break a type, confirm the gate fails on the `tsc` step, then revert:
```bash
# introduce an error: change a number attr to a bad type, e.g. add a stray line
printf '\nconst _typecheckCanary: number = "not a number";\n' >> end2end/tests/example.spec.ts
nix develop .#ci --accept-flake-config -c cargo xtask check ; echo "exit=$?"
```
Expected: non-zero exit; `jq '.steps[] | select(.name=="tsc")' .xtask/last-result.json` shows `"ok":false`. Then revert:
```bash
git checkout end2end/tests/example.spec.ts
nix develop .#ci --accept-flake-config -c cargo xtask check
```
Expected: back to `ok: true`.

- [x] **Step 7: Commit**

```bash
git add xtask/src/steps/static_checks.rs
git commit -m "feat(xtask): add a tsc --noEmit typecheck to the static-check gate

Adds a verify-only \`tsc -p end2end/tsconfig.json --noEmit\` StepSpec after
prettier, so end2end/ TypeScript type errors fail cargo xtask check/validate
(and CI's Validate (no e2e) job). Locks the step into step_order_is_locked and
adds a focused spec test. Closes #169."
```

---

## Final verification (before ship)

- [ ] Run the full local gate clean (deferred to **ship**, which archives the planning
      docs to clean the tree then runs full `validate` incl. e2e):
```bash
git status --porcelain   # must be empty (validate refuses a dirty tree)
nix develop .#ci --accept-flake-config -c cargo xtask validate --no-e2e
```
Expected: `ok: true`. (Full `validate` incl. e2e is the ship-time gate; `--no-e2e` mirrors CI's `Validate (no e2e)` job and is sufficient for this change, which touches no runtime/server code.)

- [x] Acceptance check (from #169):
  - `cargo xtask check` runs a TS type-check over `end2end/` and fails on type errors — proven in Task 3 Step 6.
  - The 15 type errors (14 `fixtures.ts` + 1 `otel.ts`) are fixed; the typecheck is green — proven in Task 2 Step 3.

## Self-review notes

- **Spec coverage:** compiler-on-PATH (Task 1 S1), type-deps reproducible/no-npm-ci (Task 1 S2 + Fallback), host gate step verify-only after prettier (Task 3), 14-error fix split 1+13 (Task 2), red→green proof (Task 3 S6), no ADR. All spec sections map to a task.
- **Placeholders:** none — every code/diff is concrete; the Fallback is fully specified with the real `npmDepsHash`.
- **Type/name consistency:** step name `"tsc"`, program `"tsc"`, args `["--noEmit","-p","end2end/tsconfig.json"]` used identically in the StepSpec and both tests; `step_order_is_locked` list matches the insertion point.
