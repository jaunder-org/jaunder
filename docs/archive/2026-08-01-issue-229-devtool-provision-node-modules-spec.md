# Spec — #229: `provision-node-modules.sh` → `devtool provision-node-modules`

## Context

`end2end/provision-node-modules.sh` symlinks the tsc type-dep closure plus the
nix-matched Playwright into `end2end/node_modules` (gitignored, so absent in
every fresh checkout and every worktree). It reads two devShell env vars —
`E2E_TYPES_NODE_MODULES` (`${e2ePackage}/node_modules`) and
`E2E_PLAYWRIGHT_TEST`
(`${pkgs.playwright-test}/lib/node_modules/@playwright/test`) — and writes
relative to `$PWD`.

Three contexts run it today, all with the repo/worktree root as cwd:

- `flake.nix:1293` — the `shellHook`, which `shellEnv` shares between **both**
  `devShells.default` (interactive/IDE support) **and** `devShells.ci` (every CI
  job runs `nix develop .#ci -c …`).
- `tools/devtool/src/check.rs:120` — `devtool check tsc` shells out to
  `bash end2end/provision-node-modules.sh` before running `tsc`. This is what
  makes `cargo xtask check|validate` self-heal `end2end/node_modules` in a
  worktree, where the shellHook never fired for that cwd. (The old standalone
  `tsc-deps` xtask step was folded into `devtool check tsc` by #188.) On the
  host, xtask reaches this via
  `cargo run --manifest-path tools/Cargo.toml -p devtool`, so a local edit is
  live.
- `flake.nix:1072-1109` — the `static-checks` Nix check derivation copies a
  writable source tree and runs `devtool check --all` from the **prebuilt
  `devtoolBin`**. This is the CI gate for `tsc`, and therefore the third place
  the ported code executes.

ADR-0051 already recorded this conversion as a surfaced follow-up. It is a
**behavior-preserving refactor** of the mechanism — what lands in
`end2end/node_modules` is unchanged — with one deliberate exception (A11).

## Decisions (resolved in design interview)

1. **`devtool`, not `xtask`.** An `xtask` subcommand would compile Rust on every
   `nix develop` — the regression #229 names. `devtool` is a prebuilt binary.
   This also satisfies **ADR-0028**'s litmus rather than bending it: the code
   must execute inside a Nix sandbox (the `static-checks` derivation above),
   which is precisely what `devtool` is for; the shellHook is the secondary host
   caller.
2. **`devtoolBin` moves from `devOnly` into `ciInputs`.** The hook lives in the
   shared `shellEnv`, so once it invokes `devtool` the CI shell needs it on PATH
   or every CI shell entry emits `devtool: command not found` and silently stops
   provisioning. `devtoolBin` is already a `nativeBuildInput` of the
   `static-checks` and coverage derivations, so CI pulls that exact store path
   anyway — the added cost to `devShells.ci` is one cached closure, not a new
   build. ADR-0028:109 states `devtoolBin` "is therefore exposed in the
   **default devShell**"; that one sentence gets a factual amendment to say both
   shells. No new ADR.
3. **Name: `devtool provision-node-modules`** — top-level, hyphenated, alongside
   the existing `seed-e2e` / `csr-bundle`.
4. **Inputs: optional flags with a hand-rolled env fallback, not clap's `env`
   feature.** `--types-node-modules <PATH>` / `--playwright-test <PATH>` are
   `Option<PathBuf>`; a shared resolver falls back to `E2E_TYPES_NODE_MODULES` /
   `E2E_PLAYWRIGHT_TEST` and, when neither is present, errors with a message
   naming the **variable** and the devShell — reproducing today's
   `: "${E2E_TYPES_NODE_MODULES:?unset — run inside the Nix devShell}"`. clap's
   `env` feature was rejected for two reasons: a `required` arg's clap error
   names the flag, never the variable or the devShell (failing A9/A10), and
   enabling the feature would churn `tools/Cargo.toml` + `tools/Cargo.lock` and
   thus `devtoolBin`'s vendored-deps derivation. The resolver is a named seam —
   `provision::StorePaths::resolve(...)`, which owns the flag→variable pairing
   and returns both paths as one value — shared verbatim by the subcommand and
   by `check::run`, so there is exactly one message site and no caller can pair
   a flag with the wrong variable.
5. **Target: `--root <DIR>`, defaulting to the current directory.** It writes
   `<root>/end2end/node_modules`. Both callers pass nothing (repo root is cwd),
   so the contract is unchanged; tests point it at a tempdir without a
   process-global `chdir`.
6. **`devtool check tsc` calls it in-process**, not by spawning itself — no
   subprocess, errors propagate as `anyhow` context. Whether a check provisions
   is a pure, testable predicate: `fn needs_provisioning(name: &str) -> bool`
   (true only for `tsc`), the seam that makes A13 checkable without executing
   `tsc`.
7. **Dot-entries stay skipped.** The bash
   `for dep in "$E2E_TYPES_NODE_MODULES"/*` glob does not match `.bin/` or
   `.package-lock.json`; the Rust port skips them too, so what tsc sees is
   byte-identical. Linking them may be desirable, but that is a separate,
   deliberate change — not a silent side effect of this refactor.
8. **Unset env is a hard error.** In the shellHook that prints an error and
   shell entry continues (the hook does not `set -e`); in `devtool check tsc` it
   fails the check.
9. **No ADR.** ADR-0051 already recorded the direction; this cycle implements
   the follow-up it named. ADR-0051's text stays as written (a historical
   record); only ADR-0028's one factual sentence about where `devtoolBin` is
   exposed is amended (decision 2).

