# #1662 Single-line authored Post titles — implementation outline

> Execute with `jaunder-iterate` (and `jaunder-dispatch` only for a bounded
> independent slice). This outline is warranted by the AtomPub public
> write-contract change and fallible shared title derivation across storage and
> server boundaries. The approved
> [spec](../specs/2026-09-24-issue-1662-single-line-post-titles.md) is
> authoritative.

## Scope

In: one authored Post Title invariant, web/Org/AtomPub create and update
rejection, early Emacs publish and safe pull, regression proof, glossary and
numberless ADR with architecture projection.

Out: migration/backfill, title length policy, new markup or rendering rules,
unrelated metadata validation, #1657's editor-title reconstruction beyond
necessary compatibility.

## Task outline

- [x] **1. Reject invalid typed and heading-derived titles before storage
      effects.**
  - Contract: `PostTitle` rejects all seven separator classes before trimming;
    an invalid Markdown or Org candidate in `derive_post_naming` becomes a typed
    failure, never `None`. Propagate it through all four storage service
    create/update call sites and exhaustive error conversions to web Validation
    and AtomPub 400 in the same compiling slice, before body canonicalization,
    Media ownership resolution, write scopes, or slug mutation. Keep
    absent/blank-title fallback and valid single-line source semantics.
  - Verification: a table of all seven separators at beginning, middle, end, and
    alone; targeted naming tests; dual-backend `#[apply(backends)]`
    create/update tests for non-CR/LF derived heading separators, unchanged Post
    on rejection, and valid derived titles. Explicit invalid source strings
    cannot enter storage's typed `Option<&PostTitle>`; test them at ingress in
    Task 2. Project the draft title ADR and glossary into `docs/ARCHITECTURE.md`
    and `CONTEXT.md` in this slice.

- [x] **2. Reject malformed titles at web, Org, and AtomPub ingress.**
  - Contract: use Task 1's already-compiled service error projections;
    distinguish invalid explicit AtomPub title from absence rather than using
    `.ok()`. Blank AtomPub `<title>` without any separator still means absent.
    Server Org header validation rejects repeated `#+TITLE:` even if structured
    title wins; preserve other AtomPub leniency and ADR-0155 atomic precedence.
    Handler rejection leaves metadata/body intact.
  - Verification: focused mapping and live handler create/update tests for
    explicit, repeated header, blank/absent, and precedence paths; browser test
    on `/posts/new` with repeated Org title lines proves validation and no saved
    Post. Capture comparable before/after composer error-state screenshots if
    presentation changes. Preserve existing rendered `<br>` behavior.

- [ ] **3. Fail safely at Emacs publish and pull boundaries.**
  - Contract: validate local TITLE source as the very first publish preflight,
    before Service Document fetch, any link or Media work, intent/checkpoint
    mutation, or write-back. Validate a pulled Member title before constructing
    replacement Org bytes or local Media, so malformed remote title cannot
    replace a matched local Post. Keep valid one-line and absent-title
    semantics.
  - Verification: table-driven pure ERT for all seven separators and
    representative edge/middle placement; live rejected-publish proofs for
    repeated TITLE and a title with an edge separator, each with zero
    network/Media calls and unchanged buffer/file/create intent; malformed
    remote pull no-clobber proof; valid single-line publish/pull round-trip.

## Risk checks

- The existing `derive_post_naming` tuple currently maps an invalid extracted
  heading to absence; making it fallible must cover all storage call sites
  without widening unrelated `PostBody` or slug policy.
- AtomPub currently uses `.ok()` to swallow _every_ `PostTitle` parse failure;
  distinguish separator-bearing invalid input from blank non-line-breaking
  whitespace before mapping errors. Keep HTTP 400 before storage side effects.
- Emacs validation and server title grammar must agree on the seven classes;
  prevent a late 400 from following a successful Media upload. Pulled malformed
  titles must not turn into Org header/body injection.
- ADR-0204 permits rendered `<br>`: the new source invariant must not change
  title sanitizer, web wrapping, or Syndication Feed projection. No production
  titles require compatibility migration.
- Finish each checked task with focused proof and `jaunder-commit`; the hook
  gates staged changes. Final `jaunder-ship` review, applicable verification,
  and PR/CI observation precede a separate merge-approval halt.
