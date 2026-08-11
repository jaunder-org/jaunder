# #420 — pin leptosfmt past the generic-tag fix

Issue: [#420](https://github.com/jaunder-org/jaunder/issues/420). Milestone:
Developer tooling & DX. Provenance: #414 (`ValidatedInput<T>`, the first generic
component), #568 (`ValidatedTextarea<T>`, the second), the #404 family.

## Summary

`leptosfmt` mangles a generic component tag whenever the tag has to wrap:

```rust
<ValidatedInput<
Username,
>
    label="Username"
    …
/>
```

It compiles and the pre-commit gate stays green — leptosfmt is idempotent on its
own output — so this is cosmetic. But it is now in **17 call sites across 9
verticals**, and recurs at every adoption.

**Two** generic components are affected, not one:

| component              | sites | verticals                                                                                 |
| ---------------------- | ----- | ----------------------------------------------------------------------------------------- |
| `ValidatedInput<T>`    | 14    | `auth`, `backup`, `email`, `invites`, `password_reset`, `profile`, `registration`, `site` |
| `ValidatedTextarea<T>` | 3     | `posts` (×2), `profile`                                                                   |

Detector for the mangled shape: `rg -n '[A-Za-z]<$' web/src` → exactly these 17
lines. (A generic tag is only ever broken this way, so a line _ending_ in `<` is
the signature.)

## What was measured

The investigation changed the shape of the fix, so the findings are the spec's
foundation:

- **The trigger is line width, not generics as such.** When a tag fits on one
  line the generic list is left alone; when the tag must wrap for its
  attributes, leptosfmt wraps the _generic parameter list too_, and mis-indents
  it. Verified with `-m 200`, under which the real call sites format to a single
  clean line.
- **It cannot be fixed at the call site.** Hand-writing the clean shape into
  `web/src/auth/component.rs` and re-running leptosfmt **re-mangled it**. Any
  "just write it this way" convention would be undone by the next gate run.
- **There is no configuration escape.** `leptosfmt --help` (0.1.33) offers
  `max-width`, `tab-spaces`, whole-file `excludes`, `config-file` and
  `override-macro-names`. No skip directive, nothing construct-scoped. Raising
  `max_width` does clear it, and reformats the entire codebase into ~150-column
  lines — rejected.
