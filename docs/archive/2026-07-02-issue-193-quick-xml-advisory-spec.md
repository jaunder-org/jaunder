# Spec — issue #193: clear RUSTSEC-2026-0194/0195 by moving to quick-xml ≥ 0.41

**Status:** design resolved, awaiting approval **Issue:**
[#193](https://github.com/jaunder-org/jaunder/issues/193) **Date:** 2026-07-02
**Related ADR:**
[0043 — quick-xml advisory: fork + git-patch bridge](../../adr/0043-quick-xml-fork-patch.md)

## Problem

`cargo-deny`'s `advisories` check (run in `cargo xtask check`/`validate` and as
the Nix `deny` crane derivation) fails repo-wide as of 2026-07-02 on two RustSec
advisories against **quick-xml 0.39.4**:

- **RUSTSEC-2026-0194** — O(N²) duplicate-attribute check in
  `BytesStart::attributes()` (CPU-exhaustion DoS on crafted XML).
- **RUSTSEC-2026-0195** — unbounded namespace-declaration allocation in
  `NsReader`/`NamespaceResolver::push` (memory-exhaustion DoS).

Both are fixed in **quick-xml ≥ 0.41.0**. AtomPub POST/PUT bodies are
client-supplied (authenticated app-password clients), so the exposure is real if
authenticated.

## Why the headline "just bump atom_syndication/rss" has no release path

quick-xml 0.39.4 is transitive via two crates, plus one direct workspace dep:

```
quick-xml 0.39.4
├── atom_syndication 0.12.8 ── common ── {jaunder, storage, web, …}
├── rss 2.0.13              ── common
└── common (direct: common/Cargo.toml `quick-xml = "0.39"`)
```

Investigation findings (2026-07-02):

- `atom_syndication` **0.12.8** and `rss` **2.0.13** are the **latest**
  published releases — and upstream `master` on both (`rust-syndication/atom`,
  `.../rss`) is **still** on `quick-xml = "0.39"` (no bump in flight). Both
  hard-require `quick-xml = "0.39"` (`features = ["encoding"]`).
- **No `quick-xml` 0.39.x backport exists** — the registry jumps 0.39.4 → 0.41.0
  (`--precise 0.39.5` → "no matching package"). So `^0.39` can never resolve to
  a fixed version.
- `[patch.crates-io]` cannot force quick-xml 0.41 directly: a patch must still
  satisfy `^0.39`, which 0.41 does not.

Therefore the only route to a **clean** advisory clearance (no ignore) is to
make the whole tree resolve to quick-xml 0.41 — which requires the two
syndication crates to stop pinning `^0.39`. Since no such release exists, we
**fork** them.

## Decision (chosen approach)

Fork both crates, bump their `quick-xml` requirement to `0.41`, wire the forks
into our build via a **git `[patch.crates-io]`**, bump `common`'s direct dep,
and open upstream PRs so the forks are temporary. Full detail and rationale in
**ADR-0043**.

## Landing sequence (two phases)

Because the advisory fails `cargo-deny` **repo-wide** (main and every branch),
the unblock is landed first, separately from the real fix:

