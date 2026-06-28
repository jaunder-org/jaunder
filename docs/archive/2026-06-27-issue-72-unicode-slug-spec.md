# Issue #72 — Unicode-preserving, never-fail slug generation (product-wide)

* Status: approved (design), pending implementation
* Deciders: mdorman, Claude
* Date: 2026-06-27
* Milestone: Emacs blogging front-end (#4) — **Slug unit**
* Governing ADR: `docs/adr/0025-unicode-slug-generation.md` (accepted) — no new ADR
* Epic spec: `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md` ("Slug unit")

## Goal

Slugs are the product-wide, user-facing post URLs. Today generation is ASCII-only
and can **hard-fail** (`"日本語"` → `None` → `NoSlugFromPost` → BadRequest), making the
engine hostile to non-western/accented-Latin authors. This unit makes slug generation
**Unicode-faithful** and **guaranteed to succeed**, per ADR-0025, with no data
migration. Surfaced by the Emacs untitled-note path but a general improvement.

## Current state (grounding)

* **Chokepoint** `Slug::from_str` (`common/src/slug.rs:22-41`): enforces
  `[a-z0-9][a-z0-9-]*`, stores the string verbatim. Every inbound URL slug, every
  stored-slug re-parse on read (`storage/src/helpers.rs:180-182`), and serde
  (`#[serde(try_from = "String")]`) funnel through it.
* **Generator** `slugify_title(&str) -> Option<String>` (`common/src/slug.rs:74-94`):
  keeps only `is_ascii_alphanumeric`, lowercases ASCII, collapses to `-`, trims;
  returns `None` when nothing survives.
* **Hard-fail** `NoSlugFromPost` in `PerformUpdateError` and `PerformCreationError`
  (`storage/src/post_service.rs` derivation at 206-215 / 327-340); mapped to HTTP in
  `web/src/posts/server.rs:230,242` and `server/src/atompub/mod.rs:204,218`.
* **Collision handling** reusable: `candidate_slug(seed, attempt)`
  (`post_service.rs:258-265`) + per-author-per-day partial unique index
  `posts_user_date_slug` (`storage/migrations/{sqlite,postgres}/0008_create_posts.sql`,
  recreated in `sqlite/0010_nullable_post_titles.sql`).
* **Inbound decoding**: leptos_router percent-decodes path params before
  `use_params_map` (`web/src/pages/posts.rs:132`), so `%E6%97%A5` reaches
  `get_post` → `slug.parse::<Slug>()` (`web/src/posts/mod.rs:318-320`) already
  decoded to `日`. **Widening `from_str` is sufficient; no manual decode needed.**
* **DB**: `slug TEXT NOT NULL`, no length cap, exact `WHERE p.slug = $2` lookup
  (`storage/src/posts.rs:695-726`). No id exists at slug-derivation time (post is
  inserted *after* the slug is chosen).

## Design (per ADR-0025)

### 1. `Slug::from_str` — widen, normalize, cap

Pipeline, in order:
1. **NFC-normalize** the input (`unicode-normalization`).
2. **Unicode-lowercase** (`str::to_lowercase`, std — already full-Unicode).
3. **Validate charset** on the normalized form: non-empty; first char
   `char::is_alphanumeric()`; every char `is_alphanumeric()` or `'-'`.
   (`is_alphanumeric()` is true for `日`/`é`/`я`/`٣`, false for symbols/emoji and
   combining marks — matching the generator.)
4. **Enforce length ≤ 80 chars** (post-normalization char count).

The stored `Slug` holds the normalized form. Because write, read-path re-parse, and
inbound lookup all pass through this gate, the compared byte strings agree. The
`InvalidSlug` error message is updated to describe the Unicode + length rule.

### 2. Backward compatibility & the length-cap safety net

Existing `[a-z0-9-]` slugs are charset-valid under the superset and NFC-invariant, so
**no migration**. The length cap is enforced in `from_str` (chosen for a clean
system-wide invariant), which is the one new constraint that could reject a
*pre-existing* slug longer than 80 chars on read. Mitigation: a regression test locks
the 80/81-char boundary, and the full `validate` (incl. e2e against seeded data) must
stay green — a too-long fixture slug would surface there. Residual risk: production
slugs > 80 chars (uncommon; ≈ an 80-char title) — accepted.

### 3. `slugify_title` — infallible & Unicode-preserving

Signature `Option<String>` → **`String`**. Keep `is_alphanumeric()` chars
(Unicode-lowercased), collapse non-kept runs to a single `-`, trim leading/trailing
`-`, truncate to 80 chars at a char boundary (re-trim trailing `-`). When the result
is empty (emoji/symbol-only/untitled), return the bare fallback **`"post"`**. The
existing per-author-per-day collision retry then yields `post`, `post-2`, `post-3`, …

### 4. Retire `NoSlugFromPost`

Generation is now infallible, so:
* Simplify the derivation in `post_service.rs` (no `.ok_or(NoSlugFromPost)`).
* Delete `NoSlugFromPost` from `PerformUpdateError` and `PerformCreationError`.
* Remove its match arms / HTTP mappings in `web/src/posts/server.rs` and
  `server/src/atompub/mod.rs`, and update the asserting tests
  (`post_service.rs:547,824,866`; `web/src/posts/server.rs:270,301`;
  `server/src/atompub/mod.rs:386,410`).
* `InvalidSlug` stays — an explicit bad `slug_override` is still rejectable.

### 5. Dependency

Add `unicode-normalization` as a **direct** dependency of `common` (already present
transitively in `Cargo.lock`; `deny.toml` allows MIT/Apache-2.0). Used only for NFC.

## Edge cases / tests

* `slugify_title`: `"café"`→`café`, `"Héllo"`→`héllo`, `"日本語"`→`日本語`,
  `"Москва"`→`москва`, `"🚀🎉"`→`post`, `"!!!"`→`post`, `"   "`→`post`; never empty.
* `Slug::from_str`: accepts legacy `[a-z0-9-]` and Unicode forms; rejects
  symbol/emoji-only, leading `-`, empty, and > 80 chars; 80 passes / 81 fails.
* **NFC**: a slug stored NFC is found by an NFD-encoded inbound request (both
  normalize to the same bytes).
* **Round-trip**: an inbound percent-encoded `日` path segment resolves to the stored
  `日` post (decode happens upstream; `from_str` normalizes).
* Collision suffix still works on a Unicode base (`café`, `café-2`).
* **Backend parity** (SQLite + Postgres): the unique index and `WHERE slug = ?`
  lookup behave identically for a Unicode slug.

## Conventions

Backend parity across SQLite + Postgres; coverage policy; verify ladder per
`CONTRIBUTING.md`. No `Co-Authored-By`. All work on
`worktree-issue-72-unicode-slug`, never `main`.
