# Unicode-Preserving, Never-Fail Slug Generation — Implementation Plan (issue #72)

**Status:** Executed 2026-06-28 — all tasks landed (commits `5cf0d43`..`2fce271`). Full `cargo xtask validate` (incl. e2e on SQLite + Postgres) green; coverage clean. Code-reviewed (ready to merge); follow-up #120 filed (combining-mark scripts). Note: a one-time production `SELECT max(char_length(slug)) FROM posts` audit is advised before deploy (the 80-char `from_str` cap is enforced read-side).

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make slug generation Unicode-faithful and guaranteed to succeed, with the charset chokepoint normalizing/validating Unicode and a length cap — no data migration.

**Architecture:** Widen the single chokepoint `Slug::from_str` (`common/src/slug.rs`) to NFC-normalize, Unicode-lowercase, accept `char::is_alphanumeric()` ∪ `-`, and cap at 80 chars; make `slugify_title` infallible (Unicode-preserving, bare-`post` fallback, 80-char truncation); make `candidate_slug` cap-aware; retire the now-unreachable `NoSlugFromPost` across storage/web/atompub. e2e (running on both SQLite and Postgres VMs) is the full-stack acceptance test.

**Tech Stack:** Rust (`common`, `storage`, `web`, `server` crates), `unicode-normalization`, Playwright/TypeScript (`end2end/`).

**Spec:** `docs/superpowers/specs/2026-06-27-issue-72-unicode-slug.md`. **Governing ADR:** `docs/adr/0025-unicode-slug-generation.md` (accepted; no new ADR).

## Global Constraints

- **Single chokepoint:** all slug validation/normalization lives in `Slug::from_str` (`common/src/slug.rs`); `slugify_title` is the single generator. Don't add slug logic elsewhere.
- **Length cap:** `MAX_SLUG_CHARS = 80` (Unicode scalar values, post-NFC), enforced in `from_str` AND applied (truncation) in `slugify_title`/`candidate_slug`.
- **Backward compatible:** the new charset is a superset of `[a-z0-9-]`; NFC/lowercase are idempotent on existing slugs → no migration. The 80-char `from_str` cap assumes no existing slug exceeds 80 chars (locked by a boundary test; e2e on seeded data backstops).
- **Never hard-fail:** `slugify_title` always returns a non-empty `String`; fallback base is `"post"`.
- **Backend parity:** SQLite + Postgres — covered by the e2e VMs (`e2e-sqlite`, `e2e-postgres`) for the create→lookup round-trip; slug logic itself is pure (`common`).
- **No `Co-Authored-By`.** Worktree only (`worktree-issue-72-unicode-slug`); review vs `wt-base-issue-72`.
- **Gate:** per-task `cargo xtask check --no-test` (run in `nix develop .#ci`); final `cargo xtask validate` (full, incl. e2e — this feature has an e2e deliverable).

---

### Task 1: Widen the chokepoint — `Slug::from_str` + `MAX_SLUG_CHARS`

**Files:**
- Modify: `common/Cargo.toml` (add `unicode-normalization`)
- Modify: `common/src/slug.rs` (`from_str`, doc comments, error message, const; tests)

**Interfaces:**
- Produces: `common::slug::MAX_SLUG_CHARS: usize = 80`; `Slug::from_str` accepting NFC/lowercased Unicode alphanumerics + `-`, ≤ 80 chars; `Slug` stores the normalized form.

- [ ] **Step 1: Add the dependency.** In `common/Cargo.toml` under `[dependencies]`, add:

```toml
unicode-normalization = "0.1"
```

(Already in `Cargo.lock` transitively; `deny.toml` allows MIT/Apache-2.0. Confirm the resolved version with `cargo tree -p unicode-normalization` and pin the major as it resolves.)

- [ ] **Step 2: Rewrite the failing tests first.** Replace the `slug_rejects_invalid_values`, `slug_parses_valid_values`, and add Unicode/cap tests in `common/src/slug.rs`'s `tests` module:

