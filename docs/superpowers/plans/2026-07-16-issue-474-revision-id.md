# Plan — #474: `RevisionId` newtype

- Spec:
  [2026-07-16-issue-474-revision-id.md](../specs/2026-07-16-issue-474-revision-id.md)
- Issue: [#474](https://github.com/jaunder-org/jaunder/issues/474)

The smallest track — done as **one commit** (define + the single field change
are trivially green together; the field is the type's only use, so `RevisionId`
is not dead).

## Task 1 — Define + thread (one commit)

- [ ] Append `pub struct RevisionId(i64)` (doc-commented, derives) to
      `common/src/ids.rs` + a unit test (mirror the sibling id types — covers
      the generated surface).
- [ ] `storage/src/posts.rs:111` `PostRevisionRecord.revision_id: i64` →
      `RevisionId`. (No construction/decode/test/wire site exists — see spec.)
- **Verify:** `cargo check --all-features --all-targets`; grep `revision_id`
  (only SQL `post_revisions` strings remain); `cargo xtask check` green
  (coverage, both-backend).
- **Commit:**
  `refactor: add RevisionId newtype; type PostRevisionRecord.revision_id (#474)`.

## Task 2 — Ship

- [ ] Cold-blind pre-merge review (the tiny diff); archive; rebase; PR (Closes
      #474); green CI; **HALT for merge approval**.

## AC coverage

| AC      | Where                        |
| ------- | ---------------------------- |
| AC1     | Task 1 (type + unit test)    |
| AC2     | Task 1 (field + grep)        |
| AC3/AC4 | Task 1 (gate, both backends) |
| AC5     | Task 2                       |
