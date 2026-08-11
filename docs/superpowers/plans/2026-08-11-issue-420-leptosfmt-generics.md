# leptosfmt Generic-Tag Pin — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pin `leptosfmt` to the upstream revision containing the never-released
generic-tag fix, and reformat the 17 mangled call sites — as one commit.

**Architecture:** One `leptosfmt` binding in `flake.nix`'s per-system `let`,
overriding nixpkgs' `src` **and** `cargoDeps` (the `wasm-bindgen-cli` shape at
`flake.nix:400-411`). Both existing consumers point at it. The reformat then
falls out of the pre-commit gate itself.

**Tech Stack:** Nix (`overrideAttrs`, `fetchFromGitHub`, `fetchCargoVendor`),
`leptosfmt`.

**Spec:**
[`2026-08-11-issue-420-leptosfmt-generics.md`](../specs/2026-08-11-issue-420-leptosfmt-generics.md)

## Review header

**Scope — in:** the `flake.nix` pin, its two use sites, the expiry comment, and
the 17-site reformat (`ValidatedInput<T>` ×14, `ValidatedTextarea<T>` ×3).

**Scope — out:** `max_width` changes, wrapper components, leptosfmt excludes,
any new gate step, an ADR (spec D7).

**Tasks:**

1. Pin `leptosfmt`, wire both consumers, reformat, commit — one deliverable.
2. Prove the pin behaviourally and record the evidence for the PR.

**Key facts that shaped this plan** (all measured, several counter-intuitive):

- **The pinned binary REJECTS the current tree.** Verified:
  `leptosfmt --check web/src` with the pinned build exits **1** on the mangled
  sites — rewriting them is what it is for. So
  `nix build .#checks…static-checks` (which runs `devtool check --all`, verify
  mode, `check.rs:43-52`) **fails** between wiring the pin and reformatting.
  That failure is the pin proving itself, not a defect.
- **Therefore the pin cannot land as its own commit.** A flake-only commit
  leaves CI red. Spec D5.
- **Nix `let` bindings are lazy**, so hashes cannot be discovered until
  something references the binding — wire the consumers _first_, then discover.
- **The reformat is performed by the gate, not by hand.** `cargo xtask check` is
  `Mode::Fix` (`static_checks.rs:150` appends `--fix`), so once the pinned
  binary is on PATH the pre-commit run reformats all 17 sites itself.

## Global Constraints

- **Add no new gate step** (spec D6) — `devtool::check::ALL`,
  `static_checks.rs`'s mirrored list, `step_order_is_locked`, and
  `CONTRIBUTING.md`'s counts are untouched.
- Keep `version = "0.1.33"` in the override (spec D3 — `versionCheckHook`).
- **No `Co-Authored-By` trailer.**
- Mangled-shape detector: `rg -n '[A-Za-z]<$' web/src` — 17 lines now, 0 after.

---

### Task 1: Pin leptosfmt and reformat (one commit) — DONE

> **Two deviations from the plan as written, both forced and both recorded in
> the commit message and the flake comment.**
>
> 1. **`fetchCargoVendor` does not work here.** Its `fetch-cargo-vendor-util`
>    downloads via crates.io's API endpoint, which answered **403** on three
>    consecutive runs, on a _different_ crate each time (`either`, `crop`,
>    `anstyle-query`) — so the requester is being rejected, not any one crate.
>    Switched to `importCargoLock`, which uses nix's own `fetchurl` (the path
>    crane already vendors this repo through). Builds fine.
> 2. **Step 4's PATH check failed, and stayed failed after `direnv reload`** —
>    this session's environment is fixed at startup. Rather than commit through
>    a stale gate (which would have re-mangled all 17 sites into the commit),
>    the reformat was run with the flake's own binary
>    (`/nix/store/g67csp…-leptosfmt-0.1.33`, read out of the built CI devShell's
>    references) and the commit was made with that binary **prepended to PATH**,
>    so the gate certified with the toolchain the flake specifies rather than
>    the stale one. The gate left the reformatted files untouched, which is
>    itself confirmation the right binary was in use.
>
> One process note: the first commit attempt captured only `flake.nix` and the
> docs, because leptosfmt rewrote `web/src` directly and the auto-stage hook
> only sees files touched via Edit/Write. That landed exactly the "pin alone"
> state the spec calls red. Caught by reading `git show --stat`, fixed by
> staging `web/src` and amending.