```rust
    #[test]
    fn slug_accepts_ascii_and_unicode_lowercasing() {
        assert_eq!("hello-world".parse::<Slug>().unwrap().as_str(), "hello-world");
        assert_eq!("Héllo".parse::<Slug>().unwrap().as_str(), "héllo"); // uppercase now lowercased
        assert_eq!("日本語".parse::<Slug>().unwrap().as_str(), "日本語");
        assert_eq!("Москва".parse::<Slug>().unwrap().as_str(), "москва");
        assert_eq!("café".parse::<Slug>().unwrap().as_str(), "café");
    }

    #[test]
    fn slug_normalizes_nfd_input_to_nfc() {
        // "cafe" + combining acute (NFD) must compare equal to NFC "café".
        let nfd = "cafe\u{0301}";
        assert_eq!(nfd.parse::<Slug>().unwrap().as_str(), "café");
    }

    #[test]
    fn slug_rejects_invalid_values() {
        assert!("".parse::<Slug>().is_err()); // empty
        assert!("-hello".parse::<Slug>().is_err()); // leading hyphen
        assert!("hello world".parse::<Slug>().is_err()); // space (not alnum/-)
        assert!("hello_world".parse::<Slug>().is_err()); // underscore
        assert!("hello@world".parse::<Slug>().is_err()); // symbol
        assert!("🚀".parse::<Slug>().is_err()); // emoji is a Symbol, not alnum
    }

    #[test]
    fn slug_enforces_length_cap() {
        let max: String = "a".repeat(MAX_SLUG_CHARS);
        assert!(max.parse::<Slug>().is_ok());
        let over: String = "a".repeat(MAX_SLUG_CHARS + 1);
        assert!(over.parse::<Slug>().is_err());
    }
```

Update `slug_serde_serializes_as_plain_string_and_validates_on_deserialize` to keep `"Bad Slug"` (still invalid: space).

- [ ] **Step 3: Run the new tests — verify they FAIL.**

Run: `cargo test -p common slug`
Expected: FAIL (current `from_str` is ASCII-only; `Héllo`/`日本語` rejected, no `MAX_SLUG_CHARS`).

- [ ] **Step 4: Implement.** In `common/src/slug.rs`: add the import + const, rewrite `from_str`, fix the doc comment (line 6) and `InvalidSlug` message.

```rust
use unicode_normalization::UnicodeNormalization;

/// Maximum slug length in Unicode scalar values (counted post-NFC).
pub const MAX_SLUG_CHARS: usize = 80;
```

```rust
#[derive(Debug, Error)]
#[error("slug must be non-empty, ≤80 chars, and contain only Unicode letters/digits and '-'")]
pub struct InvalidSlug;
```

```rust
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize so stored slugs and inbound-URL lookups compare consistently
        // regardless of case or Unicode normal form: lowercase (full-Unicode),
        // then NFC-compose. Idempotent on an already-stored slug, so the read-path
        // re-parse and inbound lookups agree on bytes.
        let normalized: String = s.to_lowercase().nfc().collect();
        let mut chars = normalized.chars();
        let first = chars.next().ok_or(InvalidSlug)?;
        if !first.is_alphanumeric() {
            return Err(InvalidSlug);
        }
        if !chars.all(|c| c.is_alphanumeric() || c == '-') {
            return Err(InvalidSlug);
        }
        if normalized.chars().count() > MAX_SLUG_CHARS {
            return Err(InvalidSlug);
        }
        Ok(Slug(normalized))
    }
```

Update the `Slug` doc comment (line 6) to describe the Unicode rule instead of `[a-z0-9][a-z0-9-]*`.

- [ ] **Step 5: Run tests — verify PASS.**

Run: `cargo test -p common slug`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add common/Cargo.toml common/src/slug.rs Cargo.lock
git commit -m "feat(slug): Unicode-normalizing, length-capped Slug::from_str chokepoint"
```

---

### Task 2: Infallible Unicode `slugify_title`

**Files:**
- Modify: `common/src/slug.rs` (`slugify_title`, its tests)

**Interfaces:**
- Consumes: `MAX_SLUG_CHARS` (Task 1).
- Produces: `slugify_title(&str) -> String` — Unicode-preserving, NFC/lowercased, `is_alphanumeric()`-only with `-` separators, ≤ 80 chars, never empty (bare `"post"` fallback). **Signature changes from `Option<String>` to `String`.**

- [ ] **Step 1: Rewrite the tests.** Replace the three `slugify_title_*` tests in `common/src/slug.rs`:

```rust
    #[test]
    fn slugify_title_preserves_unicode_lowercased() {
        assert_eq!(slugify_title("Café"), "café");
        assert_eq!(slugify_title("日本語"), "日本語");
        assert_eq!(slugify_title("Москва"), "москва");
        assert_eq!(slugify_title("Hello, World from Rust"), "hello-world-from-rust");
        assert_eq!(slugify_title("  ---Héllo!!!  "), "héllo");
    }

    #[test]
    fn slugify_title_falls_back_to_post_when_no_letters() {
        assert_eq!(slugify_title("!!!"), "post");
        assert_eq!(slugify_title("—"), "post");
        assert_eq!(slugify_title("🚀🎉"), "post");
        assert_eq!(slugify_title("   "), "post");
    }

    #[test]
    fn slugify_title_truncates_to_cap_on_char_boundary() {
        let long = "あ".repeat(200); // 200 CJK chars
        let s = slugify_title(&long);
        assert_eq!(s.chars().count(), MAX_SLUG_CHARS);
        assert!(s.parse::<Slug>().is_ok()); // truncated result is still valid
    }
