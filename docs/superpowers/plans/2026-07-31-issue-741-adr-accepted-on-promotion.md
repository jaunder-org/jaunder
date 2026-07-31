# Plan — #741: promotion sets `accepted`; `proposed` illegal on a numbered ADR

- Spec:
  [2026-07-31-issue-741-adr-accepted-on-promotion.md](../specs/2026-07-31-issue-741-adr-accepted-on-promotion.md)
- Issue: [#741](https://github.com/jaunder-org/jaunder/issues/741)
- Branch: `worktree-issue-741-adr-accepted-on-promotion` (base tag
  `wt-base-issue-741`)
- For agentic workers: drive with **`jaunder-iterate`**; delegate a task via
  **`jaunder-dispatch`** when useful. Tick checkboxes in real time.

## Review header

**Goal.** Make `cargo xtask adr promote` write `- Status: accepted` when it
numbers a draft, and make `proposed` a hard `adr-format` failure on a numbered
ADR — so the status column stops rotting. Backfill the 11 ADRs that already
rotted.

**Scope — in.** `xtask/src/adr_readme.rs` (shared status-line parse, the
numbered vocabulary, the gate), `xtask/src/adr.rs` (Pass B rewrite + summary),
the 11 ADR files, `docs/README.md` (regenerated), and the docs the change
falsifies.

**Scope — out.** Relocating promotion to merge time (#742). Sweeping
non-promoted ADRs from `promote`. Template changes. A successor-reference rule
for `superseded`.

**Tasks.**

- [x] 1. Backfill the 11 + `sync-readme` — data only, no code.
- [x] 2. Extract the shared `status_line` parse — pure refactor.
- [x] 3. `NUMBERED_STATUS_VOCAB` replaces `STATUS_VOCAB`; the gate rejects
     `proposed`.
- [x] 4. Pass B rewrites the status token; the summary reports the transition.
- [x] 5. Round-trip composition test + the promoted-README-row assertion.
- [x] 6. Update the docs the change falsifies.

**Key risks / decisions.**

- **Backfill goes FIRST, and the gate lands after it.** The pre-commit hook runs
  the full `cargo xtask check` and aborts on failure, so a gate-before-backfill
  order would make tasks 3–5 uncommittable without `SKIP_PRE_COMMIT=1` —
  breaking the hook's commit-by-commit-green invariant. Backfill-first is also
  _better_ at catching a missed file, not worse: task 3's gate landing one
  commit later is precisely the check that would catch one. The tree is green at
  every commit.
- **Task 2 is load-bearing, not tidy-up.** Two divergent parses exist today
  (`status_token` trims and accepts a bare `Status:`; `file_format_problems`
  does neither). If the rewrite and the gate disagree about which line is the
  status line, promotion emits a file that immediately fails `adr-format`. Task
  2 must land before 3 or 4.
- **`STATUS_VOCAB` is replaced, not supplemented.** Its only consumer is
  `file_format_problems`, and `mod adr_readme` is private — so keeping it
  alongside `NUMBERED_STATUS_VOCAB` leaves a `pub const` nothing calls, which
  `-D warnings` (`static_checks.rs:52`) rejects. The five-token draft vocabulary
  is documentation (template + skill); it was never validated in code.
- **Verified before planning:** widening the gate's parse to indented / bare
  `Status:` lines changes the verdict on **zero** of the 88 existing ADRs (same
  line index, same token under both parses) — so task 2 cannot silently re-judge
  a file nobody meant to touch.

## Global constraints

- Rust; `xtask` is host-only and rebuilt from the working tree by `cargo xtask`.
- Tests are **in-file `#[cfg(test)]`** — both target modules already use that
  shape. No storage/dual-backend concerns; no ADR-0019 dialect files.
- Run gates from **inside the worktree** via `devtool run -- <cmd>`
  (worktree-aware, honest exit). `ctx_execute` targets the main repo — a false
  pass.
- The pre-commit hook runs the full `cargo xtask check`. Run it before
  committing (**`jaunder-commit`**). **Never `SKIP_PRE_COMMIT=1`** — this plan
  is ordered so it is never needed. **No `Co-Authored-By` trailer.**
- Existing tests that must keep passing untouched:
  `gates_ignore_docs_adr_drafts_subdir`, `gates_ignore_docs_adr_template_md`,
  `status_token_reads_list_and_bare_forms`, and
  **`file_format_problems_flags_each_violation`** — which covers the "single
  token with nothing trailing" rule that task 2's helper must not drop.

---

## Task 1 — Backfill the 11 and resync the README

**Why.** Spec Decision 3. Doing this first keeps every later commit green.

**Files.** The 11 ADRs — 0063, 0065, 0066, 0068, 0072, 0073, 0081, 0082, 0084,
0085, 0086 — each `- Status: proposed` → `- Status: accepted`, edited
individually with `Edit`, not swept with `sed`. Then `docs/README.md` via
`sync-readme`.

**Run.**

```
devtool run -- cargo xtask adr sync-readme
devtool run -- cargo xtask check --no-test        # GREEN (parity resynced)
```

**Verify the blast radius** (spec Acceptance). Two steps — never a pipeline:

```
devtool run -- rg -o --no-filename "^- Status: \w+" docs/adr --glob "[0-9]*.md" --sort path
```

then read the parked `.xtask/run/<id>.out` with Grep. Expect **83 accepted, 5
superseded, 0 proposed**. Then confirm the diff touched only those 11 plus the
README:

```
devtool run -- git diff --stat wt-base-issue-741..HEAD -- docs
```

`docs/adr/template.md` must be absent from that diff and still read
`- Status: proposed`.

**Commit.** `docs(adr): record the 11 ADRs that were accepted in practice`

---

## Task 2 — Extract the shared status-line parse

**Why.** Spec Decision 0. Two parses exist; a third would let the rewrite and
the gate disagree.

**Files.**

- `xtask/src/adr_readme.rs` — add `status_line`; rewrite `status_token` and
  `file_format_problems` to consume it.

**Interface.**

```rust
/// The status line's 0-based index and the trimmed remainder after its `- Status:`
/// (or bare `Status:`) prefix — the one parse every consumer shares: the gate, the
/// table projection, and `adr promote`'s rewrite, so they cannot disagree about an
/// indented line.
///
/// The remainder is returned WHOLE, not pre-split: `file_format_problems` must keep
/// rejecting `- Status: accepted (superseded)` for carrying more than one token, and
/// a helper returning only the first token would silently drop that rule.
pub(crate) fn status_line(content: &str) -> Option<(usize, &str)>;
```

- `status_token` →
  `status_line(c).map_or(String::new(), |(_, r)| r.split_whitespace().next().unwrap_or("").to_string())`.
- `file_format_problems` — its
  `content.lines().find(|l| l.starts_with("- Status:"))` arm becomes a
  `status_line` match; it counts the **remainder's** tokens, so `- Status:` with
  an empty remainder still reports "must be a single token" exactly as today
  (`Some((i, ""))`, not `None`).
- `pub(crate)` because `adr.rs` consumes it in task 4.

**Test** (in-file, `adr_readme.rs`):

```rust
#[test]
fn status_line_parses_index_and_remainder_across_forms() {
    // Indented and bare-`Status:` are THE discriminating cases — they are the only
    // two where the old gate parse and the old status_token parse disagree.
    // (Trailing whitespace is not: file_format_problems already trims.)
    assert_eq!(status_line("# T\n\n- Status: accepted\n"), Some((2, "accepted")));
    assert_eq!(status_line("# T\n\n  - Status: accepted\n"), Some((2, "accepted")));
    assert_eq!(status_line("# T\n\nStatus: superseded\n"), Some((2, "superseded")));
    assert_eq!(status_line("# T\n\n- Status: a (b)\n"), Some((2, "a (b)")));
    assert_eq!(status_line("# T\n\n- Status:\n"), Some((2, "")));
    assert_eq!(status_line("# T\n\nno status\n"), None);
}
```

**Run.**

```
devtool run -- cargo nextest run -p xtask status_line   # FAIL (no such fn) -> PASS
devtool run -- cargo nextest run -p xtask adr           # PASS (all existing, incl.
                                                        # file_format_problems_flags_each_violation)
devtool run -- cargo xtask check --no-test              # GREEN
```

**Commit.** `refactor(xtask): give the ADR status line one parse`

---

## Task 3 — `NUMBERED_STATUS_VOCAB` replaces `STATUS_VOCAB`; the gate rejects `proposed`

**Why.** Spec Decision 2. The gate is what makes the property hold; the rewrite
alone leaves hand-created ADRs free to rot.

**Files.**

- `xtask/src/adr_readme.rs` — replace the constant; change
  `file_format_problems` and its doc comment (which today states the rule as "a
  single token from `STATUS_VOCAB`").

**Interface.**

```rust
/// The status tokens legal on a NUMBERED ADR. `proposed` is absent by design:
/// numbering is the acceptance event (ADR-DRAFT promotion-is-the-acceptance-event),
/// so a numbered ADR has been accepted. A draft may still carry `proposed` — drafts
/// are invisible to this gate (numberless, in a subdirectory), and `promote`
/// rewrites the token as it numbers the file.
const NUMBERED_STATUS_VOCAB: [&str; 4] =
    ["accepted", "superseded", "deprecated", "rejected"];
```

`STATUS_VOCAB` is **deleted** — see the risk note; leaving it would be a
`pub const` with no consumer in a private module, which `-D warnings` rejects.

`file_format_problems` special-cases `proposed` **before** the membership check:

```
{filename}: status is `proposed`, but numbering is the acceptance event — a decision
still under consideration belongs in docs/adr/drafts/
```

**Test.**

```rust
#[test]
fn file_format_problems_rejects_proposed_on_a_numbered_adr() {
    let p = file_format_problems("0007-a.md", 7, "# ADR-0007: A\n\n- Status: proposed\n");
    assert!(p.iter().any(|m| m.contains("docs/adr/drafts/")), "{p:?}");
}

#[test]
fn file_format_problems_accepts_every_numbered_token() {
    for t in NUMBERED_STATUS_VOCAB {
        let body = format!("# ADR-0007: A\n\n- Status: {t}\n");
        assert!(file_format_problems("0007-a.md", 7, &body).is_empty(), "{t}");
    }
}

#[test]
fn out_of_vocab_message_no_longer_advertises_proposed() {
    // Teeth: with the old five-token constant this message renders `"proposed"`
    // verbatim (adr_readme.rs:322 formats {STATUS_VOCAB:?}), telling a numbered ADR
    // that `proposed` is legal while a sibling rule rejects it.
    let p = file_format_problems("0007-a.md", 7, "# ADR-0007: A\n\n- Status: accpeted\n");
    assert!(p.iter().any(|m| m.contains("not one of")), "{p:?}");
    assert!(!p.iter().any(|m| m.contains("\"proposed\"")), "{p:?}");
}
```

**Run.**

```
devtool run -- cargo nextest run -p xtask file_format_problems   # FAIL -> PASS
devtool run -- cargo nextest run -p xtask out_of_vocab           # FAIL -> PASS
devtool run -- cargo xtask check --no-test                       # GREEN — task 1
                                                                 # already backfilled
```

If `adr-format` names **any** ADR here, task 1 missed a file — that is exactly
the catch this ordering buys.

**Commit.** `feat(xtask): a numbered ADR may not be proposed`

---

## Task 4 — Pass B rewrites the status; the summary reports it

**Why.** Spec Decision 1.

**Files.**

- `xtask/src/adr.rs` — a `rewrite_status` helper beside
  `rewrite_stem`/`rewrite_bare`; `run_promote` Pass B and the Pass C summary.

**Interface.**

```rust
/// Replace a `proposed` status token with `accepted`, in place, preserving the
/// line's prefix, indentation, and anything trailing. `None` when there is no status
/// line or its token is not `proposed` — every other token is a deliberate authorial
/// statement (`superseded` records a reversal) and survives promotion
/// byte-identically.
///
/// Token-scoped and line-anchored via `adr_readme::status_line`, so prose elsewhere
/// in the draft containing the word "proposed" is untouched.
pub fn rewrite_status(body: &str) -> Option<String>;
```

Pass B applies it as a third whole-body transform alongside the existing
`ADR-DRAFT` → `ADR-NNNN` replace and `strip_one_level`, recording per draft
whether it fired. Pass C — which builds the summary — appends
` (status: proposed -> accepted)` to that draft's line when it did, and
**nothing** when it did not. (The flag must be carried from B to C; they are
separate loops.)

**Test.**

```rust
#[test] fn promote_sets_accepted_on_a_proposed_draft() {}

#[test] fn promote_preserves_a_deliberate_status() {
    // superseded and rejected both survive byte-identically.
}

#[test] fn promote_rewrites_an_indented_status_line() {
    // THE discriminating case for task 2: a line-literal implementation passes
    // promote_sets_accepted_on_a_proposed_draft and fails this one.
}

#[test] fn promote_leaves_the_word_proposed_in_prose() {
    // "- Status: proposed\n\nWe proposed X earlier." -> only the status line moves.
}

#[test] fn promote_summary_names_the_status_transition() {
    // Assert the `(status: ...)` clause ALONE — never the whole summary. Pass C
    // always pushes the path pair in, so `summary.contains("0002-d.md")` passes
    // regardless of behavior (the standing warning at adr.rs:466-468).
    assert!(summary.contains("(status: proposed -> accepted)"));
}

#[test] fn promote_summary_is_silent_for_an_already_accepted_draft() {
    assert!(!summary.contains("status:"));
}
```

**Run.**

```
devtool run -- cargo nextest run -p xtask promote     # new FAIL -> PASS; existing PASS
devtool run -- cargo xtask check --no-test            # GREEN
```

**Commit.** `feat(xtask): promote records acceptance in the status line`

---

## Task 5 — Round-trip composition test + the promoted README row

**Why.** Spec Tests. Tasks 3 and 4 are each green in isolation while disagreeing
about which line is the status line. The round trip is the only test that proves
they compose, and the inversion check for the whole change. The README assertion
covers the one acceptance criterion neither unit test reaches.

**Files.**

- `xtask/src/adr.rs` — one new test; one assertion added to an existing test.

**Test.**

```rust
#[test]
fn a_promoted_template_draft_passes_adr_format() {
    // Teeth: revert task 4's rewrite and THIS fails (the promoted file is
    // `proposed`, which task 3 rejects) — not merely the task 4 unit test.
    let tmp = promote_repo("round-trip");
    write(&tmp, "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\n- Status: proposed\n- Date: 2026-07-31\n");
    run_promote(&tmp).unwrap();
    assert!(crate::adr_readme::format_problems(&tmp).is_empty());
}
```

And in the existing `promote_numbers_single_draft` (`adr.rs:709`) — which
already seeds a `proposed` draft **and** a markered README, unlike
`promote_repo`:

```rust
assert!(readme.contains("| My Decision | accepted |"), "readme: {readme}");
```

**Run.**

```
devtool run -- cargo nextest run -p xtask a_promoted_template_draft   # PASS
devtool run -- cargo nextest run -p xtask promote_numbers_single      # PASS
```

Then verify the teeth: temporarily revert the `rewrite_status` call in Pass B,
re-run, confirm **this** test fails, restore.

**Commit.** `test(xtask): promotion and the ADR format gate agree`

---

## Task 6 — Update the docs this change falsifies

**Why.** Spec Decision 4.

**Files (on the branch).**

- `CONTRIBUTING.md:135-136` — split the vocabulary sentence: five tokens is what
  a **draft** may carry; a **numbered** ADR carries one of the four, because
  promotion is the acceptance event.
- `xtask/src/steps/adr_check.rs` module doc — the `adr-format` bullet gains the
  numbered-vocabulary rule.
- `docs/adr/drafts/README.md` — a line stating a draft's `proposed` becomes
  `accepted` at promotion.

(`adr_readme.rs`'s own doc comment was already handled in task 3, where the
constant changed.)

**Off-branch, not shippable.** `.claude/skills/jaunder-adr/SKILL.md` needs the
same treatment (step 2's vocabulary, step 5's description of `promote`, "Change
an existing ADR's status"), but `.claude/` is untracked — `git ls-files .claude`
is empty — so it cannot ship here. Edit it in the working checkout and note it
in the PR body; it is deliberately **not** a branch acceptance criterion.

**Run.**

```
devtool run -- cargo xtask validate --no-e2e     # full local gate, GREEN
```

**Commit.** `docs: a numbered ADR is accepted by construction`

---

## Self-review

- Every spec acceptance criterion maps to a task: gate failure → 3;
  template-draft promotion → 4, 5; promoted README row → 5; deliberate tokens
  preserved → 4; indented line → 2, 4; blast-radius tally → 1; docs → 6;
  `validate --no-e2e` clean → 6.
- Ordering: 1 (data) → 2 (parse) → 3 (gate, catches a missed backfill) → 4
  (rewrite) → 5 (composition) → 6 (docs). Every commit is green;
  `SKIP_PRE_COMMIT` never needed. No task depends on anything a later task
  introduces.
- No task smuggles #742 work; no separable concerns surfaced, so there is no
  issue-filing first task.
- Task 5 is the inversion check for tasks 2–4 together; tasks 1–4 each carry
  their own discriminating assertion.
