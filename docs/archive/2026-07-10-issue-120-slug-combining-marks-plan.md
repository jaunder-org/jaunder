# Plan — issue #120: preserve combining-mark scripts (Indic conjuncts) in slug generation

**Spec:**
[`docs/superpowers/specs/2026-07-10-issue-120-slug-combining-marks.md`](../specs/2026-07-10-issue-120-slug-combining-marks.md)
**For agentic workers:** drive with **`jaunder-iterate`** (delegate via
**`jaunder-dispatch`** when useful); commit with **`jaunder-commit`**.

## Review header

**Goal:** widen the slug charset so combining marks attached to an alphanumeric
base (viramas/vowel-signs/harakat) are preserved, by classifying per **grapheme
cluster** instead of per scalar — on both `slugify_title` and the
`Slug::from_str` chokepoint. Keep every ADR-0025 invariant (NFC, never-fail,
scalar cap, strict superset → no migration).

**Scope**

- _In:_ `common/src/slug.rs` (helper + `slugify_title` + `from_str` + tests);
  `common/Cargo.toml`/`Cargo.lock` (`unicode-segmentation`); amend ADR-0025.
- _Out:_ any storage/web/schema change; homograph/confusable handling (ADR-0025
  open item); the never-fail fallback and `MAX_SLUG_CHARS` value (unchanged).

**Tasks**

1. Grapheme-aware slug charset in `common/src/slug.rs` (+ dep + tests).
2. Amend ADR-0025 to record the per-grapheme charset.

**Key risks / decisions**

- **Chokepoint agreement is load-bearing:** `slugify_title` and `from_str` must
  share the exact same keep-rule (`base_is_alphanumeric`) so a generated slug
  always re-parses and inbound lookups match. Both call the one helper.
- **Grapheme-aware truncation** must never split a cluster (would orphan a mark)
  and must stay ≤ `MAX_SLUG_CHARS` scalars (what `from_str` enforces).
- **Strict superset / no regression:** ASCII/CJK/precomposed-Latin graphemes are
  single scalars → byte-identical output; existing integration tests that assert
  CJK/accented slugs must stay green. (Design **spike-verified**: `नमस्ते` →
  clusters `["न","म","स्ते"]`, all preserved; Arabic + `café` unchanged.)

## Global constraints

- **No `Co-Authored-By` trailer.** Every commit passes the pre-commit gate — run
  `cargo xtask check` clean first (**`jaunder-commit`**); serialize edit → gate
  → commit.
- **Coverage (ADR-0050):** every new executable line in `slug.rs` must be
  covered by the new tests (grapheme keep/drop, truncation `break`, `from_str`
  accept/reject/leading-mark). No `cov:ignore` needed.
- **Import discipline:** `use unicode_segmentation::UnicodeSegmentation;` at the
  top; call `.graphemes(true)` unqualified.

---

## Task 1 — Grapheme-aware slug charset

**Files (edit):** `common/src/slug.rs`, `common/Cargo.toml`, `Cargo.lock`.

### 1a. Dependency

`common/Cargo.toml`, after `unicode-normalization = "0.1"`:

```toml
unicode-segmentation = "1"
```

Run any `cargo` command once so `Cargo.lock` records
`common → unicode-segmentation` (locked 1.13.2, already vendored);
`git add Cargo.lock`. No flake change — crane builds from the lock, the crate is
already present.

### 1b. Shared keep-rule + rewritten functions

Add the import (`use unicode_segmentation::UnicodeSegmentation;`) and a private
helper, then rewrite both sites to call it:

```rust
/// A grapheme cluster is kept in a slug iff its base scalar is a Unicode letter
/// or digit (`char::is_alphanumeric`). Attached combining marks — vowel signs,
/// viramas, harakat, nuktas — ride along with the base; a standalone mark, a
/// symbol, or an emoji has a non-alphanumeric base and is dropped. This one rule
/// is shared by generation and the `from_str` chokepoint so they always agree.
fn base_is_alphanumeric(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}
```

`slugify_title` (replace the body):

```rust
pub fn slugify_title(title: &str) -> String {
    let normalized: String = title.to_lowercase().nfc().collect();

    let mut slug = String::new();
    let mut previous_was_dash = false;
    for g in normalized.graphemes(true) {
        if base_is_alphanumeric(g) {
            slug.push_str(g);
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    // Trim a trailing '-', then truncate to the cap on a grapheme boundary: a
    // cluster (base + its marks) is never split, and the result stays within the
    // MAX_SLUG_CHARS scalar budget that `from_str` enforces.
    let trimmed = slug.trim_end_matches('-');
    let mut capped = String::new();
    let mut count = 0usize;
    for g in trimmed.graphemes(true) {
        let glen = g.chars().count();
        if count + glen > MAX_SLUG_CHARS {
            break;
        }
        capped.push_str(g);
        count += glen;
    }
    let capped = capped.trim_end_matches('-');

    if capped.is_empty() {
        "post".to_owned()
    } else {
        capped.to_owned()
    }
}
```

`Slug::from_str` (replace the char-scan with a grapheme-scan; normalization and
length check unchanged):

```rust
let normalized: String = s.to_lowercase().nfc().collect();
if normalized.chars().count() > MAX_SLUG_CHARS {
    return Err(InvalidSlug);
}
let mut graphemes = normalized.graphemes(true);
// First grapheme must be a letter/digit-based cluster (no leading '-' or mark).
let first = graphemes.next().ok_or(InvalidSlug)?;
if !base_is_alphanumeric(first) {
    return Err(InvalidSlug);
}
// Remaining graphemes: a hyphen, or a letter/digit-based cluster (its attached
// combining marks come with it).
if !graphemes.all(|g| g == "-" || base_is_alphanumeric(g)) {
    return Err(InvalidSlug);
}
Ok(Slug(normalized))
```

