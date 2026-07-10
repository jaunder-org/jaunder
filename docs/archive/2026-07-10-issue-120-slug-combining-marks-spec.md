# Spec — issue #120: preserve combining-mark scripts (Indic conjuncts) in slug generation

- **Issue:** #120 (Milestone 4 — Emacs blogging front-end)
- **Foundation:** #72, ADR-0025 (Unicode-Preserving, Never-Fail Slug Generation)
- **Owning crate:** `common` (`common/src/slug.rs`)

## Problem — corrected from the issue text (probe-backed)

Slug generation (`slugify_title`) and validation (`Slug::from_str`) both keep a
character iff `char::is_alphanumeric()`, collapsing everything else to a single
`-`. The issue frames this as "Devanagari loses vowel signs, Arabic loses
harakat" — **that framing is inaccurate.** `char::is_alphanumeric()` is
`is_alphabetic() || is_numeric()`, and `is_alphabetic()` carries the Unicode
**Alphabetic** derived property, which _includes_ `Other_Alphabetic` combining
marks. Empirically:

| Mark                                | Cat | `is_alphanumeric()`               |
| ----------------------------------- | --- | --------------------------------- |
| Devanagari vowel sign AA `\u{093E}` | Mc  | **true** (kept)                   |
| Arabic fatha `\u{064E}`             | Mn  | **true** (kept)                   |
| Devanagari **virama** `\u{094D}`    | Mn  | **false** (dropped)               |
| Tamil **pulli** `\u{0BCD}`          | Mn  | **false** (dropped)               |
| combining acute `\u{0301}`          | Mn  | false (usually NFC-composed away) |

So vowel signs and harakat already survive. **The real defect is the
virama/halant/pulli class** — combining marks _without_ the Alphabetic property,
which join consonants into conjuncts. Dropping them changes the word:

| Title    | Current slug            | Correct  |
| -------- | ----------------------- | -------- |
| `नमस्ते` | `नमस-ते` (virama → `-`) | `नमस्ते` |
| `हिन्दी` | `हिन-दी`                | `हिन्दी` |
| `தமிழ்`  | `தமிழ` (pulli lost)     | `தமிழ்`  |

Generation and validation drop identically (shared predicate), so there is **no
resolution mismatch** — but the stored URL misspells the author's language. This
is the ADR-0025 faithfulness goal falling short for conjunct scripts.

## Goal

Broaden the slug charset so a combining mark that legitimately attaches to an
alphanumeric base is preserved, on **both** generation and the `from_str`
chokepoint, while keeping every ADR-0025 invariant: NFC + Unicode-lowercase in
the chokepoint, never-fail generation, the scalar length cap, and
**backward-compatibility as a strict superset** (no migration).

## Design — grapheme-cluster awareness (`unicode-segmentation`)

Classify by **extended grapheme cluster** instead of by isolated scalar. Add
`unicode-segmentation` (the unicode-rs sibling of the already-present
`unicode-normalization`) and share one rule across both sites:

```rust
/// A grapheme is kept iff its base scalar is a Unicode letter/digit. Attached
/// combining marks (vowel signs, viramas, harakat, nuktas) ride along with the
/// base; a standalone mark, a symbol, or an emoji is a grapheme whose base is
/// not alphanumeric and is dropped.
fn base_is_alphanumeric(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}
```

Why this is correct for the virama case: a virama/halant/pulli has
`Grapheme_Cluster_Break = Extend`, and Indic vowel signs are `SpacingMark`, so
UAX #29 attaches both to the **preceding** consonant — e.g. `नमस्ते` segments as
`न | म | स् | ते`, each cluster based on a consonant (`is_alphanumeric` → true),
carrying its virama/vowel-sign. (**Verify empirically in iterate** — see Risks.)

### 1. `slugify_title`

- Normalize once to a `String`: `title.to_lowercase().nfc().collect()`.
- Iterate `.graphemes(true)`; push the whole cluster when
  `base_is_alphanumeric`, else collapse a run to a single `-` (same
  `previous_was_dash` logic, now per-grapheme).
