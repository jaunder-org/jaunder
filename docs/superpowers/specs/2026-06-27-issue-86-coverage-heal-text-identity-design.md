# Coverage heal hardening — text-identity re-anchor + line-independent CRAP manifest

**Issues:** #86 (text-identity heal/re-anchor), #7 (crap-manifest churn). One cycle.
**Milestone:** Verify-gate hardening.
**Date:** 2026-06-27

## Problem

The committed coverage artifacts — `coverage-baseline.json` (accepted-uncovered
line gaps) and `crap-manifest.json` (per-function CRAP scores) — drift on nearly
every change, forcing repeated manual regeneration. Two distinct defects:

- **#86:** The auto-heal (`cargo xtask check`, `Mode::Fix`) only heals when the
  line-identity classifier is *clean* (no regressions, no new-uncovered *by line
  number*). Any line-shifting change whose diff models an accepted gap line as
  deleted-then-reappeared (modified neighbour under `--unified=0`) produces a
  *phantom* regression/new-uncovered even though coverage is provably unchanged.
  The gate then fails and recovery requires the hidden `__regen-baseline` command
  plus a hand-rolled `jq` no-lowering check. Hit by the #51/#52/#53 transaction
  refactors and the #63 sweep.
- **#7 (residual):** The pretty-JSON format half already landed (commits
  `50c0383`, `330d26d`). What remains: the manifest stores a per-function `line`
  field and is rewritten wholesale, so any line-shift or function add/remove
  rewrites all ~692 entries even when **no CRAP score changed**.