Also refresh the prose doc comments on `Slug` (lines ~12-13) and `slugify_title`
(lines ~85-87) to say the charset is per grapheme cluster (base letter/digit +
attached combining marks) rather than per `char::is_alphanumeric()` scalar.

### 1c. Tests (in-file `#[cfg(test)] mod tests`)

Add these; keep all existing tests (they assert the unchanged superset
behaviour).

```rust
#[test]
fn slug_preserves_indic_conjunct_marks() {
    // Virama/pulli (Mn, not Alphabetic) were dropped, breaking conjuncts.
    for word in ["नमस्ते", "हिन्दी", "தமிழ்"] {
        let nfc: String = word.to_lowercase().nfc().collect();
        assert_eq!(slugify_title(word), nfc, "slugify dropped a mark in {word:?}");
        // Chokepoint agreement: the generated slug re-parses to itself.
        assert_eq!(slugify_title(word).parse::<Slug>().unwrap().as_str(), nfc);
    }
}

#[test]
fn slug_keeps_alphabetic_marks_regression() {
    // Arabic harakat already survived (Other_Alphabetic); keep it so.
    let arabic = "مَرْحَبًا";
    let nfc: String = arabic.to_lowercase().nfc().collect();
    assert_eq!(slugify_title(arabic), nfc);
    assert_eq!(arabic.parse::<Slug>().unwrap().as_str(), nfc);
}

#[test]
fn slug_drops_standalone_mark_and_rejects_leading_mark() {
    // A lone virama (no base) is degenerate: generation lands on the fallback...
    assert_eq!(slugify_title("\u{094D}"), "post");
    // ...and from_str rejects a slug that starts with a combining mark...
    assert!("\u{094D}a".parse::<Slug>().is_err());
    // ...while a mark attached to a base is accepted.
    assert!("क\u{093E}".parse::<Slug>().is_ok());
}

#[test]
fn slugify_truncates_on_grapheme_boundary_within_cap() {
    // 2-scalar clusters (consonant + vowel sign) well over the cap.
    let title = "क\u{093E}".repeat(MAX_SLUG_CHARS); // 2*MAX scalars
    let slug = slugify_title(&title);
    assert!(slug.chars().count() <= MAX_SLUG_CHARS); // cap honored
    assert_eq!(slug.chars().count() % 2, 0); // never split a 2-scalar cluster
    assert!(slug.parse::<Slug>().is_ok()); // still valid
}
```

**Run steps:**

- `cargo nextest run -p common slug` — the four new tests **FAIL** before 1b's
  rewrite (current per-scalar filter drops the virama / splits clusters),
  **PASS** after.
- `cargo nextest run -p common` — all existing slug tests still pass (superset).
- `cargo nextest run` (or the gate) — the storage/web slug integration tests
  (`test_perform_post_creation_unicode_title_preserves_slug`, the
  conflict/exhaustion cases, `web_posts` slug cases) stay green — none used a
  virama title.

**Verify (behaviour):** drive it via `verify` if useful, but the unit +
integration tests exercise generation ↔ validation ↔ round-trip directly.

**Commit:**
`feat(slug): preserve combining-mark scripts via a per-grapheme charset (#120)`

---

## Task 2 — Amend ADR-0025

**Files (edit):** `docs/adr/0025-unicode-slug-generation.md`.

Append an amendment section (keep the canonical `# ADR-0025:` heading and
`- Status: accepted` line untouched so `adr-format`/`adr-readme-parity` stay
green; no README row change):

```markdown
## Amendment (2026-07-10, #120): per-grapheme charset

Decision Outcome point 2 stated the charset as literal `char::is_alphanumeric()`
per scalar. That silently dropped combining marks **without** the Unicode
Alphabetic property — notably the virama/halant/pulli class (U+094D, U+0BCD, …)
that forms Indic conjuncts — so `नमस्ते` → `नमस-ते`, `हिन्दी` → `हिन-दी`. (Vowel
signs and Arabic harakat carry `Other_Alphabetic`, so they already survived —
the original "preserve Unicode letters/digits" framing was imprecise here.)

Refined rule: the charset is defined **per extended grapheme cluster** — a
cluster is kept iff its base scalar is `is_alphanumeric()`, carrying its
attached combining marks. `slugify_title` and `Slug::from_str` share the one
predicate (`base_is_alphanumeric`), and truncation is cluster-aligned. This
remains a strict superset of the old charset (ASCII/CJK/precomposed-Latin
clusters are single scalars → unchanged), so **no data migration**. (#120,
`unicode-segmentation`.)
```

**Verify:**
`cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check` is not
the ADR gate; run `cargo xtask check --no-test` (includes `adr-format` +
`adr-readme-parity`) → PASS.

**Commit:** `docs(adr): amend ADR-0025 — per-grapheme slug charset (#120)`

---

## Ship (via `jaunder-ship`, after the loop)

- Two-axis review (Standards + Spec) + a cold blind review of the diff.
- `cargo xtask validate` — full gate. This diff is `common`-only (no
  web/server/e2e surface), so `--no-e2e` is defensible for the local run; CI
  runs the full matrix.
- Archive spec + plan to `docs/archive/`; push; PR with a single `Closes #120`;
  merge on CI green; release the project item to **Done**.