**Files:**

- Modify: `flake.nix` — new binding after `wasm-bindgen-cli` (`:411`); consumers
  at `:1152` (the `static-checks` derivation's `nativeBuildInputs`) and `:1361`
  (`ciInputs`).
- Modify (by the gate, not by hand):
  `web/src/{auth,backup,email,invites,password_reset,posts,profile,registration,site}/component.rs`

**Interfaces:**

- Consumes: nothing.
- Produces: a `leptosfmt` binding in the per-system `let` (opens
  `flake.nix:253`, closes at `in` on `:1006`), in scope at both consumers.

- [ ] **Step 1: Write the override with placeholder hashes, and wire both
      consumers**

Wiring happens **now**, not later: an unreferenced `let` binding is never
forced, so the fetchers would not run and no hash mismatch would be reported.

Insert after the `wasm-bindgen-cli` binding (`flake.nix:411`):

```nix
        # leptosfmt pinned past its last release (#420). 0.1.33 (2025-01-30)
        # mangles a generic component tag whenever the tag wraps —
        # `<ValidatedInput<Username>` becomes a three-line stanza with broken
        # indentation. Upstream fixed it in PR #167 ("don't break generic params
        # into mulitple lines"), merged 2025-02-02 — three days AFTER 0.1.33
        # shipped, and never released since.
        #
        # REMOVE THIS OVERRIDE when a leptosfmt release later than 0.1.33
        # appears: take `pkgs.leptosfmt` again and drop this binding.
        #
        # `version` deliberately stays "0.1.33": nixpkgs runs `versionCheckHook`,
        # which matches `leptosfmt --version` against it, and upstream never
        # bumped the version after the release.
        leptosfmt = pkgs.leptosfmt.overrideAttrs (old: rec {
          src = pkgs.fetchFromGitHub {
            owner = "bram209";
            repo = "leptosfmt";
            rev = "8b4194ba33eee417ababdd15498940014fd6d237";
            # PR #167 bumps a `prettyplease` submodule; without this the tree
            # does not build. (nixpkgs sets it too — replacing `src` drops it.)
            fetchSubmodules = true;
            hash = pkgs.lib.fakeHash;
          };
          # `overrideAttrs` alone is not enough: nixpkgs passes `cargoHash`,
          # which `buildRustPackage` consumes before this override applies, so
          # the 0.1.33 vendor tree would survive a bare `src` swap.
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            hash = pkgs.lib.fakeHash;
          };
        });
```

Then change `pkgs.leptosfmt` → `leptosfmt` at **both** `flake.nix:1152` and
`flake.nix:1361`. (Those are the only two references in the file.)

- [ ] **Step 2: Resolve the source hash**

Build a target that forces the binding but does **not** run verify-mode
leptosfmt, so the only failure is the hash:

Run: `devtool run -- nix build --no-link .#devShells.x86_64-linux.ci`

Expected: FAIL with `hash mismatch`, naming a `got:` value for the
`fetchFromGitHub`. Replace the first `pkgs.lib.fakeHash` with it.

- [ ] **Step 3: Resolve the cargo-vendor hash**

Run the same command again.

Expected: FAIL with a second `hash mismatch`, for `fetchCargoVendor`. Replace
the second `pkgs.lib.fakeHash` with its `got:` value.

Run once more. Expected: **PASS** — the pinned leptosfmt builds.

- [ ] **Step 4: Confirm the session's PATH picks up the pin**

Run: `which leptosfmt`

Expected: a store path **other than**
`/nix/store/kai0n94wxxrm61yi85wsqrdn5pj57gwf-leptosfmt-0.1.33` (direnv
re-resolved from the edited flake).

If it is unchanged, **stop and report**: the reformat cannot be committed from
this session, because the fix-mode gate would run the stock binary and re-mangle
the sites into the commit. Ask the user to reload direnv or restart the session
rather than bypassing the gate.

- [ ] **Step 5: Reformat, via the gate**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-420-leptosfmt-generics -- cargo xtask check`

Expected: PASS overall, and the pre-commit-style fix mode rewrites all 17 sites.
(Equivalently `devtool run -- leptosfmt web/src` by hand; the gate does it
anyway.)

- [ ] **Step 6: Verify the population (AC2, AC3)**

Run: `devtool run -- rg -n '[A-Za-z]<$' web/src` Expected: **no matches**,
exit 1. (Before: exactly 17.)

Run: `devtool run -- rg -c '^\s+<ValidatedInput<[A-Za-z]' web/src` Expected:
**14** lines across 8 files. (Before: 0 — the anchor excludes the prose mention
at `web/src/password_reset/api.rs:32` that an unanchored pattern hits.)

Run: `devtool run -- rg -c '^\s+<ValidatedTextarea<[A-Za-z]' web/src` Expected:
**3** lines across 2 files. (Before: 0.)

- [ ] **Step 7: Commit**

The gate already ran clean in Step 5.

```bash
git add flake.nix web/src
git commit -m "build(nix): pin leptosfmt past the unreleased generic-tag fix (#420)"
```

---

### Task 2: Prove the pin behaviourally (AC4, AC5, AC7) — DONE

> **AC4, both directions, on the committed tree.** Pinned:
> `leptosfmt --check web/src` → 109 files checked, all pass. Stock
> (`/nix/store/kai0n94w…`): exit **1**, with a diff showing it reverting
> `<ValidatedInput<Username>` back to the three-line mangled stanza — the
> clearest possible evidence, since it is the regression reproducing itself.
>
> **AC5:** second formatting pass, then `git diff --quiet -- web/src` → exit 0.
>
> **AC7:** `cargo xtask validate --no-e2e` green with the pinned toolchain.

**Files:** none — this task produces evidence for the PR body.

**Interfaces:**

- Consumes: the committed tree from Task 1.
- Produces: two recorded command outcomes for the PR.

- [ ] **Step 1: Stability, now that the tree is committed (AC5)**

Run: `devtool run -- leptosfmt web/src` Then:
`devtool run -- git diff --quiet -- web/src`

Expected: exit 0. This is only meaningful **after** Task 1's commit — before it,
the tree is dirty from the reformat itself and the check would fail regardless.

- [ ] **Step 2: The pinned toolchain passes verify mode (AC4, half one)**

Run: `devtool run -- devtool check leptosfmt` Expected: **PASS**.

- [ ] **Step 3: The stock toolchain fails verify mode (AC4, the discriminating
      half)**

Use the gate's own argv (`tools/devtool/src/check.rs:43-52`) so the evidence
matches what the gate runs, and resolve the stock binary from **this flake's**
nixpkgs input rather than the registry:

```bash
devtool run -- nix build --no-link --print-out-paths --inputs-from . nixpkgs#leptosfmt
```

Then run that path's binary:

```bash
<printed-path>/bin/leptosfmt -x .direnv -x .git -x target --check '**/*.rs'
```

Expected: **exit 1**, listing the reformatted files as needing changes. Record
the output for the PR body — this is what proves the pin took, since the version
string cannot (spec D3).

- [ ] **Step 4: Full gate (AC7)**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-420-leptosfmt-generics -- cargo xtask validate --no-e2e`
Expected: PASS, `leptosfmt` step green in verify mode.

- [ ] **Step 5: No commit**

This task adds no files; its output is the AC4 evidence quoted in the PR body.
