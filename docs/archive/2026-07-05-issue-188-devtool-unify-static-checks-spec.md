# Spec — #188: unify static checks via `devtool` (host + nix share one implementation)

- **Issue:** jaunder-org/jaunder#188 (milestone "Verify-gate hardening"); aligns
  with the Devtool migration milestone (#3). Reshaped from "delete the redundant
  nix siblings".
- **Discovered during:** #185 (Markdown under prettier), the concrete
  divergence.
- **Status:** approved

## Context

Seven static checks are specified **twice** — a host `StepSpec` in
`xtask/src/steps/static_checks.rs` **and** a hermetic nix `*-check` `runCommand`
— kept in lockstep by hand. #185 is exactly how that diverges (host prettier
extended to Markdown; the nix sibling wasn't). Deleting the nix siblings (the
original framing) would fix the duplication but strip static checks out of
`nix flake check`, a standard entry point.

Instead, **eliminate the duplication, not the coverage**: move the check
invocations into `tools/devtool` — the in-sandbox tool nix already builds
(`devtoolBin`) and `coverage` already runs (`devtool coverage emit`), whose doc
says it exists for "sandboxes where `xtask` (host-only) is unavailable …
deliberately extensible." Then the host `xtask` ladder and a nix `static-checks`
derivation both call **one** `devtool check` implementation.

## Scope: cleave by whether the check compiles

The static checks split cleanly on **whether they invoke `cargo` to compile** —
which determines whether a nix check needs crane's offline vendored-deps
plumbing. Unify only the **non-compiling** ones (they're the hand-written
`runCommand` siblings that caused the #185 divergence, and a `runCommand` over
source needs no vendoring); leave the compiling ones on their existing crane
derivations.

**Migrate into `devtool` (non-compiling — no `cargo` build):**

| check       | tool                                            | src it needs                                            |
| ----------- | ----------------------------------------------- | ------------------------------------------------------- |
| `fmt`       | `cargo fmt --check` (rustfmt only — no compile) | rust source                                             |
| `leptosfmt` | `leptosfmt … --check`                           | rust source                                             |
| `prettier`  | `prettier --check end2end **/*.md`              | `end2end/` + all `*.md`                                 |
| `ert`       | `emacs --batch … run-tests.el`                  | `elisp/`                                                |
| `elisp-fmt` | `emacs --batch … jaunder-fmt-check`             | `elisp/`                                                |
| `tsc`       | `tsc --noEmit`                                  | `end2end/` + `end2end/node_modules` (from `e2ePackage`) |
| `tools-fmt` | `cargo fmt --check` on `tools/` (no compile)    | `tools/` source                                         |

**Keep as their existing crane derivations (compiling — need offline vendored
deps):** `clippy` (`craneLib.cargoClippy`) and `deny` (`craneLib.cargoDeny`,
which deliberately runs a different, offline-safe subset than the host's
advisory-fetching `cargo deny check`). These stay exactly as-is — untouched by
this issue. Their host StepSpecs also stay native.

**Stay host-only (no nix check):**

- `tools-clippy` — compiles the `tools/` workspace, which has **no vendoring
  infra** (its own `Cargo.lock`; the flake vendors only the app; cargo's
  vendoring is per-`CARGO_HOME`). Not worth building that infra; stays a native
  host StepSpec.
- `xtask-fmt`, `xtask-clippy` — lint the `xtask/` crate, **deliberately excluded
  from the flake source** (the discipline that saves ~82% of xtask-touching
  commits from busting the coverage/e2e cache). Nix structurally cannot see
  `xtask/` source. Host-only forever.

`tsc-deps` (host provisioning of `end2end/node_modules`) is folded into the
`tsc` check.

## Decisions (resolved in design interview)

1. **`devtool` owns the definitions + execution of the 7 non-compiling checks.**
   A new `devtool check` command: `devtool check <name> [--fix]` runs one;
   `devtool check --all [--fix]` runs all 7. The **`--fix` flag** (not a
   positional) makes the formatters (`fmt`, `leptosfmt`, `prettier`,
   `elisp-fmt`, `tools-fmt`) auto-fix; **default is verify** (used by `validate`
   and nix). The non-formatters (`ert`, `tsc`) ignore `--fix`. Each of the 7
   checks' tool + args live **once**, here.
