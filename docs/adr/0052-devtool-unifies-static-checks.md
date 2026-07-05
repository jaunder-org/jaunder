# ADR-0052: devtool is the single implementation of the non-compiling static checks

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-07-05
- Issue: [#188](https://github.com/jaunder-org/jaunder/issues/188)

## Context and Problem Statement

Several static checks were specified **twice** — a host `StepSpec` in
`xtask/src/steps/static_checks.rs` (the enforced `cargo xtask check`/`validate`
ladder) **and** a hermetic nix `*-check` `runCommand` sibling (`rustfmt`,
`leptosfmt-check`, `prettier-check`, `ert-check`, `elisp-fmt-check`) — kept in
lockstep by hand. #185 is exactly how that diverges: the host `prettier`
StepSpec was extended to Markdown (`**/*.md`) but the nix `prettier-check`
sibling was not, so `nix flake check` and the host gate silently checked
different file sets.

`nix flake check` is a **standard, discoverable** entry point, so simply
_deleting_ the siblings (the issue's first framing) would strip the static
checks out of it. But `xtask` is **host-only**: it is deliberately excluded from
the flake source (the discipline that keeps ~82% of `xtask`-touching commits
from busting the coverage/e2e cache — measured over the repo's history), so nix
cannot invoke the host ladder directly. `tools/devtool`, however, is the
in-sandbox tool nix **already** builds (`devtoolBin`) and the `coverage` check
**already** runs (`devtool coverage emit`); its stated purpose is "sandboxes
where `xtask` (host-only) is unavailable … deliberately extensible."

The static checks split cleanly on **whether they invoke `cargo` to compile**,
which decides whether a nix check needs crane's offline vendored-deps plumbing:

- **Compiling** (`clippy`, `cargo-deny`, `tools-clippy`) — need vendored deps +
  an offline `CARGO_HOME` in the sandbox; today served by `craneLib.cargo*`
  derivations.
- **Non-compiling** (`fmt`, `leptosfmt`, `prettier`, `tsc`, `elisp-fmt`, `ert`,
  `tools-fmt`) — need only their tool binary + the source files; a plain
  `runCommand` suffices.

## Decision

**`devtool` is the single implementation of the 7 non-compiling static checks.**
A `devtool check <name> | --all [--fix]` command owns each check's tool +
arguments (moved out of the host `StepSpec`s), and **both** callers invoke it:

- The **host** verify ladder runs `cargo run -p devtool -- check <name>` (from
  the `tools/` workspace, so a local edit is reflected — consistent with `xtask`
  being rebuilt each run); `--fix` in `check`, verify in `validate`.
- One **nix** `static-checks` `runCommand` runs `devtool check --all` over a
  broad source tree from the prebuilt `devtoolBin`, restoring the non-compiling
  static checks to `nix flake check`.

Each of those 7 checks' tool + args therefore live **exactly once**; the #185
divergence class is structurally impossible.

**The compiling checks stay where cargo has its deps.** `clippy` and `deny` keep
their crane derivations (untouched); `tools-clippy` stays a host-only `StepSpec`
(the `tools/` workspace has no vendoring infra and building it isn't worth it);
`xtask`'s own `fmt`/`clippy` stay host-only (`xtask/` is out of the flake
source). So `nix flake check` covers the non-compiling project static checks but
**not** the checker's self-lint — a deliberate, documented boundary.

**Accept the `devtoolBin` ↔ `coverage` cache coupling.** `coverage` depends on
`devtoolBin`, so moving check _definitions_ into `devtool` means a definition
change rebuilds `coverage` **locally**. Measured negligible: the definitions
changed 7× ever (all during gate construction), never alongside coverage-src;
the cost is a rare local coverage rebuild (then re-cached), and **zero in CI** —
`coverage` is cachix-push-excluded and rebuilt every run regardless. No
decoupling (split binary / manifest) is warranted.

Rejected: **deleting** the siblings outright (loses `nix flake check`'s static
coverage); letting **nix invoke `xtask`** (reverses the flake-source exclusion
that saves the cache); a **single all-ten derivation** (collapses four hard
problems — crane offline plumbing, `tools/` vendoring, offline `cargo-deny`, a
three-tree union src — into one `runCommand` that will not build).

## Consequences

- Good: `nix flake check` runs the non-compiling project static checks again,
  via the **exact code** the host runs; no hand-duplicated static-check
  siblings, so #185-class drift cannot recur.
- Good: `devtool` grows toward its stated role as the shared host/sandbox tool;
  aligns with the Devtool migration milestone (#3).
- Neutral: the static surface is now divided by the compile boundary.
  `clippy`/`deny` keep both a crane derivation and a host `StepSpec` (their args
  are craneLib-mediated and stable, far less divergence-prone than the
  hand-written `runCommand`s that caused #185).
- Bad: a static-check _definition_ edit now rebuilds `coverage` locally (the
  `devtoolBin` coupling). Accepted — rare, cheap, CI-invisible.
- **Amends [ADR-0031](0031-elisp-separately-tested-subproject.md):** its
  hermetic `ert-check` / `elisp-fmt-check` nix siblings are retired — those
  checks now run via `devtool check` (host + nix); the host `ert`/`elisp-fmt`
  StepSpecs stand.
