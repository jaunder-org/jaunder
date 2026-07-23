# Spec — #580: `SiteTitle` newtype for the site identity title

- Issue: [#580](https://github.com/jaunder-org/jaunder/issues/580)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md),
  [ADR-0065](../../adr/0065-typed-wire-args.md) (typed wire args + client
  validation)
- Related: [#402](https://github.com/jaunder-org/jaunder/issues/402)
  (`PostTitle`), [#448](https://github.com/jaunder-org/jaunder/issues/448)
  (`AbsoluteUrl` — the sibling arg),
  [#414](https://github.com/jaunder-org/jaunder/issues/414) / ADR-0065
- Date: 2026-07-23

## Problem

`update_site_identity(title: String, base_url: Option<AbsoluteUrl>)`
(`web/src/site/mod.rs`) validates the title in-body (`trim` + non-empty) while
its sibling arg is already typed (`AbsoluteUrl`, #448). The backing
`common::site::SiteIdentity.title` field is a bare `String`, so the non-empty
invariant lives only in the one handler.

## Decision

A `SiteTitle` `StrNewtype` — **trimmed, non-empty** (the trimming of `PostTitle`
plus a non-empty invariant, so a _validating_ `FromStr` like `DisplayName`, not
`PostTitle`'s `infallible`) — used as the wire arg and the `SiteIdentity.title`
field, with ADR-0065 client-side validation on the form field.

### The newtype — `common::site::SiteTitle`

Placed in `common::site` (co-located with `SiteIdentity` and
`DEFAULT_SITE_TITLE`, which it references), modelled on `DisplayName`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct SiteTitle(String);

#[derive(Debug, thiserror::Error)]
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
    /// The app default (`DEFAULT_SITE_TITLE`) — the infallible construction door
    /// `get_identity` needs for its absent/empty/invalid-config fallback.
    fn default() -> Self { Self(DEFAULT_SITE_TITLE.to_owned()) }
}
```

- **Trim + non-empty, no length cap** (per the issue; `PostTitle` also has no
  bound). The `StrNewtype` trailer (`Display`, `AsRef`/`Borrow`/`Deref<str>`,
  owned-`String`, the serde bridge, `PartialEq<str>`/`<&str>`) is generated; the
  serde `Deserialize` routes through `FromStr`, so a `SiteTitle` wire arg is
  validated (trimmed, rejects empty) at decode.
- **`Default` is load-bearing**, not decoration: `get_identity` falls back to
  `DEFAULT_SITE_TITLE` when the config value is absent/empty/unparseable, and
  storage code is `expect_used = deny`, so it needs an _infallible_ default
  door. `SiteTitle::default()` is that door (the app's canonical default title).
  `DEFAULT_SITE_TITLE: &str` stays the single source of the string.

### `common::site` — the field

`SiteIdentity.title: SiteTitle` (was `String`). `SiteIdentity` stays
`Serialize`/ `Deserialize` (wire type: `get_site_identity` returns it);
`SiteTitle`'s serde bridge keeps it a plain string on the wire.

### `web` — typed wire arg, in-body check deleted

- `web/src/site/mod.rs`:
  `update_site_identity(title: SiteTitle, base_url: Option<AbsoluteUrl>)`;
  **delete** the
  `let title = title.trim()…; if title.is_empty() { return Err(validation) }`
  block — the serde bridge now rejects an empty/whitespace title at decode
  (ADR-0065-aligned, earlier rejection). Build
  `SiteIdentity { title, base_url }` directly. Import `SiteTitle`.
- `web/src/pages/site.rs` (the site-settings form): replace the plain
  `<input>` + `RwSignal<String>` title buffer with
  `let title_field = Field::<SiteTitle>::prefilled(identity.title.as_ref());`
  and a
  `<ValidatedInput<SiteTitle> label="Site Title" name="title" field=title_field … />`.
  The dispatch guards on validity (title is required):
  `if let Some(title) = title_field.parsed() { update_action.dispatch(UpdateSiteIdentity { title, base_url: base_url_field.parsed() }); }`,
  and the save button's `prop:disabled` gains `|| !title_field.is_valid()`.
  (`Field<T>` requires `T: FromStr, Err: Display` — `SiteTitle` satisfies both.)

### `storage` — parse at the boundary

`storage/src/site_config.rs`:

- `get_identity`: **replaces** today's
  `.and_then(common::text::non_empty_owned).unwrap_or_else(|| DEFAULT_SITE_TITLE.to_owned())`
  with `.and_then(|v| v.parse::<SiteTitle>().ok()).unwrap_or_default()`.
  Behavior is exact: `non_empty_owned` trims-then-nulls-if-empty, and
  `SiteTitle::from_str` trims-then-rejects-empty — so a whitespace-only config
  value falls through to `default()` (`DEFAULT_SITE_TITLE`) identically, and a
  padded title trims the same way.
- `set_identity`: **unchanged** — `self.set(SITE_TITLE_KEY, &config.title)`;
  `&SiteTitle` deref-coerces to the `&str` param. The `get_identity` parse is
  the sole conversion; everywhere else relies on `Deref<str>` coercion (no
  gratuitous `.as_ref()`/`.to_string()`).
- **Test hygiene:** the generic config-store `get`/`set`/`delete`/`from_pairs`
  mechanics tests currently use `"site.title"` as an arbitrary sample key —
  switch them to a neutral key (e.g. `"example.setting"`) so the raw `String`
  store isn't mistaken for the typed title path. The identity tests that
  legitimately seed `SITE_TITLE_KEY` keep it.

### `server/feed` — construction sites (compiler-forced)

- `server/src/feed/regenerate.rs`: `compute_title(site_title: &str, …)` stays
  `&str`; the caller passes `identity.title.as_ref()`.
- The `SiteIdentity { title: "Jaunder".to_owned(), … }` sites in
  `feed/worker.rs` and `feed/regenerate.rs` are **test fixtures** — wrap via
  `parse_site_title("Jaunder")`.
- `server/tests/atompub/atompub_rsd.rs` also constructs
  `SiteIdentity { title: "Test"…, … }` (compiler-forced once the field is typed)
  → `parse_site_title("Test")`.

## Tests

- `common::site`: `SiteTitle` `FromStr` accept (trims, preserves casing/inner
  space), reject empty/whitespace-only (with the domain message), serde
  serialize-as-string + deserialize- validates (reject `""`),
  `Display`/`PartialEq<&str>`, and `SiteTitle::default() == DEFAULT_SITE_TITLE`.
- `common::test_support`: a `parse_site_title(&str) -> SiteTitle` fixture (the
  `parse_<name>` convention), used by the storage/feed/web test sweep.
- `storage`: the existing `get_identity`/`set_identity`/round-trip tests are
  behavior- preserving; `SiteIdentity { title: "…".to_string() }` fixtures →
  `parse_site_title("…")`, and `assert_eq!(identity.title, DEFAULT_SITE_TITLE)`
  asserts keep working via the generated `PartialEq<&str>`. The "empty title
  treated as unset" test still passes (empty → `default()`).
- `web`: the site-settings form/handler tests updated for the typed arg. **One
  observable change:**
  `server/tests/web/web_site.rs::update_site_identity_rejects_empty_title`
  currently asserts `status == INTERNAL_SERVER_ERROR` **and**
  `body.contains("site title cannot be empty")` — with the in-body check
  deleted, an empty title is rejected at serde-decode of the `SiteTitle` arg,
  yielding a _generic_ non-OK server-fn error (no specific message/status),
  exactly like the sibling `base_url` decode tests in the same file. Rewrite it
  to the `assert_ne!(status, StatusCode::OK)` shape those use (assert rejection,
  not the specific message). The title is still rejected — only the wire-error
  surface changes.

## Out of scope

- No length cap (the issue asks only non-empty).
- `base_url` (already typed, #448); `DEFAULT_SITE_TITLE` stays a `&str` const.
- The feed `compute_title` string-formatting helper stays `&str` (deliberate
  flattening).

## Acceptance

- `SiteTitle` in `common::site` with the ADR-0063 str trailer (trim +
  non-empty), a named error, and a `Default` = `DEFAULT_SITE_TITLE`;
  `SiteIdentity.title` and the `update_site_identity` wire arg typed; the
  in-body trim/non-empty check deleted.
- Client validation per ADR-0065 on the site-settings title field
  (`ValidatedInput<SiteTitle>`).
- `cargo xtask validate --no-e2e` clean.