2. **Host integration keeps `xtask`'s envelope.** In `static_checks.rs`, each
   migrated check's `StepSpec` changes from
   `{program: "cargo", args: ["clippy", …]}` to a `devtool check <name> <mode>`
   invocation. `xtask`'s runner, per-step `StepResult`s, `[ok]`/`[FAIL]` output,
   and JSON sidecar are **unchanged** — the tool+args just move behind
   `devtool`. No structured-output parsing. `xtask-fmt`/`xtask-clippy` stay as
   native `cargo` StepSpecs.
   - **The host invokes the _current-source_ `devtool`, not a prebuilt binary**,
     so a local edit to a check is reflected immediately (consistent with
     `xtask` being rebuilt from source every run): the StepSpec runs
     `cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check <name> <mode>`
     (cargo caches the build; devtool rebuilds only when `tools/` changes). The
     **nix** `static-checks` derivation instead uses the prebuilt `devtoolBin`
     on `PATH`. Same `check` subcommand both places; only the launcher differs.
3. **Nix: one `static-checks` `runCommand` for the 7 non-compiling checks;
   `clippy`/`deny` stay crane.** A single `pkgs.runCommand "static-checks"` —
   deliberately **not** crane, since none of the 7 compiles — with
   `nativeBuildInputs = [devtoolBin, rustToolchain (rustfmt), leptosfmt, prettier, nodejs, emacsForCi]`,
   running `devtool check --all` (default = verify) over a **broad source tree**
   (rust + `end2end/` + `elisp/` + `tools/`
   - all `*.md`, minus `xtask/`/`node_modules`/`target`). A broad src is fine
     here because this derivation is **cheap** (fmt/leptosfmt/prettier/emacs/tsc
     — seconds, no compile), so an occasional whole-tree cache-bust costs
     little. Delete only the five **non-compiling** siblings — `rustfmt`,
     `leptosfmt-check`, `prettier-check`, `ert-check`, `elisp-fmt-check` — and
     add `static-checks`. **Keep `clippy` and `deny` crane derivations
     untouched.** `nix flake check` again runs the non-compiling project static
     checks — via the same `devtool check` the host runs.
4. **`tsc` node deps reuse the existing vendored `e2ePackage`.** `flake.nix`
   already builds `e2ePackage = buildNpmPackage ./end2end` and exports
   `E2E_TYPES_NODE_MODULES` (`${e2ePackage}/node_modules`), provisioned into
   `end2end/node_modules` for the devShell `tsc`. `devtool check tsc` provisions
   `end2end/node_modules` from `E2E_TYPES_NODE_MODULES` when set (mirroring
   `end2end/provision-node-modules.sh`), then runs `tsc`. The `static-checks`
   derivation sets that env from `e2ePackage`. No new node vendoring.
5. **Accept the `devtoolBin` ↔ `coverage` cache coupling.** Moving check
   _definitions_ into `devtool` means a definition change rebuilds `devtoolBin`
   → rebuilds `coverage`. Measured negligible: the StepSpec definitions changed
   7× ever (all gate-construction), never alongside coverage-src; the cost is a
   rare **local** coverage rebuild (~2–4 min, then re-cached). **CI is
   unaffected** (coverage is cachix-push-excluded → rebuilt every run). No
   decoupling (no split binary / manifest) — not worth the complexity.