```

- [ ] **Step 2: Run — verify FAIL.**

Run: `cargo test -p common slugify`
Expected: FAIL (current returns `Option<String>`, ASCII-only; type mismatch + wrong values).

- [ ] **Step 3: Implement.** Replace `slugify_title` in `common/src/slug.rs`:

```rust
/// Converts a title to a slug: NFC-normalized, Unicode-lowercased, keeping only
/// `char::is_alphanumeric()` characters and collapsing other runs into single
/// hyphens. Truncated to `MAX_SLUG_CHARS`. Never fails: when nothing usable
/// remains (emoji/symbol-only, untitled), returns the bare fallback `"post"`,
/// and the caller's per-author-per-day collision retry disambiguates.
#[must_use]
pub fn slugify_title(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_dash = false;
    for ch in title.to_lowercase().nfc() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            previous_was_dash = false;
        } else if !slug.is_empty() && !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.chars().count() > MAX_SLUG_CHARS {
        slug = slug.chars().take(MAX_SLUG_CHARS).collect();
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    if slug.is_empty() {
        "post".to_owned()
    } else {
        slug
    }
}
```

- [ ] **Step 4: Run — verify PASS.**

Run: `cargo test -p common slug`
Expected: PASS (all of Task 1 + Task 2 tests).

- [ ] **Step 5: Commit.**

```bash
git add common/src/slug.rs
git commit -m "feat(slug): infallible Unicode slugify_title with post fallback + length cap"
```

---

### Task 3: Consume the infallible generator — derivation, cap-aware collisions, retire `NoSlugFromPost`

This is one atomic commit: removing the `NoSlugFromPost` variant breaks its match arms, so the enum + all arms + all asserting tests change together to keep every commit compiling.

**Files:**
- Modify: `storage/src/post_service.rs` (derivation 206-215 & 327-340; `candidate_slug` 258-265; `PerformUpdateError` 94-107; `PerformCreationError` 242-255; tests 527-548, 817-844, 864-868)
- Modify: `web/src/posts/server.rs` (NoSlugFromPost arms ~230, ~242; status tests ~270, ~301)
- Modify: `server/src/atompub/mod.rs` (NoSlugFromPost arms ~204, ~218; status tests ~386, ~410)

**Interfaces:**
- Consumes: `slugify_title(&str) -> String` (Task 2), `MAX_SLUG_CHARS` (Task 1).
- Produces: derivation that never yields `NoSlugFromPost`; `candidate_slug` output always ≤ 80 chars.

- [ ] **Step 1: Make `candidate_slug` cap-aware (failing test first).** Add to `post_service.rs` tests:

```rust
    #[test]
    fn candidate_slug_keeps_suffix_within_cap() {
        let seed: String = "a".repeat(common::slug::MAX_SLUG_CHARS); // 80
        let c = candidate_slug(&seed, 1); // would be 82 chars naively
        assert!(c.chars().count() <= common::slug::MAX_SLUG_CHARS);
        assert!(c.ends_with("-2"));
        assert!(c.parse::<common::slug::Slug>().is_ok());
    }
```

Run: `cargo test -p storage candidate_slug` → FAIL (current appends unconditionally → 82 chars → would fail `from_str`).

- [ ] **Step 2: Implement cap-aware `candidate_slug`** (`post_service.rs:258-265`):

```rust
#[must_use]
pub fn candidate_slug(slug_seed: &str, attempt: usize) -> String {
    if attempt == 0 {
        return slug_seed.to_owned(); // already ≤ MAX_SLUG_CHARS from slugify_title
    }
    let suffix = format!("-{}", attempt + 1);
    let max_base = common::slug::MAX_SLUG_CHARS.saturating_sub(suffix.chars().count());
    let mut base: String = slug_seed.chars().take(max_base).collect();
    while base.ends_with('-') {
        base.pop();
    }
    format!("{base}{suffix}")
}
```

- [ ] **Step 3: Simplify the derivation** to drop `NoSlugFromPost`. Update path (`post_service.rs:206-215`):

```rust
        let slug = match slug_override.and_then(common::text::non_empty) {
            Some(raw) => raw
                .parse::<Slug>()
                .map_err(|_| PerformUpdateError::InvalidSlug)?,
            None => slugify_title(&metadata.slug_seed)
                .parse::<Slug>()
                .map_err(|_| PerformUpdateError::InvalidSlug)?,
        };
