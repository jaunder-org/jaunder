# Plan — #475: `AudienceId` newtype

- Spec:
  [2026-07-16-issue-475-audience-id.md](../specs/2026-07-16-issue-475-audience-id.md)
- Issue: [#475](https://github.com/jaunder-org/jaunder/issues/475)

## Commit strategy (two commits, per #471 precedent)

- **Commit 1 — define** `common::ids::AudienceId` + a unit test
  (From/Into/Display/FromStr/ serde-bare-int). Unused → workspace stays green.
- **Commit 2 — thread** it through common/visibility + storage + web + tests.
  The `AudienceTarget::Named(AudienceId)` change and the record/trait flips
  ripple across crates, so it lands atomically (an intermediate per-crate commit
  wouldn't compile; the gate rejects a non-compiling commit). Mechanical; lean
  on `cargo check --all-features --all-targets`.

## Task 1 — Define `AudienceId`

- [ ] Append `pub struct AudienceId(i64)` (doc-commented,
      `#[derive(Copy, Clone, Debug,     PartialEq, Eq, Hash, IdNewtype)]`) to
      `common/src/ids.rs`.
- [ ] Unit test exercising the generated surface (mirror the `UserId` test in
      `ids.rs`).
- **Verify:** `cargo test -p common ids::`; commit
  `refactor(common): add AudienceId id newtype (#475)`.

## Task 2 — Thread `AudienceId`

- [ ] **common/visibility.rs** — `AudienceTarget::Named(i64)` →
      `Named(AudienceId)`.
- [ ] **storage/audiences.rs** — `AudienceRecord.audience_id`; object + dispatch
      trait (`create_audience -> AudienceId`;
      `update/delete/add_member/remove_member/list_members`
      `audience_id: AudienceId` params); impls (`.bind(i64::from(..))`; wrap the
      `list_audiences` decode tuple pos 0 and the `RETURNING audience_id`
      scalars on create/update).
- [ ] **storage/posts.rs** — `audience_target_row`
      (`Named(id) => (.., Some(i64::from(id)))`); `audience_target_from_row`
      (`audience_id.map(|id| AudienceTarget::Named(AudienceId::from(id)))`);
      in-file test `Named(n)` literals (`:2222`, etc.).
- [ ] **storage/site_config.rs** — test literal `Named(7)` →
      `Named(AudienceId::from(7))` (match arms use `Named(_)` — no change).
- [ ] **web/posts/mod.rs** — `AudienceSelection.named: Vec<AudienceId>`; the two
      conversion fns (`.map(AudienceTarget::Named)` still works;
      `Named(id) => named.push(*id)`); tests
      (`selection(base, named: &[AudienceId])`, `Named(5/9/3)` literals).
- [ ] **web/audiences/mod.rs** — DTO `audience_id: AudienceId` +
      `#[store(key: AudienceId = ...)]` (first newtype store-key — confirm it
      compiles); `#[server]` fn params **and**
      **`create_audience(...) -> WebResult<AudienceId>`** (return `Ok(id)`
      directly; consumers read only the error, so no ripple);
      `rename/delete/add/remove/list_audience_members` `audience_id: AudienceId`
      params; `AudienceHeader`/`MemberChecklist` component
      `audience_id: AudienceId` (hidden-input `value=` relies on `Display`).
- [ ] **web/pages/{ui,posts}.rs** — `AudienceSelection` picker sites compile
      (the `.named` field type changed; the picker pushes/removes `AudienceId`).
- **Verify:**
  1. `cargo check --all-features --all-targets` green.
  2. AC2 edit-map struck; supplementary grep (`audience_id: i64`, `Named(i64)`,
     `Vec<i64>` audience fields) over touched files.
  3. `cargo xtask check` green (clippy, coverage, both-backend tests).
- **Commit:** `refactor: thread AudienceId through storage/web (#475)`.

## Task 3 — Final gate + ship

- [ ] `cargo xtask validate --no-e2e` clean; cold-blind pre-merge review
      (Standards+Spec); rebase on origin/main; PR (Closes #475); merge on green
      CI.

## AC coverage

| AC  | Task                            |
| --- | ------------------------------- |
| AC1 | Task 1                          |
| AC2 | Task 2 verify (edit-map)        |
| AC3 | Task 1 serde test + Task 3 e2e  |
| AC4 | Task 2 (both backends) + Task 3 |
| AC5 | Task 3                          |
