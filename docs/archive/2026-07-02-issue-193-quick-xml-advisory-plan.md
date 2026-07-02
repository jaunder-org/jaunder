# Plan — issue #193: quick-xml ≥ 0.41 via atom_syndication/rss forks

**Spec:**
[2026-07-02-issue-193-quick-xml-advisory.md](../specs/2026-07-02-issue-193-quick-xml-advisory.md)
**ADR:** [0043](../../adr/0043-quick-xml-fork-patch.md) **Branch:**
`worktree-issue-193-quick-xml-advisory`

Each task is one clean, gate-verified commit (`cargo xtask check` green) unless
marked **(external)** — GitHub/fork operations that produce no jaunder-repo
commit.

## Prerequisite (before any commit here)

- [x] **P0. Rebase onto main once hotfix PR #194 has merged.** #194 lands the
      temporary `deny.toml` ignore of RUSTSEC-2026-0194/0195 on `main`. Rebase
      this branch onto the updated `main` so the ignore is inherited and the
      verify gate is green during development. (Until then, every commit here
      would fail `cargo-deny`.) The design docs (spec, ADR-0043, README row) are
      already staged and become Task 1's commit.

## Tasks

- [x] **1. Land the design docs.** Commit the staged spec,
      `docs/adr/0043-quick-xml-fork-patch.md`, and the `docs/README.md`
      ADR-table row. Pure docs; gate green.

- [x] **2. (external) Create + patch the forks.**
  - `gh repo fork rust-syndication/atom --org jaunder-org` and
    `… rust-syndication/rss --org jaunder-org`.
  - On each fork, branch `quick-xml-0.41`; set `quick-xml = "0.41"` (keep
    `features = ["encoding"]`); keep crate versions `0.12.8` / `2.0.13` so
    `[patch]` applies. `rss`'s `atom_syndication` dep already tracks `0.12.8` —
    leave it (the `[patch]` swaps the source, versions still satisfy).
  - Build + run each fork's own test suite (`cargo test`) against quick-xml
    0.41; fix any hard breakage. Expected delta: the version bump only;
    optionally swap the single deprecated `decode_and_unescape_value` call →
    `decoded_and_normalized_value` to keep the upstream PR warning-clean. **Do
    not** open upstream PRs yet (Task 6).
  - Push the branches; **record the exact commit rev of each** (needed for
    pinning).

- [x] **3. Wire the forks into the workspace + hermetic Nix vendoring (one
      atomic commit).** This must be a single commit: the Cargo-level `[patch]`
      alone would break the hermetic Nix build until the vendoring lands, so
      they land together to keep the gate green.
  - Root `Cargo.toml`: `[patch.crates-io]` `atom_syndication` and `rss` → the
    git forks at the pinned revs from Task 2.
  - `common/Cargo.toml`: `quick-xml = "0.39"` → `"0.41"`.
  - Regenerate `Cargo.lock`; confirm a **single** quick-xml `0.41.x` and **no**
    0.39.x (`cargo tree -i quick-xml`).
  - `flake.nix`: add each fork as a `flake = false` input; feed both to crane's
    vendor step via `overrideVendorGitCheckout` so the `git+https://…?rev=…`
    sources resolve from the pinned flake inputs (no build-time network).
    `flake.lock`: pin the inputs.
  - `deny.toml`: add `jaunder-org` under `[sources.allow-org].github`.
  - **Verify (risk-retirement — this is the spike):** `cargo xtask check` green
    **and** the hermetic Nix path resolves the git patch — e.g.
    `nix build .#checks.<sys>.deny` and the app derivation build succeed with no
    network in the sandbox. If crane vendoring of the git source proves
    intractable, STOP and reassess (fallback options in ADR-0043: vendored-path
    `[patch]`, or scoped-ignore-as-end-state) before proceeding.

- [x] **4. Verify AtomPub round-trip (no functional regression).** Run
      `common`'s atompub tests and the elisp live-integration suite; confirm
      serialize/parse behaviour is unchanged on quick-xml 0.41. If green with no
      code change, this may fold into Task 3's verification rather than a
      separate commit; otherwise commit any fixups.

- [x] **5. Remove the temporary advisory ignore (the climax).** Delete the
      RUSTSEC-2026-0194/0195 entries (and their scaffold comment) from
      `deny.toml` `[advisories].ignore` — the tree is now on quick-xml 0.41, so
      the advisories genuinely no longer apply. Verify
      `cargo deny check advisories` passes **with no ignore**, then full
      `cargo xtask check` / `validate --no-e2e` green. Commit. End state: no
      advisory ignore, single quick-xml 0.41.x.

- [x] **6. (external, user-gated) File follow-up + open upstream PRs.**
  - File a fresh GitHub issue (`jaunder-issues`): _"Drop quick-xml git
    `[patch]` + forks once atom_syndication/rss publish releases on quick-xml ≥
    0.41"_ — the drop-fork tracker (per resolved decision #3). It records the
    exit steps from ADR-0043.
  - **After** the user reviews the local green result, open PRs to
    `rust-syndication/{atom,rss}` from the fork branches (outward-facing —
    confirm first).

## Ship (jaunder-ship, after plan execution + user go)

Final review, archive spec/plan, push branch, open the #193 PR (closes #193),
merge (halt). The follow-up issue from Task 6 stays open as the drop-fork
tracker.

## Notes / risks

- **The one real unknown is Task 3's Nix git-vendoring** (first git `[patch]` in
  this repo). Everything else is mechanical. Task 3 is deliberately the spike.
- Coverage cache: `.ts` edits aren't involved, but Cargo.lock/flake changes will
  rebuild Nix derivations (cachix-warm). Run `cargo clean` on cadence if the
  sweep is long.
- After #194 merges, other branches also rebase; no coordination needed beyond
  P0.