1. **Unblock (standalone hotfix — PR
   [#194](https://github.com/jaunder-org/jaunder/pull/194), already open).** An
   8-line, config-only, _temporary_ `[advisories].ignore` of
   RUSTSEC-2026-0194/0195 in `deny.toml`, on its own branch off `main`, so every
   branch's gate is green while the real fix lands. **Not** part of this branch.
2. **Real fix (this branch, #193).** Forks + `[patch]` + Nix vendoring move the
   whole tree to quick-xml ≥ 0.41; the **final task removes the temporary
   ignore** (from `main`, inherited via rebase after #194 merges). The end state
   carries **no advisory ignore**.

### Why the code delta is small (0.39 → 0.41)

The 0.39→0.41 surface is additive-heavy (UTF-16 `DecodingReader`, `XmlVersion`,
new error variants, the two advisory fixes — both non-breaking). The genuinely
breaking 0.40.0 items do **not** touch code paths any of our crates exercise:

| 0.40.0 breaking change                                   | atom / rss / common impact                                                                                       |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| #914 removed deprecated `NsReader::{prefixes,resolve,…}` | none — all use plain `Reader`, never `NsReader`                                                                  |
| #944 `read_text()` → `BytesText`                         | none — appears only in an atom _test_                                                                            |
| #371 deprecated `Attribute::unescape_value` family       | soft — atom & rss each call `decode_and_unescape_value` once; deprecated, still identical behaviour, not removed |
| new error-enum variants                                  | none — all wrap quick-xml errors (`Error::Xml(e)` / `From<quick_xml::Error>`), never match its enum exhaustively |

`common`'s own usage (`escape::{escape,unescape}`, `Writer`, `events::*`,
`From<quick_xml::Error>`) is entirely in the stable subset. Expected fork patch:
bump the version requirement per crate; optionally swap the one deprecated call
to silence warnings.

### The real work: hermetic Nix vendoring of a git patch

The flake builds Rust with **crane** (`craneLib.buildDepsOnly`/`buildPackage`),
and `cargo-deny` itself runs as `craneLib.cargoDeny`. Crane vendors from
`Cargo.lock`; a git `[patch]` introduces a `git+https://…?rev=…` source that a
sandboxed (no-network) build cannot fetch without help. The reproducible crane
pattern is to add each fork as a `flake = false` **flake input** (pinned in
`flake.lock`) and feed it to crane via `overrideVendorGitCheckout`. This is the
load-bearing, non-trivial part and is validated early in the plan (spike) before
the rest is built out.

## Scope

In scope:

- Fork `rust-syndication/atom` → `jaunder-org/atom` and `rust-syndication/rss` →
  `jaunder-org/rss`; branch bumping `quick-xml` to `0.41`; verify each fork
  builds + its own tests pass.
- Root `Cargo.toml` `[patch.crates-io]` → the two git forks (pinned rev).
- `common/Cargo.toml`: `quick-xml = "0.39"` → `"0.41"`.
- `flake.nix`/`flake.lock`: fork flake inputs + crane git-checkout override so
  the hermetic build (app + `deny` + `nextest` + coverage) resolves the patched
  tree.
- `deny.toml`: add `jaunder-org` to `[sources.allow-org].github` (git-source
  policy). **No** `[advisories].ignore` entry.
- Open upstream PRs against `rust-syndication/{atom,rss}` (a discrete,
  user-gated step — see Open questions).
- ADR-0043 + `docs/README.md` ADR-table row.

Out of scope (file as follow-ups):

- Dropping the git `[patch]` + reverting to plain registry version bumps once
  upstream ships releases with quick-xml ≥ 0.41 (tracked follow-up; #193 stays
  open until then, or closes with the follow-up as the tracker).

## Acceptance

- `cargo-deny` advisories pass with **no advisory ignores**; `Cargo.lock`
  resolves quick-xml **≥ 0.41.0** as the single quick-xml version (no duplicate
  0.39.x).
- `cargo xtask check` and `cargo xtask validate --no-e2e` green (host).
- Nix `deny`, app build, and `nextest` checks green (hermetic, with the fork
  inputs).
- No functional regression in AtomPub serialize/parse: `common` atompub tests +
  the elisp live-integration suite pass.

## Resolved decisions (approved 2026-07-02)

1. **Nix vendoring mechanism** — flake-input + `overrideVendorGitCheckout`
   (reproducible/ cachix-friendly), validated by the plan's spike. _(approved)_
2. **Upstream PR timing** — open the `rust-syndication` PRs **after** the local
   tree is green and reviewed (a discrete, user-gated step). _(approved)_
3. **#193 disposition** — **close #193 on green**; a **fresh follow-up issue**
   tracks dropping the fork/`[patch]` once upstream releases with quick-xml ≥
   0.41. _(approved)_