```

Creation path (`post_service.rs:327-340`):

```rust
        let slug_seed = match slug_override.and_then(common::text::non_empty) {
            Some(raw) => raw
                .parse::<Slug>()
                .map_err(PerformCreationError::InvalidSlug)?
                .to_string(),
            None => slugify_title(&metadata.slug_seed),
        };
```

(Both drop the `.to_ascii_lowercase()` pre-pass — `from_str` now lowercases — and the `.ok_or(NoSlugFromPost)`.)

- [ ] **Step 4: Remove the `NoSlugFromPost` variant** from `PerformUpdateError` (line ~98) and `PerformCreationError` (line ~246) in `post_service.rs`.

- [ ] **Step 5: Build to surface every break, then fix each.**

Run: `cargo build -p storage -p jaunder` (or `cargo xtask check --no-test`)
Expected: errors at the former match arms — `web/src/posts/server.rs` (~230 `PerformUpdateError::NoSlugFromPost`, ~242 `PerformCreationError::NoSlugFromPost`) and `server/src/atompub/mod.rs` (~204, ~218). **Delete each `NoSlugFromPost` arm** (the surrounding `InvalidSlug`/other arms stay).

- [ ] **Step 6: Update the asserting tests** to the never-fail behavior:
  - `post_service.rs::test_perform_post_creation_no_slug_from_body` (527-548): body `"!!!"` no longer errs. Rewrite to assert success with the fallback slug:

```rust
    #[tokio::test]
    async fn test_perform_post_creation_symbol_only_title_falls_back_to_post() {
        let (_pool, storage) = setup_test_db().await;
        let post_id = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "!!!".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();
        let record = storage.get_post(post_id).await.unwrap();
        assert_eq!(record.slug.as_str(), "post");
    }