6. **New ADR + amend ADR-0031.** A numberless draft records the decision and its
   **cleave rationale**: the **non-compiling** static checks are unified in
   `devtool` (one implementation, invoked by the host ladder _and_
   `nix flake check`), because they were hand-duplicated `runCommand` siblings
   prone to divergence (#185); the **compiling** checks (`clippy`/`deny`) stay
   on crane derivations (offline vendored deps), and `tools-clippy` / `xtask`'s
   self-lint stay host-only (no tools vendoring / flake-source exclusion).
   Promoted at ship. **ADR-0031** stays `accepted`, gains a path-form Note: its
   `ert-check` / `elisp-fmt-check` siblings now route through `devtool check`;
   the host StepSpecs stand; the stale "…by `nix flake check`" Consequences line
   is corrected (elisp static checks run via `devtool`, host + nix).
7. **Docs:** `CONTRIBUTING.md` (the "Nix VM checks" list → one `static-checks`
   entry; the verify-ladder/devtool description), `elisp/README.md`, and the
   `elisp/scripts/run-tests.el` comment.

## Acceptance criteria (observable)

1. **AC1 — `devtool check` exists and runs each check.** `devtool check <name>`
   (each of the 7 migrated names) and `devtool check --all` run the tool with
   the spec'd args; `--fix` makes the 5 formatters auto-fix. Unit-tested where
   pure (the spec table / arg construction); the invocations themselves are
   integration-verified by the gate.
2. **AC2 — host gate unchanged in behavior.** `cargo xtask check` (fix) and
   `validate --no-e2e` (verify) still run all the same checks with the same
   per-step `[ok]`/`[FAIL]` output and pass/fail semantics — now via
   `devtool check` for the migrated ones; `xtask-fmt`/`xtask-clippy` still
   native. Green on a clean tree.
3. **AC3 — nix `static-checks` derivation.**
   `nix eval .#checks.x86_64-linux --apply builtins.attrNames` lists
   `static-checks` and **none** of the five deleted siblings (`rustfmt`,
   `leptosfmt-check`, `prettier-check`, `ert-check`, `elisp-fmt-check`);
   **`clippy` and `deny` remain** (kept crane), as do
   `coverage`/`e2e-*`/`e2e`/`e2e-elisp-integration`.
   `nix build .#checks.x86_64-linux.static-checks` passes on a clean tree (runs
   `devtool check --all`, all tools provisioned incl. `tsc` via `e2ePackage`).
4. **AC4 — the migrated checks are single-source.** Each of the **7 migrated**
   checks' tool
   - args appear exactly once (in `devtool`); a grep confirms no residual
     duplicated tool-arg strings for them in `flake.nix` (the 5 siblings are
     gone) or `static_checks.rs` (their StepSpecs now invoke `devtool check`).
     `clippy`/`deny` are out of scope and keep their crane + host-StepSpec
     forms.
5. **AC5 — `nix flake check --no-build` evaluates cleanly**; no dangling
   reference to a deleted binding/derivation. `end2endSrc` (used **only** by the
   deleted `prettier-check`) is **removed** — `static-checks` builds its own
   broad `staticCheckSrc`, not `end2endSrc`. Retained bindings
   (`commonArgs`/`cargoArtifacts` for the kept `clippy`/`deny`/`coverage`,
   `emacsForCi`, `emacsSrc` (still used by `e2e-elisp-integration`),
   `e2ePackage`) are intact.
6. **AC6 — new ADR drafted + ADR-0031 amended.** Draft in `docs/adr/drafts/`
   (numberless, `# ADR-DRAFT:` heading) states the devtool-unification decision;
   promotes cleanly at ship (`adr promote` numbers it + syncs the README;
   `adr-format`/`adr-readme-parity` green). ADR-0031 keeps `accepted` + the
   path-form Note.
7. **AC7 — docs updated.** `CONTRIBUTING.md`, `elisp/README.md`,
   `elisp/scripts/run-tests.el` reflect the `devtool`-unified reality (no
   hermetic-`*-check`-sibling claims for the 5 migrated ones); the "Nix VM
   checks" list shows the `static-checks` entry (and still `clippy`/`deny`).
   `prettier`/`adr-*` gates green.

## Out of scope

- **The compiling checks** — `clippy`/`deny` stay on their crane derivations
  (offline vendored deps); `tools-clippy` stays a host-only StepSpec (no
  tools-workspace vendoring infra, not worth building). A future issue could
  unify these if the infra is ever built.
- Migrating `xtask-fmt`/`xtask-clippy` (structurally impossible — flake-source
  exclusion).
- Removing/altering `coverage`, `e2e-*`, `elisp-integration`.
- Splitting `devtool` to decouple the coverage cache (decided against; coupling
  is minor).
- CI (`ci.yml`) changes — CI runs `cargo xtask validate` (host ladder, now via
  `devtool` for the 7) + the e2e matrix; no `flake check` static siblings were
  ever in CI.

## Testing / verification ladder

- Pure unit tests for the `devtool check` spec table / arg construction
  (host-side).
- `cargo xtask check` / `validate --no-e2e` green (AC2) — exercises
  `devtool check` on the host.
- `nix build .#checks.x86_64-linux.static-checks` green (AC3) — exercises it in
  the sandbox, incl. `tsc` via `e2ePackage`. `nix eval …attrNames` for the
  attr-set delta (AC3/AC5).
- Grep for residual duplicated tool-arg strings (AC4) and dangling refs
  (AC5/AC7).
- ADR gates green; draft promotes at ship (AC6).
