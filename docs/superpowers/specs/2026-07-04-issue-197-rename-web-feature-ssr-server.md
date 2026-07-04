# Spec — Rename the `web` crate feature `ssr` → `server` (issue #197)

**Issue:** jaunder-org/jaunder#197 (milestone 8, "Off concurrent SSR").
Follow-up from #180 / PR #192.

## Problem

After #180 removed the reactive SSR page render, the `web` crate's `ssr` cargo
feature no longer means "isomorphic SSR." It now compiles the **server-side
data-API build** — the `#[server]` fn impls plus `leptos/ssr` — with **no page
render**. The name `ssr` is therefore misleading. Rename it to `server` for
honesty. Purely mechanical; **no behavior change**.

## Scope

The `web`-owned `ssr` feature is fully self-contained: no `.rs` file outside
`web/src/` gates on it, and `web` is the only crate that defines an `ssr`
feature. `common`, `storage`, `tools`, `csr`, and `server/src` have none. The
feature is activated transitively via the `server` crate's `web` dependency —
nothing in `flake.nix`, `xtask`, or CI names it — so renaming the two Cargo.toml
references plus the in-crate `cfg` sites is sufficient for the build.

**Code footprint (22 files):**

- `web/Cargo.toml` — the feature definition.
- `server/Cargo.toml` — the `web` dependency's feature list.
- 20 files under `web/src/` carrying `#[cfg(feature = "ssr")]` /
  `#[cfg_attr(feature = "ssr", …)]`.

**Docs (decided scope — "live guidance + ADR-0041 note"):**

- `CONTRIBUTING.md` and `docs/web-style-guide.md` §8 — the docs that actively
  instruct contributors to write the old name.
- `docs/adr/0041` — correct the now-false "still named `ssr`" bullet.

## Non-goals

- **Do not** touch `leptos`'s own `ssr` feature — `leptos/ssr`,
  `leptos_meta/ssr`, `leptos_router/ssr`, and the
  `leptos = { features = ["ssr"] }` deps in `server/Cargo.toml` are a different
  crate's feature and stay verbatim.
- **Do not** touch the `csr`, `hydrate`, or `default` features.
- **No behavior change** — this is a rename only.
- **Do not** edit `docs/archive/**`, ADR-0013, or ADR-0017 — historical records
  left as point-in-time narrative (they reference the old name in context).

## Acceptance criteria (observable)

1. **`web/Cargo.toml`** defines a `server` feature whose value list is identical
   to the former `ssr` list (`dep:anyhow`, `common/metrics`, `leptos/ssr`,
   `leptos_meta/ssr`, `leptos_router/ssr`, `dep:leptos_axum`, `dep:axum`,
   `dep:base64`, `dep:chrono`, `dep:storage`, `dep:email_address`,
   `dep:tracing`). No `ssr = [` key remains. `default` and `csr` are unchanged.
2. **`server/Cargo.toml`** — the `web` dependency reads `features = ["server"]`.
   The two `leptos` dependencies still read `features = ["ssr"]` (leptos's own).
3. **`rg 'feature = "ssr"' web/src`** returns **zero** matches; every former
   `cfg` and `cfg_attr` site now reads `feature = "server"`.
4. **`CONTRIBUTING.md`** and **`docs/web-style-guide.md` §8** show
   `#[cfg(feature = "server")]`; neither contains `feature = "ssr"` afterward.
5. **`docs/adr/0041`** no longer states the feature is "still named `ssr`"; the
   bullet reflects that it is now `server`, renamed in #197.
6. **`leptos/ssr` / `leptos_meta/ssr` / `leptos_router/ssr`** appear unchanged
   everywhere (grep confirms the leptos feature is untouched).
7. **ADR-0013, ADR-0017, and `docs/archive/**`\*\* are byte-unchanged.
8. **Gate green:** `cargo xtask check` passes — the workspace, the `server`
   crate, and the `web` unit tests compile and run under the renamed feature,
   proving no build reference was missed and no behavior changed.

## Verification

`cargo xtask check` (static + clippy + coverage/tests) is the acceptance gate;
criterion 8 is the load-bearing check. Criteria 1–7 are confirmable by `rg`
(`rg 'feature = "ssr"' web/src CONTRIBUTING.md docs/web-style-guide.md docs/adr/0041*`
→ no matches) and `git diff wt-base-issue-197..HEAD --stat`.
