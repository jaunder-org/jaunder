# RenderedHtml Field Gate - Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing `rendered-html-from-trusted` gate so direct
production struct fields typed `RenderedHtml` are reviewed marker sites, then
mark the legitimate in-tree fields and update the ADR projection.

**Architecture:** Keep one xtask step and one marker token. The rendered HTML
gate will enumerate two populations before classification: existing
`from_trusted` door mentions plus direct `RenderedHtml` field mentions. Marker
classification must happen after those populations are merged, so a field marker
does not appear stale to the call-site scanner and a call-site marker does not
appear stale to the field scanner.

**Tech Stack:** Rust, `syn`, xtask static checks, ADR docs.

**Spec:**
[`2026-08-21-issue-701-renderedhtml-row-field-gate.md`](../specs/2026-08-21-issue-701-renderedhtml-row-field-gate.md)

## Review Header

**Scope - in:** the `rendered-html-from-trusted` xtask step, its tests, local
markers on existing direct `RenderedHtml` production fields, ADR-0079, ADR-0123,
and `docs/ARCHITECTURE.md`.

**Scope - out:** runtime behavior, SQL text, storage decode behavior, serde or
wire shape changes, a new xtask step, a new ADR, recursive container type
coverage, and general SQL column-to-field correspondence.

**Tasks:**

1. Commit the approved spec and this approved plan.
2. Extend the gate scanner and marker classification.
3. Mark legitimate fields and update docs.
4. Run the full check, commit, review, and hand off to ship.

## Global Constraints

- Preserve the existing `rendered-html-from-trusted:allow <reason>` marker
  token.
- Preserve current `from_trusted` call-site behavior.
- Classify call-site mentions and field mentions together before orphan-marker
  detection.
- Keep direct field type coverage explicit: `RenderedHtml`,
  `common::render::RenderedHtml`, imports/aliases resolving to `RenderedHtml`,
  and type aliases whose target is `RenderedHtml`.
- Keep wrappers/containers out of scope unless the implementation deliberately
  adds and tests recursive coverage. The expected plan path is to ignore
  `&RenderedHtml`, `Option<RenderedHtml>`, `Vec<RenderedHtml>`, and
  `Box<RenderedHtml>` and document that boundary.
- Fail closed on unresolved direct single-ident field types.
- Account explicitly for cfg-gated support files that are scanned as standalone
  source files. `storage/src/test_support.rs` is included by
  `#[cfg(any(test, feature = "test-utils"))]` from `storage/src/lib.rs`, but the
  source scanner reads it directly under `storage/src`; the implementation must
  either exempt that file/class deliberately or mark the field as an accepted
  over-included support surface.
- Accept conservative over-inclusion for direct fields inside inline modules
  whose non-`RenderedHtml` type is only defined inside the same inline module,
  unless the implementation adds scope-aware local definition resolution. The
  existing resolver's top-level-file model makes fail-closed behavior cheaper
  and more honest than pretending full type resolution exists.
- No `Co-Authored-By` trailer.
- Run `devtool run -- cargo xtask check` before each commit.

---

### Task 1: Commit Approved Planning Docs

**Files:**

- Add:
  `docs/superpowers/specs/2026-08-21-issue-701-renderedhtml-row-field-gate.md`
- Add:
  `docs/superpowers/plans/2026-08-21-issue-701-renderedhtml-row-field-gate.md`

**Steps:**

- [x] Run `devtool run -- cargo xtask check`.
- [x] Stage the spec and plan with `git add`.
- [x] Commit: `docs: plan rendered html field gate (#701)`.

---

### Task 2: Extend the Gate Scanner

**Files:**

- Modify: `xtask/src/steps/rendered_html_from_trusted_check.rs`
- Modify if needed: `xtask/src/steps/ident_gate.rs`

**Implementation:**

- [ ] Add a field population scanner that parses each source with `syn` and
      walks Rust struct fields under the existing `POLICED_ROOTS`.
- [ ] Reuse the existing owner-alias census for `RenderedHtml`, so imported
      aliases and `type Html = RenderedHtml` style aliases are caught.
- [ ] Treat direct path field types whose final segment is a known
      `RenderedHtml` owner alias as field mentions.
- [ ] Treat direct single-ident field types that cannot be resolved to a local
      definition or import as field mentions, failing closed.
- [ ] Ignore resolvable non-`RenderedHtml` direct types.
- [ ] Ignore wrapper/container/borrowed forms on the expected path:
      `&RenderedHtml`, `Option<RenderedHtml>`, `Vec<RenderedHtml>`, and
      `Box<RenderedHtml>`.
- [ ] Preserve the current test-code exemption policy for fields under
      `#[cfg(test)]`, `#[test]`, `#[rstest]`, and equivalent existing scanner
      handling.
