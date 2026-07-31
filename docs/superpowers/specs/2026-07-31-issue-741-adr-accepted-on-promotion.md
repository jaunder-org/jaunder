# Spec — #741: promotion sets `accepted`; `proposed` is illegal on a numbered ADR

- Issue: [#741](https://github.com/jaunder-org/jaunder/issues/741)
- Milestone: none
- Governing ADR: `docs/adr/drafts/promotion-is-the-acceptance-event.md`
  (authored by this issue; numbered at ship)
- Related: [#742](https://github.com/jaunder-org/jaunder/issues/742) (relocates
  promotion to merge; blocked by this)
- Date: 2026-07-31

## Problem

`docs/adr/template.md:3` seeds `- Status: proposed`. `run_promote`
(`xtask/src/adr.rs`) rewrites a draft's `ADR-DRAFT` token and strips one level
from its relative links, but leaves the status line alone.
`file_format_problems` (`xtask/src/adr_readme.rs`) checks the status line's
presence, single-token-ness, and `STATUS_VOCAB` membership — never its value. So
`proposed` is a default state with no transition out of it and nothing that
notices.

11 of the 88 numbered ADRs carry `proposed` (the rest: 72 `accepted`, 5
`superseded`). All 11 were audited against the tree and every one is in force;
none is superseded:

| ADR  | Evidence in force                                                  |
| ---- | ------------------------------------------------------------------ |
| 0063 | `macros/src/{str,id,num}_newtype.rs`; ~30 newtypes in `common/src` |
| 0065 | `ValidatedInput`/`ValidatedTextarea` across 14 `web/src` files     |
| 0066 | `xtask/src/server_fns.rs`                                          |
| 0068 | `Tag` + `TagLabel` in `common/src/tag.rs`                          |
| 0072 | `UtcInstant` in `common/src/time.rs`                               |
| 0073 | `url` in workspace + `common` deps; `AbsoluteUrl` in 9 files       |
| 0081 | `xtask/src/server_fn_coverage/`, `xtask/src/traces/`               |
| 0082 | 55 `endpoint = "/<vertical>/<op>"` sites across 15 files           |
| 0084 | `common/src/media.rs` — `Filename` / `ProfferedFilename`           |
| 0085 | the enumerating gates in `xtask/src/steps/`                        |
| 0086 | `xtask/src/steps/thin_components.rs`                               |

`docs/README.md`'s Status column is generated from those files, so the index
shows `proposed` in the same 11 rows while `adr-readme-parity` stays green — it
faithfully mirrors a wrong file.

## Decision

Per the governing ADR: **`drafts/` is the `proposed` state; promotion is the
acceptance event; a numbered ADR is therefore never `proposed`.**

### 0. One status-line parse, shared by all three consumers

The status line is currently parsed **twice, differently**: `status_token`
matches `- Status:` or a bare `Status:` after `trim_start`;
`file_format_problems` matches only a non-indented `- Status:`. Adding a third
parse for the rewrite is what would let the rewrite and the gate disagree on an
indented or trailing-whitespace line — promotion would then emit a file that
immediately fails `adr-format`.

So this change first extracts a single helper in `adr_readme.rs`:

```rust
/// The status line's 0-based index and the trimmed remainder after its `- Status:`
/// / `Status:` prefix, under the one parse every consumer shares.
///
/// The remainder is returned whole, not pre-split: `file_format_problems` must keep
/// rejecting `- Status: accepted (superseded)` for having more than one token, and a
/// helper that returned only the first token would silently drop that check.
pub(crate) fn status_line(content: &str) -> Option<(usize, &str)>;
```

`status_token` takes the remainder's first whitespace-delimited token;
`file_format_problems` counts the remainder's tokens (so `- Status:` with an
empty remainder still reports "must be a single token", as today). `pub(crate)`
because `adr.rs`'s rewrite consumes it.

`status_token`, `file_format_problems`, and the new promote rewrite are all
rewritten to consume it. This is a refactor with no behavior change of its own,
and it is a prerequisite for the rest — not an optional tidy-up.

### 1. `promote` rewrites the status (`xtask/src/adr.rs`, Pass B)

