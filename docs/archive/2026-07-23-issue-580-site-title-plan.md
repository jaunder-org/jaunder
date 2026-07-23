# Plan — #580: `SiteTitle` newtype for the site identity title

Spec:
[2026-07-23-issue-580-site-title.md](../specs/2026-07-23-issue-580-site-title.md)
· Issue [#580](https://github.com/jaunder-org/jaunder/issues/580)

## Review header

**Goal.** Add a `SiteTitle` `StrNewtype` (trim + non-empty, `Default` =
`DEFAULT_SITE_TITLE`) and thread it through `SiteIdentity.title` + the
`update_site_identity` wire arg, deleting the in-body check and adding ADR-0065
client validation on the site-settings title field.

**Scope.** In: `SiteTitle` in `common::site` (+ tests); the atomic swap of
`SiteIdentity.title` → handler / form / storage boundary / feed constructors;
the test sweep + `parse_site_title` fixture. Out: length cap; `base_url`
(typed); the feed `compute_title(&str)` helper.

**Tasks.**

1. `SiteTitle` in `common::site` (validating `StrNewtype`, `Default`, error,
   tests).
2. Atomic swap: `SiteIdentity.title: SiteTitle`; handler + form + storage +
   feed + test sweep
   - `parse_site_title` fixture.

**Key risks / decisions.**

- Task 2 is **atomic** (the field type swap breaks the handler, form, storage,
  feed, and tests together).
- `SiteTitle::default()` = `DEFAULT_SITE_TITLE` is the infallible fallback door
  `get_identity` needs (storage is `expect_used = deny`).
- The form title is REQUIRED + programmatically dispatched →
  `Field<SiteTitle>` + `if let Some(title) = title_field.parsed()` +
  disabled-until-valid.
- `update_site_identity_rejects_empty_title` (web_site.rs) must be rewritten
  from specific-message/`500` to `assert_ne!(status, OK)` (decode-time rejection
  now).

**For agentic workers:** **jaunder-iterate**; Task 2 is a candidate for
**jaunder-dispatch**.

## Global constraints

- No `Co-Authored-By`. `cargo xtask check` clean before commit (hook enforces).
  Task 2 closes with `cargo xtask validate --no-e2e`.
- Follow `CONTRIBUTING.md` (ADR-0065 client-validation pattern; `parse_<name>`
  test fixtures).

---

## Task 1 — `SiteTitle` in `common::site`

**Files** — `common/src/site.rs`: add (after
`SiteIdentity`/`DEFAULT_SITE_TITLE`), importing `macros::StrNewtype`,
`std::str::FromStr`, `thiserror::Error`:

```rust
/// The site's human-facing title — trimmed, non-empty. The validating counterpart of the
/// infallible `PostTitle`; modelled on `DisplayName`. `Default` is the app's `DEFAULT_SITE_TITLE`.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SiteTitle(String);

#[derive(Debug, Error)]
#[error("site title cannot be empty")]
pub struct InvalidSiteTitle;

impl FromStr for SiteTitle {
    type Err = InvalidSiteTitle;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.is_empty() { return Err(InvalidSiteTitle); }
        Ok(SiteTitle(t.to_owned()))
    }
}

impl Default for SiteTitle {
    fn default() -> Self { Self(DEFAULT_SITE_TITLE.to_owned()) }
}
```

Tests (in-file, driving every generated branch): `FromStr` accept (trims,
preserves inner space/casing), reject empty/whitespace-only (domain message),
serde serialize-as-string + deserialize **accept** round-trip **and**
deserialize-rejects-`""`, `Display`/`PartialEq<&str>`, and
`SiteTitle::default() == DEFAULT_SITE_TITLE` (mirrors the `DisplayName` test
set).

**Run:** `cargo nextest run -p common site`; `cargo xtask check`.

**Commit:** `feat(common): SiteTitle StrNewtype in common::site (#580)`

---

## Task 2 — thread `SiteTitle` end-to-end (atomic) + form + test sweep

One cohesive commit.

**`common/src/site.rs`:** `SiteIdentity.title: SiteTitle` (was `String`).

**`common/src/test_support.rs`:** add `parse_site_title(s: &str) -> SiteTitle`
(via `FromStr`, `parse_<name>` convention) + a **fresh**
`use crate::site::SiteTitle;` (the file has no existing `common::site` import).
(Used by the sweep below — coverage-measured.)

**`web/src/site/mod.rs`:**
`update_site_identity(title: SiteTitle, base_url: Option<AbsoluteUrl>)`; delete
the `let title = title.trim()…; if title.is_empty() {…}` block; build
`SiteIdentity { title, base_url }` directly. Import `SiteTitle` (ungated — it's
a wire arg type). Keep `#[tracing::instrument(skip(title, base_url))]`.

**`web/src/pages/site.rs`:** replace the plain `<input>` + `RwSignal<String>`
title buffer:

- `let title_field = Field::<SiteTitle>::prefilled(&identity.title);`
  (`&SiteTitle` deref-coerces to the `&str` param — no `.as_ref()`).
- swap the `<input name="title" …/>` for
  `<ValidatedInput<SiteTitle> label="Site Title" name="title" field=title_field class="j-site-input" field_class="j-site-field j-site-field-wide" />`
  (match the neighboring `AbsoluteUrl` ValidatedInput's props/formatting).
- submit:
  `if let Some(title) = title_field.parsed() { update_action.dispatch(UpdateSiteIdentity { title, base_url: base_url_field.parsed() }); }`.
- button
  `prop:disabled=move || !title_field.is_valid() || !base_url_field.is_valid()`.
- Drop the now-unused `RwSignal`/`event_target_value` title plumbing.

**`storage/src/site_config.rs`:**

- `get_identity`: replace
  `.and_then(common::text::non_empty_owned).unwrap_or_else(|| DEFAULT_SITE_TITLE.to_owned())`
  with `.and_then(|v| v.parse::<SiteTitle>().ok()).unwrap_or_default()`. Import
  `SiteTitle`; drop the now-unused `common::text::non_empty_owned` /
  `DEFAULT_SITE_TITLE` imports if unused.
- `set_identity`: **unchanged** — `self.set(SITE_TITLE_KEY, &config.title)`;
  `&SiteTitle` deref-coerces to the `&str` param exactly as `&String` did. (No
  `.as_ref()`.)
- The one and only conversion is the `get_identity` parse above — the
  untyped-store→typed-value boundary, identical to every config newtype. No
  other `.as_ref()`/`.parse()`/`.to_string()` is introduced anywhere (rely on
  `Deref<str>` coercion).
- Tests — two distinct groups:
  - **Identity tests** (`get_identity`/`set_identity`/round-trip): keep seeding
    the real `SITE_TITLE_KEY`; `SiteIdentity { title: "…".to_string() }`
    fixtures → `parse_site_title("…")`;
    `assert_eq!(identity.title, DEFAULT_SITE_TITLE)` unchanged (generated
    `PartialEq<&str>`); the "empty title treated as unset" test unchanged (empty
    → `default()`).
  - **Generic store-mechanics tests** (`get`/`set`/`delete`/`from_pairs`
    round-trips that use `"site.title"` merely as a _sample key_): switch the
    sample key to a neutral one (e.g. `"example.setting"`) so nobody mistakes
    the raw `String` store for the typed title path.
    (`storage/src/site_config.rs` — the `from_pairs([("site.title", "T"), …])`
    seed and the `store.set/get/delete("site.title", …)` mechanics tests.)

**`server/src/feed/regenerate.rs`:** `compute_title(&identity.title, …)` is
**unchanged** — the helper stays `&str` and `&SiteTitle` deref-coerces. Only the
test fixtures `SiteIdentity { title: "Jaunder".to_owned() }` →
`parse_site_title("Jaunder")` change.

**`server/src/feed/worker.rs`:** the three test-fixture
`SiteIdentity { title: "Jaunder".to_owned() }` → `parse_site_title("Jaunder")`.

**`server/tests/atompub/atompub_rsd.rs`:**
`SiteIdentity { title: "Test".to_string() }` → `parse_site_title("Test")`.

**`server/tests/web/web_site.rs`:** rewrite
`update_site_identity_rejects_empty_title` — drop the
`body.contains("site title cannot be empty")` + `INTERNAL_SERVER_ERROR`
assertions; assert `assert_ne!(status, StatusCode::OK)` (decode-time rejection),
matching the sibling `base_url` decode tests. Any `SiteIdentity` fixtures /
`.title` string asserts updated (reads survive via `PartialEq<&str>`).

**Run / final gate:** `cargo check` across common/storage/server/web (+
`--features server`); `cargo nextest run -p storage site_config`;
`cargo xtask validate --no-e2e` green. Confirm the in-body check is gone
(`rg -n "site title cannot be empty" web/src` → nothing) and
`SiteIdentity.title` is `SiteTitle` everywhere.

**Commit:**
`refactor(common,web,storage,server): type SiteIdentity.title as SiteTitle (#580)`

## Self-review

- Task 1 self-contained (`SiteTitle` + tests; `SiteIdentity.title` still
  `String`); Task 2 is the atomic swap. No partial state committed.
- Every acceptance criterion maps: `SiteTitle` + trailer + `Default` → Task 1;
  typed field/arg
  - deleted check + client validation → Task 2.