## Consequences to expect

- Deleting the script changes `e2ePackage` (`flake.nix:488-497` copies all of
  `./end2end` into the store), so its store path changes, every e2e derivation
  rebuilds, and every existing `end2end/node_modules` is stale on this commit —
  A8's repoint path gets exercised for real. `npmDepsHash` is unaffected.
- `tools/` is excluded from the coverage derivation's source (`flake.nix:1128`)
  and `devtoolBin` builds with `doCheck = false`, so the new code is gated by
  `tools-test` / `tools-clippy` only. No coverage evidence or `cov:ignore`
  obligation attaches to it.

## Acceptance criteria (observable)

**Command surface**

- A1. `devtool provision-node-modules --help` succeeds and lists
  `--types-node-modules`, `--playwright-test`, and `--root`; the first two
  document their env-var fallback in their help text.
- A2. With no flags and both env vars set, `devtool provision-node-modules`
  provisions `<cwd>/end2end/node_modules` — i.e. the flags default from
  `E2E_TYPES_NODE_MODULES` / `E2E_PLAYWRIGHT_TEST`.
- A3. `--root <DIR>` provisions `<DIR>/end2end/node_modules` and leaves the
  process cwd untouched.
- A4. An explicit flag wins over the corresponding env var when both are set.

**Provisioning result** (given a types dir containing `@types`, `typescript`,
`undici-types`, `playwright`, `playwright-core`, `@playwright`, `.bin`, and
`.package-lock.json`, plus a separate playwright-test dir)

- A5. Every non-dot entry of the types dir **except `@playwright`** exists at
  `<root>/end2end/node_modules/<name>` as a symlink resolving to that entry.
- A6. No dot-entry (`.bin`, `.package-lock.json`) is created in
  `<root>/end2end/node_modules`.
- A7. `<root>/end2end/node_modules/@playwright` is a **real directory**
  containing a single symlink `test` → the `--playwright-test` path — i.e. the
  nix-matched Playwright, not the types dir's own `@playwright` copy, even
  though the types dir also contains an `@playwright` entry.
- A8. Running the command twice in a row succeeds both times and leaves an
  identical tree (idempotent), including when the first run left `@playwright`
  as a symlink and the second must replace it with a directory.
- A9. Re-running after the types dir's store path changes repoints every symlink
  to the new path.

**Failure modes**

- A10. With `E2E_TYPES_NODE_MODULES` unset and no `--types-node-modules`, the
  command exits non-zero with a message naming that **variable** and telling the
  reader to run inside the Nix devShell.
