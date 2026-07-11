# Plan — issue #314: converge the audiences vertical (+ relocate the Topbar twin)

Spec of record: the #314 issue body + ADR-0056
(`docs/adr/0056-web-canonical-colocated-leptos.md`). Scope confirmed with
maintainer: convergence + full Topbar-twin relocation, all in one cycle; broader
cleanup steered from the resulting diff at review.

## Tasks

- [x] **1. Establish `web::ui` and relocate the `Topbar` twin.**
      `web/src/ui/{mod,topbar}.rs` + `pub mod ui;` in `lib.rs`. `topbar.rs`
      holds the pure `render_topbar` and the reactive `Topbar`; `escape_html`
      reached (now `pub(crate)`) from `web::render`. Golden tests `_with_sub_` /
      `_without_sub_`.
- [x] **2. Shim the origins.** `pages/ui.rs` re-exports `crate::ui::Topbar`;
      `render/mod.rs` re-imports `render_topbar` from `web::ui::topbar`.
- [x] **3. Converge the audiences page.** **Option A** — single co-located
      `web::audiences` whose `#[component]` UI sits at module level beside the
      `#[server]` fns, decomposed into
      `CreateAudienceForm`/`AudienceRow`/`AudienceHeader`/`MemberChecklist` with
      a shared `Revalidate` context signal (maintainer steer). Router
      re-pointed; `pages/audiences.rs` deleted.
- [x] **4. Boundary verification.** `cargo xtask check --no-test` green (host
      static + clippy + wasm clippy); no `target_arch` gate, no fake stub.
- [x] **5. Full gate.** `cargo xtask validate` green on the rebased tree
      (Postgres storage tests + all four e2e combos + coverage — 16,916
      executable lines, 0 failures / 0 guard violations / 0 CRAP over
      threshold).

## Emergent (maintainer-directed review + #334 rebase)

- [x] **Q3 — author-scope membership in storage.**
      `remove_member`/`list_members` take `author_user_id` and WHERE-filter it
      (both backends, no migration); web `assert_owns_audience` deleted;
      storage + web tests updated; added `audience_members_are_author_scoped`.
      Cross-author now scopes to empty/no-op (200) rather than `NotFound`.
- [x] **Merge the audiences SQL dialect (ADR-0019).** `AudienceDialect`
      dissolved into shared `$n` SQL in the generic `AudienceStore<DB>` impl;
      bound on `Backend` with `storage.audiences.*` `db.system` spans; backend
      files → type aliases.
- [x] **Cold-review fixes.** Audience-list refetch is a context `Revalidate`
      signal (create/rename/delete); each `MemberChecklist` owns a **local**
      members trigger (add/remove) — so a toggle re-fetches only that audience,
      never remounting the list. Restored sticky-signal retention (no `Loading…`
      flash on mutation). Added `audiences.spec.ts` (element-handle no-remount
      guard + refetch). Second cold review: no blocking/should-fix; remaining
      NITs dispositioned (pre-existing or deliberate).
- [x] **Rebase onto #334 (thin-web error layering).** Resolved `web::audiences`
      conflicts to bare `.await?` (conversion via the `host`/`storage` `From`
      impls) + Q3 scoping; `map_audience_error` is now the storage
      `From<AudienceError>`. `check --no-test` green post-rebase.
- [x] **Review test-gaps folded in + follow-ups filed.** Adversarial test review
      surfaced author-boundary + delete-cascade gaps; folded into the tree
      (cross-author add rejected, cross-author rename/delete scoped,
      per-endpoint unauth, delete-cascades-memberships, empty-list assertion).
      Follow-ups: #346/#349/#350 (view-model + newtype) and #354–#358 (test
      gaps, e2e flash, `post_form` dedup).

## Acceptance (from the issue)

- `AudiencesPage` co-located in `web::audiences`; no `pages/audiences.rs`
  remains.
- `Topbar` twin (`Topbar` + `render_topbar`) in `web::ui::topbar`; shim in
  `pages/ui.rs`.
- Co-located `#[component]` UI (ADR-0056); no `target_arch` gate; no fake host
  stub (ADR-0055).
- `cargo xtask validate` green.