- **Upstream already fixed it, and never released the fix.**
  [PR #167](https://github.com/bram209/leptosfmt/pull/167), _"fix: don't break
  generic params into mulitple lines"_, resolving
  [#156](https://github.com/bram209/leptosfmt/issues/156), merged
  **2025-02-02**. Release **0.1.33** shipped **2025-01-30** — three days
  earlier. It is still the latest release; upstream has published nothing since.
- **The fix works on our code.** Built upstream `main`
  (`8b4194ba33eee417ababdd15498940014fd6d237`, 2025-03-21) and ran it on a copy
  of `web/src/auth/component.rs`: both mangled sites became the clean shape.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                                                                                                                         |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | Pin `leptosfmt` to `bram209/leptosfmt` at `8b4194ba33eee417ababdd15498940014fd6d237` (upstream's last commit; contains PR #167). One binding in `flake.nix`'s per-system `let` (the one holding the `cargo-crap` and `wasm-bindgen-cli` pins), reaching both use sites.                                                                                                          |
| **D2** | **Mechanism: override `src` _and_ `cargoDeps`**, following the `wasm-bindgen-cli` pin already in this flake (`flake.nix:400-411`). `overrideAttrs` alone is not enough — nixpkgs' `leptosfmt` passes `cargoHash`, which `buildRustPackage` consumes _before_ `overrideAttrs` applies, so replacing only `src` keeps the 0.1.33 vendor tree.                                      |
| **D3** | **Leave `version` at `"0.1.33"`.** nixpkgs' package runs `versionCheckHook`, which matches `leptosfmt --version` against `version`; upstream never bumped the version after the release, so any "more honest" string breaks the build.                                                                                                                                           |
| **D4** | The override re-specifies `fetchSubmodules = true`. nixpkgs already sets it, but replacing `src` wholesale drops it, and PR #167 bumps a `prettyplease` submodule — without it the tree does not build.                                                                                                                                                                          |
| **D5** | **The pin and the reformat are one commit**, covering all 17 sites and both components. Not merely "one deliverable": the pin alone leaves the tree in a state the verify-mode gate **rejects** (the pinned binary wants to rewrite the mangled sites, so CI would be red on that commit), and the reformat alone is undone by the next fix-mode gate run. They cannot be split. |
| **D6** | **The pin is proved by the existing verify-mode gate, not by a new check and not by version.** Once the sites are clean, `cargo xtask validate`'s read-only `leptosfmt` step fails against unpinned 0.1.33 (it wants to re-mangle them) and passes against the pin. No new gate step, so no `devtool::check::ALL` / `static_checks` / `CONTRIBUTING` churn.                      |
| **D7** | No ADR. A dev-time formatter pin with a stated expiry, changing whitespace only — the same shape as the unrecorded `cargo-crap` (`flake.nix:358`) and `wasm-bindgen-cli` (`:400`) pins. The comment at D8 is the durable record.                                                                                                                                                 |
| **D8** | The override carries a comment naming PR #167, release 0.1.33, and #420, and the removal condition: **a leptosfmt release later than 0.1.33**. This is a comment, not an enforced gate — no more is claimed.                                                                                                                                                                     |
| **D9** | Out of scope: raising `max_width`, per-type wrapper components, excluding files from leptosfmt. Each considered and rejected above.                                                                                                                                                                                                                                              |

## Acceptance criteria

- **AC1 — the pin is in place.** `flake.nix` builds `leptosfmt` from rev
  `8b4194ba33eee417ababdd15498940014fd6d237` with `fetchSubmodules = true` and
  an overridden `cargoDeps`, and **both** current consumers use it: `ciInputs`
  (`flake.nix:1361`, feeding the devShells) and the `static-checks` check
  derivation's `nativeBuildInputs` (`flake.nix:1152`, which runs
  `devtool check --all`).

- **AC2 — every mangled site is gone.** `rg -n '[A-Za-z]<$' web/src` returns
  **no matches**. (Before: exactly 17.) This is the whole population — both
  components, all 9 verticals.

- **AC3 — the clean shape is what landed.** Anchored to the tag's line position,
  so a prose mention cannot satisfy it:
  `rg -c '^\s+<ValidatedInput<[A-Za-z]' web/src` reports **14** lines across 8
  files, and `rg -c '^\s+<ValidatedTextarea<[A-Za-z]' web/src` reports **3**
  across 2. Both are **0** before the change, so the criterion discriminates.

  (An unanchored `<ValidatedInput<[A-Za-z]` would over-match: it already hits a
  prose comment at `web/src/password_reset/api.rs:32`.)

- **AC4 — the pin is proved behaviourally.** Demonstrated, and recorded in the
  PR: on the reformatted tree, the **verify-mode** leptosfmt step
  (`cargo xtask validate --no-e2e`, or `devtool check leptosfmt` without
  `--fix`) **fails** with nixpkgs' unpinned 0.1.33 and **passes** with the pin.
  A version-string assertion is explicitly not acceptable — the pinned binary
  still reports `leptosfmt 0.1.33` (D3).

  Note `cargo xtask check` runs `Mode::Fix` (`static_checks.rs:150` appends
  `--fix`), so it auto-repairs and cannot discriminate. Verify mode is the
  criterion.

- **AC5 — the reformat is stable.** After the reformat is **committed**, running
  the pinned leptosfmt over `web/src` again leaves the tree clean
  (`git diff --quiet -- web/src`). Measuring this before the commit cannot work:
  the tree is already dirty from the reformat itself, so the check would fail
  regardless of whether the second pass changed anything.

- **AC6 — the expiry is written down.** The override carries the D8 comment.

- **AC7 — the gate is green.** `cargo xtask validate --no-e2e` passes.

## Risks

- **We track an unreleased commit.** Accepted: the alternative is an indefinite
  wait on a dormant upstream while the site count grows. The rev is immutable
  and the expiry condition is written down.
- **Hashes to obtain.** Written expecting two (source + cargo vendor). As
  delivered there is **one**: `fetchCargoVendor` proved unusable here (crates.io
  answers 403 to its downloader), so vendoring went through `importCargoLock`,
  which derives from the lockfile and needs no hash of its own. The remaining
  source hash was independently re-verified with `nix-prefetch-git`: a clean
  clone reproduces it exactly, so it is not an artifact of one machine's store.
  A wrong hash fails loudly rather than silently using the old binary.
- **A future nixpkgs change to `leptosfmt`'s build shape** could break the
  override — loudly (the devShell stops building), not silently.
- **The comment is not enforced.** Nothing scans for expiry conditions, so the
  pin could outlive its usefulness unnoticed. Same as the two existing pins;
  accepted rather than solved here.