```

(If `get_post(post_id)` isn't the exact accessor, use the same fetch the neighboring success tests use — mirror `test_perform_post_creation_slug_conflict_retries` at 550+.)

  - Add a Unicode success test alongside it:

```rust
    #[tokio::test]
    async fn test_perform_post_creation_unicode_title_preserves_slug() {
        let (_pool, storage) = setup_test_db().await;
        let post_id = perform_post_creation(
            &storage,
            PostCreation {
                user_id: 1,
                body: "# 日本語\n\nbody".to_owned(),
                title: None,
                format: PostFormat::Markdown,
                slug_override: None,
                published_at: None,
                max_attempts: 100,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .unwrap();
        let record = storage.get_post(post_id).await.unwrap();
        assert_eq!(record.slug.as_str(), "日本語");
    }
```

  - `test_perform_creation_error_display_and_debug` (817-844): delete the `NoSlugFromPost` block (824-828).
  - `perform_update_error_no_slug_from_title_display` (864-868): delete this test.
  - `web/src/posts/server.rs` status tests (~270, ~301) and `server/src/atompub/mod.rs` (~386, ~410): these fed a slug-less body and asserted a 4xx. Either delete the now-impossible case or repoint it at a still-failing input (e.g. an explicit invalid `slug_override` → `InvalidSlug` → same status). Pick whichever keeps the test meaningful; do not assert a removed variant.

- [ ] **Step 7: Run the gate.**

Run: `nix develop .#ci -c cargo xtask check --no-test`
Expected: PASS (compiles; all updated tests green).

- [ ] **Step 8: Commit.**

```bash
git add storage/src/post_service.rs web/src/posts/server.rs server/src/atompub/mod.rs
git commit -m "refactor(slug): consume infallible slugify_title; cap-aware collisions; retire NoSlugFromPost"
```

---

### Task 4: e2e — Unicode permalink round-trip (both backends)

The full-stack acceptance test: it runs in both the `e2e-sqlite` and `e2e-postgres` Nix VMs, giving backend-parity coverage for create→publish→lookup of a Unicode slug.

**Files:**
- Create: `end2end/tests/unicode-slug.spec.ts`

**Interfaces:**
- Consumes: the running app with Tasks 1-3 landed; helpers `register`, `goto`, `click`, `waitForSelector` (`end2end/tests/helpers.ts`); `test`/`expect`/timeout scalers (`end2end/tests/fixtures.ts`).

- [ ] **Step 1: Write the spec.** Mirror `end2end/tests/posts.spec.ts` (title is the `# heading` in `textarea[name="body"]`; publish via `button[name="publish"][value="true"]`; read the generated href from `[data-test="permalink-link"]`, never construct it; the browser percent-encodes the path).

```ts
import { test, expect, hydrationHeavyFirstNavigationTimeoutMs } from "./fixtures";
import { goto, click, waitForSelector, register } from "./helpers";

test("a Unicode-titled post is reachable at its permalink", async ({ page }, testInfo) => {
  test.slow();
  await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000));

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Café 日本語\n\nunicode body");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");
  await expect(page.locator(".j-save-summary")).toContainText("Post published.");

  // The app generated the slug; read it rather than constructing it.
  const slug = await page
    .locator('.j-save-summary [data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slug).toBe("café-日本語");

  const href = await page
    .locator('.j-save-summary [data-test="permalink-link"]')
    .getAttribute("href");
  expect(href).toBeTruthy();

  await goto(page, href!); // browser percent-encodes the Unicode path segment
  await expect(page.locator("article h1")).toContainText("Café 日本語");
  await expect(page.locator(".j-post-body")).toContainText("unicode body");
});

test("an emoji-only title falls back to the 'post' slug and is reachable", async ({ page }, testInfo) => {
  test.slow();
  await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000));

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# 🚀🎉\n\nemoji body");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".j-save-summary");
  await expect(page.locator(".j-save-summary")).toContainText("Post published.");

  const slug = await page
    .locator('.j-save-summary [data-test="slug-value"]')
    .getAttribute("data-slug");
  expect(slug).toBe("post");

  const href = await page
    .locator('.j-save-summary [data-test="permalink-link"]')
    .getAttribute("href");
  await goto(page, href!);
  await expect(page.locator(".j-post-body")).toContainText("emoji body");
});
```

- [ ] **Step 2: Verify the `[data-test]` hooks exist.** Confirm `data-test="slug-value"` (with `data-slug`) and `data-test="permalink-link"` are emitted by the save summary (`web/src/pages/posts.rs`; referenced in `posts.spec.ts:128-145`). If the emoji `# 🚀🎉` heading doesn't render a visible `article h1`, assert only on `.j-post-body` for that case (already done above).

- [ ] **Step 3: Run the e2e locally (chromium).**

Run: `bash scripts/e2e-local.sh` (or `cargo leptos end-to-end`) — executes `playwright test --project chromium --workers=1`.
Expected: both new tests pass. (If the slug assertion `café-日本語` differs from what the generator produces for the exact heading text, correct the expected value to the generator's output — the round-trip/reachability assertions are the real check.)

- [ ] **Step 4: Commit.**

```bash
git add end2end/tests/unicode-slug.spec.ts
git commit -m "test(e2e): Unicode-titled and emoji-only posts resolve at their permalinks"
```

---

### Task 5: Final full-gate verification

**Files:** none (verification only).

- [ ] **Step 1: Full validate (incl. e2e on SQLite + Postgres).**

Run: `nix develop .#ci -c cargo xtask validate`
Expected: exit 0; `coverage` clean; `e2e-sqlite` + `e2e-postgres` green (the new spec runs in both).

- [ ] **Step 2: Review the branch diff vs the fork point.**

Run: `git diff wt-base-issue-72..HEAD --stat`
Expected: only `common/{Cargo.toml,src/slug.rs}`, `Cargo.lock`, `storage/src/post_service.rs`, `web/src/posts/server.rs`, `server/src/atompub/mod.rs`, `end2end/tests/unicode-slug.spec.ts`, and the planning docs. No stray files; `main` untouched.

---

## Self-Review

**Spec coverage:** widen/normalize/cap `from_str` (Task 1) · infallible Unicode `slugify_title` + `post` fallback + truncation (Task 2) · cap-aware collisions (Task 3) · retire `NoSlugFromPost` + mappings + tests (Task 3) · `unicode-normalization` dep (Task 1) · NFC NFD-matches-NFC (Task 1 test) · 80/81 boundary (Task 1) · symbol/emoji→`post` (Task 2 + Task 4) · inbound percent-decode round-trip + backend parity (Task 4 e2e on both VMs) · collision suffix on Unicode base (Task 3 `candidate_slug` test + existing retry test). All spec sections map to a task.

**Placeholder scan:** the only soft spots are deliberately compiler-guided (Task 3 Step 5 arm removal, with exact file:line) and the e2e expected-slug value (Task 4 Step 3 note) — both have a concrete fallback instruction. No TBD/TODO.

**Type consistency:** `slugify_title(&str) -> String` (Task 2) is consumed in Task 3's derivation (no `.ok_or`); `MAX_SLUG_CHARS` (Task 1) used in Task 2 + Task 3 `candidate_slug`; `candidate_slug(&str, usize) -> String` unchanged signature; `Slug::from_str`/`as_str` names consistent across tasks and e2e assertions.
