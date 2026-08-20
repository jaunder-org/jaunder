# Issue #564: boundary-aware derived Post summaries

## Problem

Jaunder has two derived-text paths that currently cap by raw scalar count:

- submitted summaries enter through `PostSummary::from_str`, which trims,
  rejects empty input, and rejects values over `MAX_POST_SUMMARY_CHARS`; that
  public boundary is **not** a truncation path;
- fallback summary labels are generated internally from already-non-blank
  `SummarySeed`s and currently truncate by raw scalar count;
- untitled slug derivation in `common::render::derive_post_naming` uses the
  first non-blank body line as a seed and currently caps that seed by raw scalar
  count before slugifying it.

The derived paths are safe but visibly rough. A long title or first body line
can become a draft/timeline label or derived slug seed cut in the middle of a
word or sentence, e.g. `the quick brown fo`. That output is not user-entered
text, but users still see it as automatic Post metadata.

## Scope

In scope:

- One shared internal boundary-truncation helper, used by every derived-text cap
  this issue touches. It owns the sentence/word/hard-cap algorithm so summary
  fallback and render slug seeding cannot drift.
- `common::post_summary::PostSummary::truncated` and
  `common::post_summary::SummarySeed`, including whether the seed type still
  carries enough invariant value after the shared helper exists. The
  implementation must either keep it with a clear non-blank proof role or remove
  the indirection and route fallback construction through an equally explicit
  private helper.
- `common::post_summary::SummarySeed::first_body_line`, if `SummarySeed`
  remains, because body-line seeds have their own tighter
  `MAX_BODY_LINE_SEED_CHARS` excerpt cap before the broader `PostSummary`
  invariant is applied.
- `common::render::derive_post_naming`'s untitled-post first-body-line seed,
  because issue #564 explicitly names the render/body-line derivation path that
  feeds slug generation.
- Storage/web call sites that consume `PostRecord::fallback_summary_label` or
  derived slugs only as needed to keep names/comments truthful.
- Unit coverage for the truncation contract.

Out of scope:

- Submitted `PostSummary` validation semantics. Over-cap submitted summaries
  still fail instead of truncating.
- Changing `MAX_POST_SUMMARY_CHARS` or `MAX_BODY_LINE_SEED_CHARS`.
- HTML/Markdown/Org sentence parsing. This is a plain-text label heuristic over
  the already-selected seed string, not a renderer-aware excerpt generator.
- Adding a new summary-generation subsystem, background job, or database field.

## Decisions

1. **One helper owns boundary truncation.** The implementation may keep
   source-selection functions separate, but every derived-text cap in this spec
   must call the same helper rather than reimplementing sentence/word/hard-cap
   selection in multiple modules.

2. **`SummarySeed` is in scope for simplification.** The plan must explicitly
   decide whether `SummarySeed` still pays for itself once truncation moves into
   the shared helper. Keeping it is acceptable only if its non-blank proof role
   stays clear at call sites; removing it is acceptable only if derived
   `PostSummary` construction still has one local, infallible path whose tests
   prove non-blankness and caps.

3. **Boundary-aware truncation applies only when a cap is needed.** Inputs
   already within the relevant cap are returned unchanged except for the
   existing source trimming.

4. **Prefer complete sentence boundaries, then word boundaries, then hard scalar
   cap.** When a seed exceeds its cap:
   - choose the last sentence terminator (`.`, `!`, or `?`) within the cap when
     it yields a non-empty prefix;
   - otherwise choose the last Unicode whitespace boundary within the cap when
     it yields a non-empty prefix;
   - otherwise fall back to the current hard scalar cap for one long word, URL,
     or token.

5. **The result never exceeds the existing scalar cap.** Counts remain in
   Unicode scalar values to match the current `PostSummary` invariant and tests.
   The implementation must not byte-slice through UTF-8.

6. **No synthetic ellipsis.** Derived labels remain source text prefixes. This
   avoids spending part of the cap on a glyph, avoids implying a user-authored
   punctuation mark, and keeps storage/web comparisons simple.

7. **Trim trailing whitespace after a boundary cut.** The seed is already
   non-blank; trimming a selected cut boundary cannot create an empty value when
   the decision's non-empty-prefix guard is applied.

## Acceptance criteria

- Summary fallback and render slug seeding use one shared boundary-truncation
  helper for their respective caps; tests or code review can verify there is no
  second copy of the algorithm.
- The implementation either keeps `SummarySeed` with a documented non-blank
  proof role or removes it in favor of a simpler single derived-summary
  construction path; it must not leave a confusing two-stage cap pipeline whose
  second cap is known to be a no-op for body-line seeds.
- A derived summary from a long title cuts at the last complete sentence within
  `MAX_POST_SUMMARY_CHARS` when one exists.
- A derived summary from a long title with no sentence boundary cuts at the last
  word boundary within `MAX_POST_SUMMARY_CHARS` when one exists.
- A derived summary from a long token with no usable boundary still hard-caps at
  `MAX_POST_SUMMARY_CHARS` and remains valid UTF-8.
- A derived summary from the first non-blank body line applies the same boundary
  preference within `MAX_BODY_LINE_SEED_CHARS`.
- An untitled Post's body-line-derived slug seed applies the same boundary
  preference within its existing 100-scalar cap before slugification.
- Submitted over-cap summaries still fail through `PostSummary::from_str`; they
  are not silently truncated.
- Existing `fallback_summary_label` behavior remains total on `PostBody` and
  still uses no title/slug fallback.

## Verification

- Add or update unit tests in `common/src/post_summary.rs` for sentence, word,
  hard-token, Unicode, body-line, submitted-summary rejection, and the chosen
  derived-summary construction path (`SummarySeed` kept or removed).
- Add or update `common/src/render.rs` tests for the untitled body-line-derived
  slug seed boundary behavior.
- Run the targeted common/storage tests that cover `PostSummary`,
  `derive_post_naming`, and `fallback_summary_label`.
- Run `devtool run -- cargo xtask check` before commit.