- A11. Same for `E2E_PLAYWRIGHT_TEST` / `--playwright-test`.
- A12. When the types path is set but does not exist, the command exits non-zero
  with an error naming that path. (The bash version silently created a symlink
  literally named `*`; the port does not. This is the one deliberate behavior
  change.)

**Callers**

- A13. `tools/devtool/src/check.rs` no longer spawns `bash`; `devtool check tsc`
  invokes the provisioning function in-process before running `tsc`.
- A14. `needs_provisioning("tsc")` is true and `needs_provisioning(n)` is false
  for every other name in `check::ALL`.
- A15. `flake.nix`'s `shellHook` invokes `devtool provision-node-modules`
  instead of `bash end2end/provision-node-modules.sh`.
- A16. `devtoolBin` appears in `ciInputs`, so it is on PATH in **both**
  `devShells.ci` and `devShells.default`; and the `shellHook` text contains no
  `cargo` invocation (no Rust compile is added to shell entry).
- A17. `end2end/provision-node-modules.sh` is deleted; no file outside
  `docs/adr/` and `docs/archive/` references it by filename, **and** no
  surviving comment describes provisioning as a shell script — specifically
  `flake.nix:1097-1098` ("the provision script guards on each with `${VAR:?}`"),
  `flake.nix:1280-1293` (the shellHook comment), and
  `tools/devtool/src/check.rs:110-111` ("via the shared script") are each
  reworded to name the subcommand.
- A18. ADR-0028's sentence at line 109 names both devShells rather than only the
  default one.

**Gates**

- A19. From a worktree where `end2end/node_modules` has been removed,
  `cargo xtask check` is green and afterwards `end2end/node_modules` exists and
  is populated — the self-heal property, unchanged. (`validate` reaches
  provisioning by the identical `devtool check tsc` path; `check` is the
  sufficient witness.)
- A20. `nix build .#checks.<system>.static-checks` is green — the
  prebuilt-`devtoolBin` path CI actually uses, which `cargo xtask check` does
  not exercise (it runs devtool via `cargo run`).

## Out of scope

- Linking dot-entries (`.bin`) — see decision 7.
- Pruning entries in `end2end/node_modules` that a previous store path left
  behind; the script never did, and neither does the port.
- Migrating any other shell script to `devtool`.
- Changing `e2ePackage`, `playwright-test`, or which deps the closure contains.
- Rewording ADR-0051.

## Testing / verification ladder

- **Library tests (`tools/devtool/src/provision.rs`, run by the `tools-test`
  step — `cargo test --manifest-path tools/Cargo.toml`):** tempdir tests driving
  `provision::run` against a fake store tree —
  `provisions_visible_entries_as_symlinks` (A5), `skips_dot_entries` (A6),
  `pins_playwright_test_over_e2e_package_copy` (A7),
  `is_idempotent_across_reruns` (A8),
  `replaces_a_stale_playwright_symlink_with_the_dir` (A8),
  `repoints_symlinks_when_the_store_path_changes` (A9),
  `errors_when_types_dir_missing` (A12); plus `resolve_paths` tests for A10/A11
  message content, driven by explicit arguments (not process env).
- **CLI test (`tools/devtool/tests/provision_cli.rs`, same step):** spawns
  `env!("CARGO_BIN_EXE_devtool")` with `--root <tempdir>` to cover the surface
  the in-process caller bypasses — `--help` lists the flags (A1), env-var
  defaulting works with no flags (A2), `--root` does not disturb cwd (A3), an
  explicit flag beats the env var (A4).
- **`check.rs` unit test:** `needs_provisioning` over `check::ALL` (A14).
- **Machine gate:** `rm -rf end2end/node_modules` in this worktree, then
  `devtool run --cwd <worktree> -- cargo xtask check` — proves A19 and exercises
  the in-process caller (A13).
- **Nix gate:** `nix build .#checks.<system>.static-checks` (A20).
- **Manual, once:** `nix develop` in the main checkout and `nix develop .#ci`,
  confirming both shells provision via the new command with no
  `command not found` (A15, A16).
- **Code inspection at ship:** A13's "no `bash` spawn", A17's comment rewording,
  A18.