Pass B already reads each draft body and applies two independent whole-body
transforms — `ADR-DRAFT` → `ADR-NNNN` and `strip_one_level` — before writing
under the assigned number. The status rewrite joins them:

- If `status_line` reports the token `proposed`, that **token** is replaced with
  `accepted` **in place**, preserving the line's prefix, indentation, and any
  trailing content.
- **Every other token is passed through byte-identically.** `superseded`,
  `rejected`, `deprecated` on a draft are deliberate authorial statements and
  must survive promotion.
- A body with no status line is left alone (already malformed; `adr-format`
  reports it on the promoted file, which is the existing behavior).
- Because the rewrite is token-scoped and line-anchored, prose elsewhere in the
  draft containing the word "proposed" is untouched.

Pass B records the per-draft transition so Pass C — which builds the summary —
can report it. The summary line becomes:

```
docs/adr/drafts/<slug>.md -> docs/adr/NNNN-<slug>.md (status: proposed -> accepted)
```

with the parenthetical **omitted entirely** when no rewrite occurred.

### 2. `proposed` is illegal on a numbered ADR (`xtask/src/adr_readme.rs`)

`STATUS_VOCAB`'s only consumer is `file_format_problems` — `sync-readme` and
`render_block` never read it, so "the set `sync-readme` renders" is not a thing
that exists. And `mod adr_readme` is private, so a `pub const` nothing calls is
dead code that `-D warnings` rejects. The five-token vocabulary therefore lives
where it has always actually lived — `docs/adr/template.md` and the
`jaunder-adr` skill — and the constant is **replaced**, not supplemented:

```rust
/// The status tokens legal on a NUMBERED ADR. `proposed` is absent by design:
/// numbering is the acceptance event, so a numbered ADR has been accepted. A draft
/// may still carry `proposed` — drafts are invisible to this gate (numberless, in a
/// subdirectory), and `promote` rewrites the token as it numbers the file.
const NUMBERED_STATUS_VOCAB: [&str; 4] =
    ["accepted", "superseded", "deprecated", "rejected"];
```

`file_format_problems` validates against `NUMBERED_STATUS_VOCAB`, and
special-cases `proposed` so the diagnosis points at the fix rather than at a
list:

```
{filename}: status is `proposed`, but numbering is the acceptance event — a decision
still under consideration belongs in docs/adr/drafts/
```

Validating against the subset is what keeps the two messages consistent: the
out-of-vocabulary error now lists only the four tokens a numbered ADR may carry,
so it can no longer advertise `proposed` as legal while a sibling rule rejects
it.

Drafts are unaffected — they are invisible to `adr_files` (non-recursive
`read_dir`, `is_file`, leading-number filter), so `file_format_problems` never
sees one.

No escape hatch. A genuinely-under-consideration decision belongs in `drafts/`.

### 3. Backfill the 11 (by hand, in this change)

Set exactly the 11 ADRs listed above to `accepted`, then
`cargo xtask adr sync-readme` to refresh the table's status cells. `promote`
does **not** sweep them — it has no knowledge of ADRs it is not promoting, and a
blanket rewrite would silently assert that 11 decisions are in force.

### 4. Documentation this change falsifies

Each of these currently states or implies that `proposed` is legal on a numbered
ADR, or describes a `promote` that does not touch status. All are updated here:

- `CONTRIBUTING.md:135-136` — the "one of
  proposed/accepted/superseded/deprecated/ rejected" sentence.
- `xtask/src/steps/adr_check.rs` module doc — the `adr-format` rule summary.
- `xtask/src/adr_readme.rs` — `file_format_problems`' own doc comment, which
  states the rule as "a single token from `STATUS_VOCAB`".
- `docs/adr/drafts/README.md` — state that a draft's `proposed` becomes
  `accepted` at promotion.