These also make the gate fragile under **overlapping changes**: when a
concurrently-landed branch shifts lines in a shared file, the merged tree's
baseline lines mismatch → phantom failures → manual regen. Closing both defects
is also a prerequisite for automating the gate (#99): a pre-commit hook that ran
today's churny heal would generate spurious diffs on every commit.

## Scope

**In (this cycle):**

1. #86 — sound text-identity re-anchor predicate + gate/heal behaviour.
2. #7 — line-independent CRAP manifest (compare key + rewrite trigger), `line`
   retained as a labelled hint.
3. A keep-ours merge driver for the two JSON artifacts (mechanism only).

**Out (follow-on issue, filed as plan task 1):**

- Auto-registering the merge driver across clones/worktrees via a self-healing
  installer, and any eager post-merge re-heal hook — the *automatic wiring*.
  Coherent with #99; this cycle delivers the mechanism, the follow-on delivers
  the automation.

## Design

### 1. #86 — sound text-identity re-anchor

The naïve predicate from the issue text ("current per-file uncovered-text
multiset ⊆ baseline accepted-text multiset") is **unsound**: if a branch covers
one uncovered `    }` (improvement) and a *different* `    }` in the same file
regresses, the multiset is unchanged and it wrongly passes. Equal-count guards
do not help — counts are equal.

The sound predicate keys on **what the diff removed vs. what appeared**:

- `structural_texts(file)` = texts of accepted baseline gaps the classifier put
  in the `structural` bucket (gap line deleted / unmapped by the diff).
- `appeared_texts(file)` = texts of lines the classifier newly flagged
  (`regressions` ∪ `new_uncovered`).
- **Safe re-anchor iff, per file, the `appeared_texts` multiset ⊆
  `structural_texts` multiset** — every newly-flagged uncovered line is
  explained by an accepted gap of identical text that vanished elsewhere (the
  line genuinely *moved*).

Worked cases:

| Case | Classifier | `appeared ⊆ structural`? | Verdict |
|---|---|---|---|
| Pure line-shift (#51/#63) | gap removed here, reappears there, same text | yes | **safe → heal+pass** |
| Net-zero swap (cover one `}`, regress another `}`) | improvement (not structural) + regression | no (`}` not in structural) | **fail** |
| Genuine new uncovered code | new_uncovered, nothing removed | no | **fail** |
| Genuine regression (no shift) | regression, nothing removed | no | **fail** |

This is strictly sounder than the issue text and naturally yields the offending
`file:line:text` list (the `appeared` lines not covered by `structural`) that
#87 will format.

**Primitive.** Factor a single source of truth, reused by the gate and (later)
#88's reanchor command:

```
reanchor_is_safe(structural: &[FileLines+text], appeared: &[FileLines+text])
    -> ReanchorSafety { safe: bool, lowering: Vec<FileLineText> }
```

`lowering` is the per-file `appeared` entries with no matching `structural` text
— the genuine coverage losses.

**Gate logic** (replaces the `verdict.is_clean()`-only heal condition):

1. Run line-classify as today, producing the four buckets.
2. If line-clean (`is_clean()`): heal exactly as today (clean ⟹ safe).
3. If line-dirty: evaluate `reanchor_is_safe(structural, regressions ∪
   new_uncovered)`.
   - **safe** → pure drift. `Mode::Fix`: heal (`Baseline::from_files(current)`,
     renumbered) and **pass**. `Mode::Check`: **pass without mutating**
     (consistent with "validate never writes"; the baseline re-anchors on the
     next Fix run).
   - **not safe** → **fail**, reporting `lowering`.
4. CRAP regressions still independently fail the gate and block heal, as today.

`reanchor_is_safe` generalises the existing clean check: a line-clean verdict has
empty `appeared`, so `appeared ⊆ structural` holds trivially.

**Note on residual ambiguity.** Text-identity cannot distinguish two
identical-text lines in the same file when one is *removed as an accepted gap*
and an unrelated identical-text line *regresses* in the same change — a far rarer
coincidence than the global-multiset hole, and accepted as the bounded cost of
self-healing line-shifts. The line-identity classifier remains the primary
signal; text-identity only *excuses line failures that the diff explains as
moves*, it does not replace classification. Documented in CONTRIBUTING.

### 2. #7 — line-independent CRAP manifest (approach A)

Keep `line` in the manifest as a **labelled, non-authoritative jump-to hint**,
but make both the regression compare and the rewrite trigger ignore it.

- **Compare key** becomes `(crate, file, function, ordinal)` where `ordinal` is
  the index of an entry among those sharing `(crate, file, function)`, ordered by
  `line`. Required for the two real collisions in the current manifest
  (`HandlerError::from` ×8, `AtomPubError::from` ×2). Ordering by line is stable
  under pure shifts (relative order preserved), so the key is shift-stable.
- **Rewrite trigger** compares a `line`-stripped canonical normalisation of the
  report against the committed manifest. Pure line-shifts (same scores) → no
  rewrite. A real change to `crap`/`coverage`/`cyclomatic`, or a function
  added/removed → rewrite **all** entries, refreshing every `line` wholesale.
- `line` therefore lags reality only between a pure-shift commit and the next
  real CRAP change, which refreshes it. Labelled as a hint in a doc comment and
  in the CONTRIBUTING / dispatch-skill schema note.

**Edge case:** ordinals on each side are derived from that side's own (possibly
lagged vs. fresh) line order. A function added/removed *within* a same-name group
can misalign ordinals for that group — a legitimate structural change that is
correctly surfaced, and confined to the rare same-name-in-file case. Documented.

### 3. Keep-ours merge driver

Both JSON artifacts are generated; their authoritative content is regenerated by
the (now sound, now quiet) Fix-mode heal. So the merge strategy is **keep-ours,
no conflict markers**, deferring authority to the next `check`:

- Committed `.gitattributes` maps `coverage-baseline.json` and
  `crap-manifest.json` to a `merge=coverage-keepours` driver.
- The driver is defined as the trivial keep-ours driver (`driver = true`),
  registered via `git config` — provided as a one-shot `cargo xtask` registration
  subcommand (since git config is not version-controlled) plus documentation.
- **Safety:** keep-ours is the *safe* direction. If a merge genuinely lowers
  coverage, the stale "ours" baseline does not accept the new gap → the gate
  **fails** post-merge rather than silently accepting it. (Taking theirs/union
  could mask a lowering.) The gate remains the authority; the driver only removes
  the textual conflict.

This eliminates the "every merge conflicts on the manifests → take ours, regen"
toil for overlapping changes — but only the *mechanism*; automatic registration
is the follow-on.

## Testing

- `reanchor_is_safe`: move (safe), genuine regression (fail), net-zero swap
  (fail), new code (fail), multi-file isolation, `lowering` contents.
- Gate: Fix heals+passes a pure-shift; Check passes a pure-shift without writing;
  CRAP regression still blocks heal.
- CRAP compare: regression detected under line-shift (ordinal key), same-name
  collision disambiguated, ordinal stable under shift.
- Rewrite trigger: no-op on pure line-shift; rewrites (and refreshes lines) on a
  real score/cyclomatic change and on function add/remove.
- Merge driver: keep-ours leaves ours intact, no conflict markers; post-merge
  gate still fails a genuine lowering.

## Docs

- CONTRIBUTING coverage section: text-identity re-anchor semantics + the
  documented residual ambiguity; `line` as a non-authoritative hint.
- dispatch-skill manifest schema note: `line` is a hint that may lag.
- ADR: record the text-identity re-anchor semantics (a non-obvious heal-safety
  decision diverging from the issue's literal prescription) — number assigned in
  the plan, with its row added to the `docs/README.md` ADR table.

## Risks

- **Text-identity false-pass** on identical-text same-file coincidences
  (bounded; documented; line-classifier remains primary).
- **Stale `line` hints** under many overlapping merges (self-limiting; refreshed
  on the next real CRAP change; labelled).
- **Merge driver requires registration** to take effect — until the follow-on
  automates it, an unregistered clone still hits conflicts. Documented one-shot
  command bridges the gap.
