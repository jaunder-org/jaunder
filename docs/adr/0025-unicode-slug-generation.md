# ADR-0025: Unicode-Preserving, Never-Fail Slug Generation

* Status: accepted
* Deciders: mdorman, Claude
* Date: 2026-06-26

## Context and Problem Statement

Slugs are the product-wide user-facing post URLs. Today `slugify_title`
(`common/src/slug.rs:75-94`) keeps only `[a-z0-9]` and drops everything else, so
`"café"` → `"caf"`, `"日本語"` → `None`; a `None` becomes `NoSlugFromPost`
(`storage/src/post_service.rs:286`) and the publish **hard-fails**. The `Slug` newtype
`FromStr` (`slug.rs:25-40`) enforces `[a-z0-9][a-z0-9-]*` and is the **single
chokepoint** both slug *generation* and inbound *URL resolution*
(`web/src/posts/mod.rs:282-283`) funnel through. This makes the engine hostile to
non-western and accented-Latin authors and able to refuse a post outright. The Emacs
untitled-note path surfaced it, but it is a general defect.

## Decision Drivers

* Be actively attractive to non-western and Gen-Z audiences, not merely non-hostile.
* Slug generation must **never** hard-fail.
* One chokepoint defines the system-wide slug charset — change it once, correctly.
* Don't break existing slugs or require a data migration.

## Decision Outcome

**(A) Unicode-preserving slugs + a guaranteed fallback.**

1. **Never hard-fail.** When derivation yields nothing usable, synthesize a
   non-semantic slug (e.g. `post-<id>` or a short hash). Every post gets a slug.
2. **Preserve Unicode.** Broaden the charset to Unicode letters/digits
   (`char::is_alphanumeric()` — true for `日`/`é`/`я`/`٣`, false for symbols **and
   emoji**, which are Unicode *Symbols*, not letters). `日本語`→`日本語`,
   `café`→`café`; symbol/emoji-only input keeps nothing → lands on the fallback.
3. **Normalize in the chokepoint.** Centralize **NFC** normalization and
   Unicode-lowercasing in `Slug::from_str`, so stored slugs and inbound-URL lookups
   compare consistently (the DB unique index and `WHERE slug = ?` compare bytes; the
   wire form is percent-encoded UTF-8).
4. **Add a length cap** (CJK inflates ~9 bytes/char percent-encoded; today there is none).
5. **Backward compatible:** existing `[a-z0-9-]` slugs remain valid (the new charset is
   a superset) → **no data migration**.

Rejected: ASCII transliteration (`deunicode`/`any_ascii`). It keeps URLs pure-ASCII
but romanizes a user's language (e.g. CJK→pinyin) — lossy, often wrong, and the exact
second-class treatment we want to avoid.

## Consequences

* Good: faithful URLs for CJK/Cyrillic/Arabic/accented Latin; publish always succeeds.
* Good: existing slugs and links keep working; no migration.
* Bad: percent-encoded UTF-8 is ugly in contexts that don't decode for display (raw
  server logs, plain-text email) — cosmetic, not functional.
* Bad: NFC discipline is now load-bearing — every `Slug` construction must normalize,
  or visually-identical slugs in different normal forms won't match.
* Open: confusable/homograph slugs (Cyrillic `а` vs Latin `a`) can be visually
  identical but distinct; per-author-per-day scoping bounds the impact; revisit only if
  abused. Requires verifying Leptos percent-decodes the slug path segment to UTF-8
  before `Slug` parsing.