`.claude/skills/jaunder-adr/SKILL.md` also needs updating (step 2's vocabulary,
step 5's description of `promote`, and "Change an existing ADR's status"), but
`.claude/` is **untracked** — `git ls-files .claude` is empty — so that edit
cannot ship on this branch and is not a branch acceptance criterion. It is done
in the working checkout as a separate, un-shipped change.

### 5. `docs/adr/template.md` is unchanged

It keeps seeding `- Status: proposed`: that is the honest state of a draft, it
keeps a draft a well-formed ADR minus its number (`jaunder-adr` step 2), and it
is gate-exempt because it carries no leading number.

## Tests

- `adr_readme.rs` — `status_line`: canonical, **indented**, **bare `Status:`**,
  multi-token remainder, empty remainder, absent. Assert index **and**
  remainder. Note that trailing whitespace is _not_ discriminating —
  `file_format_problems` already `.trim()`s, so both parses agree on it today;
  only indentation and the bare form actually diverge.
- `adr.rs` — promote rewrites a `proposed` draft to `accepted`; leaves a
  `superseded` draft and a `rejected` draft byte-identical; leaves a draft whose
  _prose_ contains "proposed" unchanged outside the status line; rewrites an
  **indented** status line (the discriminating case for Decision 0 — a
  line-literal implementation passes the canonical case and fails this one).
- `adr.rs` — extend the existing `promote_numbers_single_draft` (which already
  seeds a `proposed` draft and a markered README) with an assertion that the
  generated row reads `| accepted |`. This is the only coverage of a _newly
  promoted_ ADR's README status cell; `promote_repo` writes no README, so the
  round-trip test below cannot cover it.
- `adr.rs` — the summary carries `(status: proposed -> accepted)` for a
  rewritten draft and **does not** contain `status:` for an already-`accepted`
  draft. Assert on that clause alone, never on the whole summary — the file's
  existing warning (`adr.rs:466-468`) applies verbatim: Pass C always pushes the
  path pair in, so a whole-summary `contains` passes regardless of behavior.
- `adr_readme.rs` — `file_format_problems` flags a numbered ADR reading
  `proposed` with the drafts-pointing message; accepts each of the other four
  tokens; still flags a genuinely out-of-vocab token, and that message no longer
  names `proposed`.
- **Round-trip (the composition test):** promote a draft seeded from
  `docs/adr/template.md`, then assert `format_problems` on the resulting tree is
  empty. This is the one test that proves the rewrite and the gate agree;
  reverting the Pass B rewrite must make **this** test fail, not only the unit
  test.
- The existing `gates_ignore_docs_adr_drafts_subdir` and
  `gates_ignore_docs_adr_template_md` tests still pass unchanged.

## Acceptance

- `cargo xtask validate --no-e2e` **fails** on a numbered ADR whose status is
  `proposed`, naming the file and pointing at `docs/adr/drafts/`.
- Promoting a draft seeded from `docs/adr/template.md` yields a numbered ADR
  with `- Status: accepted` and no human edit to the status line, and that ADR's
  generated `docs/README.md` row reads `accepted`.
- Promoting a draft explicitly marked `superseded` or `rejected` yields that
  same token, unchanged.
- Promoting a draft with an indented or trailing-whitespace status line yields a
  file that passes `adr-format`.
- **Backfill blast radius:** immediately after the backfill,
  `docs/adr/[0-9]*.md` tallies **83 `accepted`, 5 `superseded`, 0 `proposed`**
  (was 72/5/11), and `docs/adr/template.md` still reads `- Status: proposed`. At
  ship the branch's own ADR is promoted, so the merged tree is **84/5/0** — the
  invariant that survives is "0 `proposed`", not a fixed accepted count.
- `adr-readme-parity` green, with the 11 backfilled rows showing `accepted`.
- No occurrence of the five-token list survives in `CONTRIBUTING.md`,
  `adr_check.rs`, or `adr_readme.rs` as the rule for a **numbered** ADR.
- `cargo xtask validate --no-e2e` clean on the branch.

## Non-goals

- **Relocating promotion to merge time** (#742). This spec changes what
  promotion _means_, not _when_ it runs; the decision holds under either flow.
- **Sweeping non-promoted ADRs from `promote`** — deliberately excluded,
  Decision 3.
- **Preserving `STATUS_VOCAB` as a constant.** It is replaced by
  `NUMBERED_STATUS_VOCAB` (Decision 2); the five-token draft vocabulary is
  documentation, not code, because no code path ever validated a draft.
- **Changing the template, `sync-readme`'s generation rules, or the curated
  Title column.**
- **A successor-reference rule for `superseded`/`deprecated` ADRs.** A real gap
  (nothing makes a `superseded` ADR name what superseded it), but a separate
  one.