- **Grapheme-aware truncation:** accumulate whole clusters while
  `running_scalar_count + cluster.chars().count() <= MAX_SLUG_CHARS`; stop
  before exceeding. Never split a cluster (which could orphan a mark or bust the
  cap); never exceed the scalar cap `from_str` enforces. Then
  `trim_end_matches('-')`.
- Empty result → `"post"` fallback (unchanged).

### 2. `Slug::from_str` (the chokepoint)

- Normalize unchanged (`to_lowercase().nfc()`).
- Length check unchanged: `normalized.chars().count() > MAX_SLUG_CHARS` →
  reject.
- Validate grapheme-wise: the **first** grapheme must satisfy
  `base_is_alphanumeric` (no leading `-`, no leading/standalone mark); every
  subsequent grapheme must be either `"-"` or `base_is_alphanumeric`. Empty →
  reject.
- This is behaviour-identical to today on all existing inputs (ASCII/CJK
  graphemes are single scalars; `-`, space, `_`, emoji handled exactly as
  before) and additionally accepts mark-bearing clusters — a strict superset.

`base_is_alphanumeric` is the single shared definition both sites call, honoring
ADR-0025's "one chokepoint defines the charset."

### 3. Dependency

Add to `common/Cargo.toml`: `unicode-segmentation = "1"` (unicode-rs family;
pairs with the existing `unicode-normalization = "0.1"`). Update `Cargo.lock`.

## Invariants (all preserved)

- **NFC + chokepoint agreement:** both sites normalize identically; a generated
  slug re-parsed on read (`storage::helpers`) and matched to an inbound URL
  (`web::posts::get_post`) still compares byte-for-byte.
- **Never-fail:** the `"post"` fallback is untouched; `slugify_title` output
  always parses (first kept grapheme is alphanumeric-based, no leading mark).
- **Backward-compatible superset:** ASCII/CJK/accented-Latin (precomposed)
  graphemes are single scalars → identical output; the change only _adds_
  mark-bearing clusters. Existing `[a-z0-9-]` slugs stay valid → **no
  migration**.
- **Length cap:** still `MAX_SLUG_CHARS` scalars; truncation is now
  cluster-aligned so it never splits a grapheme.

## ADR

**Amend ADR-0025** (accepted): its Decision Outcome point 2 documents the
charset as literal `char::is_alphanumeric()` per scalar. Add an amendment
section recording that the charset is defined **per grapheme cluster** — a
cluster is kept iff its base scalar is `is_alphanumeric()`, so attached
combining marks (viramas/vowel-signs/harakat) are preserved. Note it remains a
strict superset (no migration) and clarify the imprecise "vowel signs" framing.
Via `jaunder-adr` this is an in-place amendment to the existing ADR, not a new
number.

## Tests (in-file `#[cfg(test)] mod tests`, `common/src/slug.rs`)

- **Virama round-trip (the fix):** `slugify_title("नमस्ते")` keeps the virama;
  the result `.parse::<Slug>()` equals it (generation↔validation agreement).
  Same for `हिन्दी` and Tamil `தமிழ்`.
- **Regression — already-kept marks stay kept:** Arabic `مَرْحَبًا` (harakat)
  and a Devanagari vowel sign round-trip unchanged.
- **Standalone mark dropped / leading mark rejected:** a lone combining mark in
  a title is dropped by generation; a slug string starting with a combining mark
  is rejected by `from_str`.
- **Backward-compat:** existing ASCII/CJK/`café` cases unchanged (extend the
  current tests, don't replace).
- **Truncation never splits a cluster:** a title of mark-bearing clusters around
  the cap truncates on a cluster boundary and stays ≤ `MAX_SLUG_CHARS` scalars.
- **Chokepoint idempotence:** `slugify_title(x).parse::<Slug>()` round-trips for
  each new case.

## Risks

- **Grapheme-clustering depends on `unicode-segmentation`'s Unicode version.**
  The virama-attaches-to-base behaviour is UAX #29 and stable, but the empirical
  `graphemes(true)` grouping of the Indic examples must be **verified in
  iterate** (a spike test) before trusting the design — the issue's premise was
  already wrong once, so confirm, don't assume.
- **Homograph/confusable surface** is unchanged by this (ADR-0025 open item);
  out of scope.