- [ ] Decide and implement the `storage/src/test_support.rs` policy explicitly:
      either teach the field scanner to treat cfg-gated support files as
      test/support-only, or mark `SeededPost.rendered_html` with a reason that
      says it is an intentionally over-included test-support surface.
- [ ] Merge existing `from_trusted` mentions and new field mentions per file
      before calling marker classification and orphan detection.
- [ ] Keep failure prose clear enough to distinguish a `from_trusted` door from
      a `RenderedHtml` field trust surface while preserving the current recovery
      path and marker census.
- [ ] Update the module docs to describe the second population, direct-type
      boundary, fail-closed unresolved-ident behavior, and unreadable classes.

**Tests:**

- [ ] Existing `from_trusted` tests still pass unchanged or with only wording
      adjustments forced by the new combined report.
- [ ] An unmarked production field `rendered_html: RenderedHtml` fails.
- [ ] A field marker on the line immediately above passes when it has a reason.
- [ ] A bare field marker without a reason fails.
- [ ] A stale/orphan field marker fails.
- [ ] A marker for a field is not reported stale when there is no call-site
      mention on the next line.
- [ ] A marker for a `from_trusted` call is not reported stale when there is no
      field mention on the next line.
- [ ] A marked line containing both a `from_trusted` mention and a
      `RenderedHtml` field mention fails as a shared-line marker violation,
      proving the per-line site count is across both populations.
- [ ] Test-only `RenderedHtml` fields are exempt under the same policy as the
      call-site scanner.
- [ ] The field scanner catches: `RenderedHtml`, `common::render::RenderedHtml`,
      an in-file `use ... as ...` alias, and a type alias whose target is
      `RenderedHtml`.
- [ ] The direct-type boundary is pinned with tests showing `&RenderedHtml`,
      `Option<RenderedHtml>`, `Vec<RenderedHtml>`, and `Box<RenderedHtml>` are
      ignored.
- [ ] Unrelated resolvable types do not need markers, including names that only
      contain the `RenderedHtml` substring.
- [ ] An unresolved direct field type requires a marker, and both marked and
      unmarked cases are covered.
- [ ] A direct field inside an inline module whose type is defined only inside
      that inline module is either correctly recognized as resolvable or is
      covered by an explicit conservative-overreach test/documentation path.

**Check:**

- [ ] Run `devtool run -- cargo xtask check --no-test`.

---

### Task 3: Mark Legitimate Fields And Update Docs

**Files:**

- Modify: `storage/src/helpers.rs`
- Modify: `storage/src/posts.rs`
- Modify: `common/src/seed.rs`
- Modify: `common/src/feed/metadata.rs`
- Modify: `common/src/render.rs`
- Modify if needed: `storage/src/test_support.rs`
- Modify if the new scanner finds another production direct field: the owning
  source file under `POLICED_ROOTS`
- Modify: `docs/adr/0079-rendered-html-sanitization.md`
- Modify: `docs/adr/0123-rendered-html-storage-decode.md`
- Modify: `docs/ARCHITECTURE.md`

**Implementation:**

- [ ] Run the new gate and use its derived population to confirm every current
      production direct `RenderedHtml` field under `POLICED_ROOTS`.
- [ ] Add immediate-above-line markers with specific reasons to legitimate
      fields. Expected current production sites are: `PostRow.rendered_html`,
      `PostRecord.rendered_html`, `PostRevisionRecord.rendered_html`,
      `RenderedPost.rendered_html`, `FeedItem.content_html`, and
      `RenderOutput.html`.
- [ ] Confirm whether `SeededPost.rendered_html` in
      `storage/src/test_support.rs` is exempted by the chosen support-file
      policy or marked as accepted over-inclusion; do not leave its behavior
      accidental.
- [ ] Do not weaken typing or change field names, derives, SQL, serde, storage,
      rendering, or wire behavior.
- [ ] Update ADR-0079 so it states direct `RenderedHtml` fields are now part of
      the `rendered-html-from-trusted` marker gate rather than a residual #701
      gap.
- [ ] Update ADR-0123 so the sqlx decode bridge remains non-sanitizing, but row
      and direct trust-carrying fields are mechanically reviewed by the widened
      gate.
- [ ] Update `docs/ARCHITECTURE.md` with the projected current truth from those
      ADRs.

**Check:**

- [ ] Run `devtool run -- cargo xtask check --no-test`.

---

### Task 4: Full Check, Commit, And Review

**Files:**

- All implementation and docs files changed by Tasks 2 and 3.

**Steps:**

- [ ] Run `devtool run -- cargo xtask check`.
- [ ] Stage the checked implementation and docs with `git add`.
- [ ] Commit: `tooling: gate rendered html fields (#701)`.
- [ ] Run the jaunder review flow against the branch work.
- [ ] Address review findings with another checked commit if needed.
- [ ] Stop for the ship gate once review is clean.
