# ADR-0043: quick-xml advisory: fork + git-patch bridge (RUSTSEC-2026-0194/0195)

- Status: accepted
- Date: 2026-07-02
- Issue: [#193](https://github.com/jaunder-org/jaunder/issues/193)

## Context

RUSTSEC-2026-0194 (O(N²) duplicate-attribute check) and RUSTSEC-2026-0195
(unbounded namespace-declaration allocation) fail `cargo-deny`'s `advisories`
check repo-wide. Both are DoS advisories against **quick-xml 0.39.4**, fixed in
**quick-xml ≥ 0.41.0**. AtomPub POST/PUT bodies are client-supplied
(authenticated app-password clients), so the exposure is real if authenticated.

quick-xml 0.39.4 enters the graph three ways: transitively via
`atom_syndication 0.12.8` and `rss 2.0.13`, and as a direct dep of `common`
(`quick-xml = "0.39"`).

The obvious fixes are all blocked:

- **Bump the crates.** `atom_syndication 0.12.8` / `rss 2.0.13` are the _latest_
  releases and upstream `master` on both still pins `quick-xml = "0.39"`. No
  release exists that depends on quick-xml ≥ 0.41.
- **Lock-file bump.** `cargo update -p quick-xml` locks 0 packages; the registry
  has no 0.39.x backport (0.39.4 → 0.41.0 directly), so `^0.39` can never reach
  a fixed version.
- **`[patch.crates-io]` quick-xml directly.** A patch must satisfy the existing
  requirement; 0.41 does not satisfy `^0.39`.

## Decision

Move the whole tree to quick-xml **0.41** by forking the two syndication crates,
raising _their_ quick-xml requirement, and wiring the forks in with a git
`[patch.crates-io]` — **not** by ignoring the advisories.

1. **Fork** `rust-syndication/atom` → `jaunder-org/atom` and
   `rust-syndication/rss` → `jaunder-org/rss`. On a patch branch, change
   `quick-xml = "0.39"` → `"0.41"` (keeping crate versions `0.12.8` / `2.0.13`
   so `[patch]` applies). The touched quick-xml API subset is stable across
   0.39→0.41 (see spec); expected delta is the version requirement plus at most
   swapping one deprecated `decode_and_unescape_value` call.
2. **Patch** the workspace: root `Cargo.toml` `[patch.crates-io]` points
   `atom_syndication` and `rss` at the forks at a **pinned rev**;
   `common/Cargo.toml` raises its direct `quick-xml = "0.39"` → `"0.41"`.
   Result: a single quick-xml `0.41.x` in `Cargo.lock`, no 0.39.x remaining,
   advisories cleared with **no ignore**.
3. **Hermetic Nix build.** The flake builds with crane and runs `cargo-deny` as
   a crane derivation; a sandboxed build cannot fetch a git `[patch]`. Each fork
   is therefore added as a `flake = false` **flake input** (pinned in
   `flake.lock`) and fed to crane's vendor step via `overrideVendorGitCheckout`,
   so the git source is content-addressed and reproducible (cachix-friendly),
   with no build-time network.
4. **Dependency policy.** `deny.toml` gains `jaunder-org` under
   `[sources.allow-org].github` to authorize the git sources. No
   `[advisories].ignore` entry is added.
5. **Upstream.** Open PRs against `rust-syndication/{atom,rss}` so the forks are
   a temporary bridge, not a permanent maintenance burden.

## Consequences

- **Positive.** The advisories are cleared _correctly_ — the vulnerable code is
  gone from the tree, not merely silenced. No accepted-risk window. The upstream
  PRs, if merged, benefit the wider ecosystem and let us delete the whole
  apparatus.
- **Negative / cost.** We carry two forks and a git `[patch]` until upstream
  releases; the flake now has two extra inputs and a crane vendor override — the
  first git `[patch]` in this repo, so it sets the pattern. `deny.toml`'s
  sources policy is loosened for one org.
- **Reproducibility.** Because the forks are pinned flake inputs (rev + narHash
  in `flake.lock`), the hermetic build stays deterministic and Cachix-cacheable;
  the git patch does not reintroduce network into the sandbox.

## Exit / how to drop this

When `atom_syndication` and `rss` publish releases depending on quick-xml ≥
0.41: delete the `[patch.crates-io]` entries, the two flake inputs, and the
crane `overrideVendorGitCheckout`; raise the `atom_syndication`/`rss` version
requirements in `common/Cargo.toml` to the fixed releases; remove `jaunder-org`
from `[sources.allow-org]`; archive the forks. Tracked as a follow-up to #193.

## Staging: a temporary ignore bridges the repo-wide breakage

The advisory fails `cargo-deny` on `main` and every branch, so a scoped
`[advisories].ignore` of the two IDs is landed **first**, as a standalone hotfix
(PR #194), purely to make the gate green everywhere while this fix is built.
That ignore is a short-lived scaffold, not the resolution: the **final step of
this ADR's work removes it**, and the end state carries no advisory ignore. The
two phases are deliberately separate PRs so the unblock can merge in minutes
without waiting on the fork/patch/Nix work.

## Alternatives considered

- **Scoped `deny.toml` ignore as the _permanent_ answer** (referencing #193)
  until upstream releases. Precedented (RUSTSEC-2024-0436, RUSTSEC-2026-0173)
  and the issue's stated fallback; simplest, no Nix work. Rejected as the _end
  state_ because it accepts the (authenticated-only) DoS risk rather than
  eliminating it, and the fork patch is trivial on the code side — so we use it
  only as the temporary bridge above, not the resolution.
- **Vendored-path `[patch]`** (patched crates checked into `vendor/`). Hermetic
  without git-source handling, but carries two crate sources in-tree for a
  throwaway bridge.
- **Replace `atom_syndication`/`rss`** with an alternative syndication library
  on quick-xml ≥ 0.41. Largest change, real functional-regression risk in
  AtomPub serialize/parse — disproportionate for a dependency-advisory task.
