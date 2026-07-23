# FormatToggle + PostFormat→strum Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate `PostFormat` off the bespoke `StrEnum` derive to `strum` (the
first pass of `StrEnum` retirement, #607), give it a reusable `sqlx` bridge so
it stores as a typed value, and extract a shared `FormatToggle` Leptos component
that dedupes the four `.j-seg` toggle blocks.

**Architecture:** `PostFormat` gains `strum::VariantArray` (enumeration) +
`EnumMessage` (editor labels) + `EnumString`/`parse_err_ty` (a `thiserror`
`InvalidPostFormat`), serde via a `String` proxy (`into`/`try_from`). A reusable
`impl_text_column_enum!` `macro_rules!` in `common` supplies the dual-backend
`Type`/`Encode`/`Decode` bridge. The toggle iterates
`PostFormat::VARIANTS.filter_map(get_message)`.

**Tech Stack:** Rust, `strum` 0.28, `serde`/`serde_qs` 0.15, `sqlx`
(dual-backend sqlite+postgres), `thiserror`, Leptos.

**Spec:**
`docs/superpowers/specs/2026-07-22-issue-572-format-toggle-component.md` — the
"what/why". This plan is the "how"; sections are referenced, not restated.

## Review header (for the approver)

- **Goal:** dedupe the `.j-seg` toggle via a shared component, by first making
  `PostFormat` a strum enum that offers variant-enumeration + labels, stored via
  a reusable typed `sqlx` bridge.
- **Scope IN:** `PostFormat` (common) strum migration; reusable
  `impl_text_column_enum!` bridge; typed `.bind`/decode in storage (posts write
  ×3 sites + the `PostRecordParts` read tuple); `FormatToggle` component + the 4
  call sites; ADR draft (already written) promoted at ship.
- **Scope OUT (→ #607):** migrating the remaining `StrEnum` enums (visibility
  ×4, media, registration); deleting the `StrEnum` macro. No wire-token /
  DB-representation / variant changes.
- **Tasks:**
  1. Migrate `PostFormat` → strum (common only; storage keeps compiling via
     `Display`).
  2. Reusable `impl_text_column_enum!` bridge + typed storage bind/decode
     (round-trip covers it).
  3. `FormatToggle` component + replace the four `.j-seg` blocks (e2e covers
     it).
- **Key risks/decisions:** `parse_err_ty` / `EnumMessage` / the bridge have no
  in-repo precedent → Task 1 & 2 are the compile-and-test spike; serde uses a
  `String` proxy (`into`/`try_from`) — a derived enum decodes via `serde_qs` but
  yields serde's generic "unknown variant" message, and `web_posts.rs` asserts
  the domain `post format must be …` message, so deserialize routes through
  `FromStr`; Task 2's read-tuple flip retires `helpers.rs`'s manual `format`
  parse and its `rejects_invalid_format` unit test (the rejection moves to
  column-decode).

## Global Constraints

- **Wire/DB tokens are frozen:** `markdown` / `org` / `html`, from strum
  `serialize_all = "snake_case"` (serde routes through `as_str` via the `String`
  proxy, so the token is single-sourced). No data migration (stored bytes
  identical).
- **`InvalidPostFormat` is load-bearing:** keep the exact name, `common::render`
  path, `Display` message `post format must be "markdown", "org", or "html"`,
  and `impl std::error::Error`. `host/src/error.rs:386,670` and
  `storage/src/posts.rs:23` depend on it.
- **`StrEnum` stays in the tree** — only `PostFormat` leaves it this issue.
- **Backend parity (ADR-0053):** storage tests are dual-backend; a bare
  `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern`
  guard. Follow the existing `storage/src/posts.rs` test harness.
- **Coverage policy:** the `thiserror` error path and the bridge
  `Encode`/`Decode` must be exercised by tests (don't add uncovered code).
- **Commits:** run `cargo xtask check` before each commit (jaunder-commit);
  conventional-commit messages; **no `Co-Authored-By` trailer**.

---

### Task 1: Migrate `PostFormat` to strum (common)

**Files:**

- Modify: `common/src/render.rs:14-32` (the `PostFormat` enum + its
  `InvalidPostFormat`), and imports at the top of the file.
- Test: `common/src/render.rs` `#[cfg(test)]` module (in-file, the crate
  convention).

**Interfaces:**

- Consumes: `strum` (already `common/Cargo.toml:19`), `thiserror` (`:20`),
  `serde`.
- Produces (public API — dependents rely on these unchanged unless noted):
  - `PostFormat` — `Copy` enum, variants `Markdown` (default) / `Org` / `Html`.
  - `impl Display for PostFormat` → the wire token (`"markdown"`).
  - `impl FromStr for PostFormat { type Err = InvalidPostFormat; }` +
    `TryFrom<&str>` (from `parse_err_ty`).
  - `impl Serialize + Deserialize for PostFormat` → the wire token.
  - `impl AsRef<str> for PostFormat` (strum `AsRefStr`) — consumed by Task 2's
    bridge.
  - `PostFormat::VARIANTS: &'static [PostFormat]` (strum `VariantArray`) —
    declaration order.
  - `PostFormat::get_message(&self) -> Option<&'static str>` (strum
    `EnumMessage`) — `Some("Markdown")`, `Some("Org")`, `None` for `Html`.
  - `pub struct InvalidPostFormat` (`thiserror`) — name/path/message preserved.

- [ ] **Step 1: Write the new failing tests** (added to the existing
      `#[cfg(test)]` mod; the five existing `PostFormat` tests at `:442-488` are
      left unchanged and must stay green)

```rust
#[test]
fn post_format_serde_json_round_trips() {
    assert_eq!(serde_json::to_string(&PostFormat::Markdown).unwrap(), "\"markdown\"");
    assert_eq!(serde_json::to_string(&PostFormat::Org).unwrap(), "\"org\"");
    assert_eq!(serde_json::to_string(&PostFormat::Html).unwrap(), "\"html\"");
    assert_eq!(serde_json::from_str::<PostFormat>("\"markdown\"").unwrap(), PostFormat::Markdown);
    assert_eq!(serde_json::from_str::<PostFormat>("\"html\"").unwrap(), PostFormat::Html);
    assert!(serde_json::from_str::<PostFormat>("\"bogus\"").is_err());
}

#[test]
fn post_format_variants_and_editor_labels() {
    use strum::{EnumMessage, VariantArray};
    assert_eq!(
        PostFormat::VARIANTS,
        &[PostFormat::Markdown, PostFormat::Org, PostFormat::Html]
    );
    assert_eq!(PostFormat::Markdown.get_message(), Some("Markdown"));
    assert_eq!(PostFormat::Org.get_message(), Some("Org"));
    assert_eq!(PostFormat::Html.get_message(), None); // renderer-internal → not offered
}
```

- [ ] **Step 2: Run the new tests, verify they fail**

Run:
`cargo nextest run -p common render::tests::post_format_variants_and_editor_labels render::tests::post_format_serde_json_round_trips`
Expected: FAIL — `VARIANTS` / `get_message` not found; (serde still works today,
but `VARIANTS`/`get_message` won't compile).

- [ ] **Step 3: Migrate the enum** to the contract in Interfaces. The tests
      above + the five existing tests pin every observable branch (Display
      token, FromStr Ok/Err, the error message, VARIANTS order, each label), so
      the body is the derive/attr list below — no hand-written logic beyond the
      `parse_err_fn` shim.

Replace `common/src/render.rs:14-32` with:

```rust
/// The format/markup language used to author a post body.
///
/// A `strum` string enum (ADR: `docs/adr/drafts/adopt-strum-retire-str-enum.md`):
/// `serialize_all = "snake_case"` gives the wire/DB token, `VariantArray` the
/// enumeration, `EnumMessage` the editor label (absent = not user-authored), and
/// `parse_err_ty` the named `InvalidPostFormat`. serde routes through an owned
/// `String` proxy (`into`/`try_from`) so an invalid token surfaces the domain
/// `InvalidPostFormat` message (see the impls below).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Default,
    serde::Serialize, serde::Deserialize,
    strum::VariantArray, strum::AsRefStr,
    strum::Display, strum::EnumString, strum::EnumMessage,
)]
#[serde(into = "String", try_from = "String")]
#[strum(serialize_all = "snake_case")]
#[strum(parse_err_ty = InvalidPostFormat, parse_err_fn = post_format_parse_err)]
pub enum PostFormat {
    /// CommonMark/GitHub-flavored Markdown.
    #[default]
    #[strum(message = "Markdown")]
    Markdown,
    /// Emacs Org-mode format.
    #[strum(message = "Org")]
    Org,
    /// Pre-rendered HTML. Renderer-internal provenance (#445); never user-authored,
    /// so it carries no editor `message` and is filtered out of format toggles.
    Html,
}

/// Error returned when a string matches no `PostFormat` variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

fn post_format_parse_err(_: &str) -> InvalidPostFormat {
    InvalidPostFormat
}

// serde `into`/`try_from` proxy: deserialize an owned String through FromStr so an
// invalid token surfaces the domain `InvalidPostFormat` message (asserted at the web
// boundary, server/tests/web/web_posts.rs `invalid_format` cases) rather than serde's
// generic "unknown variant". Single-sources the wire token in `as_str` (no rename_all).
impl From<PostFormat> for String {
    fn from(format: PostFormat) -> Self {
        format.as_ref().to_owned()
    }
}
impl TryFrom<String> for PostFormat {
    type Error = InvalidPostFormat;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
```

Then fix imports at the top of `render.rs`: remove `use macros::StrEnum;`
(PostFormat was its only user in this file — confirm with
`rg -n 'StrEnum' common/src/render.rs`); no new `use` is required (derives and
`thiserror::Error` are referenced by path). Leave `RenderedHtml` and everything
else untouched.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common render` Expected: PASS — all `PostFormat`
tests (5 existing + 2 new) green. The five existing tests are unchanged:
`Display`/`Debug`/`FromStr`/message are all preserved.

- [ ] **Step 5: Confirm dependents still compile** (the enum's public API is
      unchanged; `Display` still backs the storage `.to_string()` binds, so
      storage is untouched this task)

Run: `cargo xtask check --no-test` Expected: PASS (fmt + clippy + workspace
build). If clippy flags `must_use` on any strum-generated item, it is generated
code — not applicable here; investigate only a real warning.

- [ ] **Step 6: Commit**

```bash
git add common/src/render.rs
git commit -m "refactor(common): migrate PostFormat off StrEnum to strum (#572)"
```

Run `cargo xtask check` first (jaunder-commit); it must pass clean. No
`Co-Authored-By`.

---

### Task 2: Reusable `sqlx` bridge + typed `PostFormat` storage (common + storage)

**Files:**

- Create: `common/src/db_enum.rs` (the reusable `impl_text_column_enum!` macro).
- Modify: `common/src/lib.rs` (add `mod db_enum;`), `common/src/render.rs`
  (invoke the macro for `PostFormat`).
- Modify: `storage/src/posts.rs:1902-1915`, `storage/src/sqlite/posts.rs:95`,
  `storage/src/postgres/posts.rs:97` (typed write binds).
- Modify: `storage/src/helpers.rs:125-130` (comment), `:138` (tuple type),
  `:201-203` (delete parse), and the `#[cfg(test)]` `build_post_record` tests
  (`:498-647`).
- Test: `storage/src/posts.rs` `#[cfg(test)]` (dual-backend round-trip),
  `storage/src/helpers.rs` `#[cfg(test)]` (updated unit tests).

**Interfaces:**

- Consumes: `PostFormat: AsRef<str> + FromStr<Err = InvalidPostFormat>` (Task
  1); `InvalidPostFormat: std::error::Error + Send + Sync + 'static` (Task 1's
  `thiserror` unit struct satisfies this).
- Produces:
  - `impl_text_column_enum!($ty)` — a crate-internal `macro_rules!` emitting
    `sqlx::Type`/`Encode`/`Decode` for `$ty`, delegating to `String`/`&str`.
    Reusable by #607.
  - `impl sqlx::{Type, Encode, Decode} for PostFormat` (under
    `feature = "sqlx"`).
  - `PostRecordParts` element 7 becomes `PostFormat` (was `String`);
    `build_post_record` receives a typed `format`.

- [ ] **Step 1: Write the reusable macro** `common/src/db_enum.rs`

```rust
//! Reusable `sqlx` bridge for `strum` string enums stored as a TEXT token.
//!
//! `strum` has no `sqlx` integration and `sqlx`'s own `#[derive(Type)]` maps a plain
//! enum to a *native* DB enum type (not the TEXT token these are stored as, dual
//! backend). So this declarative macro lifts the `String`-delegating bridge shape
//! (`RenderedHtml`, `render.rs`) into one reusable definition: given a type that is
//! `AsRef<str>` (strum `AsRefStr`) + `FromStr` (strum `EnumString`), it binds/decodes
//! the token as a typed value — like the `StrNewtype` newtypes (#438) — instead of a
//! stringly `.as_str()`/`.to_string()` strip. Introduced with `PostFormat` (#572);
//! reused for the other stored enums in #607.

/// Emit `sqlx::Type`/`Encode`/`Decode` for a `strum` string enum stored as TEXT.
/// Requires `$ty: AsRef<str> + FromStr` where the `FromStr::Err` is
/// `std::error::Error + Send + Sync + 'static`.
macro_rules! impl_text_column_enum {
    ($ty:ty) => {
        #[cfg(feature = "sqlx")]
        const _: () = {
            impl<DB: sqlx::Database> sqlx::Type<DB> for $ty
            where
                String: sqlx::Type<DB>,
            {
                fn type_info() -> <DB as sqlx::Database>::TypeInfo {
                    <String as sqlx::Type<DB>>::type_info()
                }
                fn compatible(ty: &<DB as sqlx::Database>::TypeInfo) -> bool {
                    <String as sqlx::Type<DB>>::compatible(ty)
                }
            }

            impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for $ty
            where
                for<'a> &'a str: sqlx::Encode<'q, DB>,
            {
                fn encode_by_ref(
                    &self,
                    buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
                ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                    let s: &str = self.as_ref();
                    <&str as sqlx::Encode<'q, DB>>::encode_by_ref(&s, buf)
                }
            }

            impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for $ty
            where
                &'r str: sqlx::Decode<'r, DB>,
            {
                fn decode(
                    value: <DB as sqlx::Database>::ValueRef<'r>,
                ) -> Result<Self, sqlx::error::BoxDynError> {
                    let s = <&str as sqlx::Decode<'r, DB>>::decode(value)?;
                    Ok(s.parse()?)
                }
            }
        };
    };
}

pub(crate) use impl_text_column_enum;
```

Add `mod db_enum;` to `common/src/lib.rs` (near the other `mod` declarations).
In `common/src/render.rs`, below `InvalidPostFormat`, add:

```rust
crate::db_enum::impl_text_column_enum!(PostFormat);
```

- [ ] **Step 2: Write the failing tests** in `storage/src/posts.rs`
      `#[cfg(test)]`, mirroring the existing dual-backend tests
      `post_round_trips_slug_title_body_username_and_tag` (`:2962`) and
      `get_post_rejects_a_malformed_slug_column` (`:3036`) verbatim in structure
      (`#[apply(backends)]` + `#[case] backend`, whole-`TestEnv` bound per
      ADR-0053, `env.base.pool()` raw-SQL corruption,
      `sqlx::Error::ColumnDecode` assert). All helpers used (`seed_user`,
      `parse_slug`, `CreatePostInput`, `get_post_by_id`, `ViewerIdentity`,
      `AudienceTarget`, `CloseablePool`, `Backend`) are already in scope in this
      test module.

```rust
#[apply(backends)]
#[tokio::test]
async fn post_format_column_round_trips_all_variants(#[case] backend: Backend) {
    // Keep the whole `TestEnv` bound (ADR-0053 TempDir hazard).
    let env = backend.setup().await;
    let user_id = seed_user(&env.state).await;
    let posts = &*env.state.posts;

    // Org and Html exercise the bridge Encode (write) + Decode (read) for the
    // non-default variants; Markdown is covered by the existing round-trip tests.
    for (i, fmt) in [PostFormat::Org, PostFormat::Html].into_iter().enumerate() {
        let post_id = posts
            .create_post(&CreatePostInput {
                user_id,
                title: None,
                slug: parse_slug(&format!("fmt-{i}")),
                body: "body".into(),
                format: fmt,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .unwrap();
        let record = posts
            .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.format, fmt);
    }
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_a_malformed_format_column(#[case] backend: Backend) {
    let env = backend.setup().await;
    let user_id = seed_user(&env.state).await;
    let posts = &*env.state.posts;
    let post_id = posts
        .create_post(&CreatePostInput {
            user_id,
            title: None,
            slug: parse_slug("good"),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .unwrap();

    // Land a bogus token in `format` via a raw bind (the typed bind could not
    // produce it), then assert the read fails at column-decode — the bridge's
    // Decode error arm (`parse()` → InvalidPostFormat). This replaces the deleted
    // `build_post_record` rejects_invalid_format unit test (rejection moved to decode).
    let sql = "UPDATE posts SET format = $1 WHERE post_id = $2";
    match env.base.pool() {
        CloseablePool::Sqlite(pool) => {
            sqlx::query(sql).bind("bogus").bind(i64::from(post_id)).execute(pool).await.unwrap();
        }
        CloseablePool::Postgres(pool) => {
            sqlx::query(sql).bind("bogus").bind(i64::from(post_id)).execute(pool).await.unwrap();
        }
    }
    let err = posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap_err();
    assert!(
        matches!(err, sqlx::Error::ColumnDecode { .. }),
        "expected a column-decode error, got: {err:?}"
    );
}
```

- [ ] **Step 3: Run the new tests, verify they fail**

Run:
`cargo nextest run -p storage post_format_column_round_trips_all_variants get_post_rejects_a_malformed_format_column`
Expected: FAIL — before the bridge, `record.format` decodes from a `String`
tuple slot / the typed `Decode` isn't wired, so it won't compile or won't
reject.

- [ ] **Step 4: Migrate the storage write + read to typed `PostFormat`**

Write binds (drop the stringly conversions — the bridge binds the value
directly):

- `storage/src/posts.rs:1902` remove `let format = input.format.to_string();`;
  `:1915` `.bind(format.as_str())` → `.bind(input.format)`.
- `storage/src/sqlite/posts.rs:95` `.bind(input.format.to_string())` →
  `.bind(input.format)`.
- `storage/src/postgres/posts.rs:97` `.bind(input.format.to_string())` →
  `.bind(input.format)`.

Read tuple:

- `storage/src/helpers.rs:138` change element 7 `String` → `PostFormat`.
- `storage/src/helpers.rs:201-203` delete the
  `let format = format.parse::<PostFormat>().map_err(...)?;` block — `format`
  now arrives typed; use it directly in the `PostRecord { .. }` construction.
- `storage/src/helpers.rs:125-130` update the comment: `format` now decodes via
  its `impl_text_column_enum!` bridge (#572), like the newtypes — no longer a
  `String` that parses here.

- [ ] **Step 5: Update the `build_post_record` unit tests**
      (`storage/src/helpers.rs:498-647`)

Each `build_post_record((...))` call currently passes a `String` for the format
slot (five call sites: `:507`, `:535`, `:603`, `:629`, `:655`) — change those to
the typed `PostFormat` (e.g. `PostFormat::Markdown`). Delete
`test_build_post_record_rejects_invalid_format` (`:535`): an invalid format can
no longer reach `build_post_record` (it's rejected at column-decode). That
rejection is now covered by Task 2 Step 2's decode-rejection test. Also update
the now-stale test-module comment at `helpers.rs:528-532` ("`format` … still
parse in `build_post_record`"), which this task falsifies.

- [ ] **Step 6: Run the storage tests + the bind gate, verify pass**

Run: `cargo nextest run -p storage posts helpers` Expected: PASS — round-trip
(Org, Html) green, decode-rejection green, existing post CRUD green,
`build_post_record` unit tests green. Run: `cargo xtask check` (includes the
`sqlx-newtype-bind` guard and coverage) Expected: PASS — no `.bind(...)` strip
flagged (binds are now typed values); the bridge `Encode`/`Decode` are covered
by the round-trip + rejection tests.

- [ ] **Step 7: Commit**

```bash
git add common/src/db_enum.rs common/src/lib.rs common/src/render.rs storage/src/posts.rs storage/src/sqlite/posts.rs storage/src/postgres/posts.rs storage/src/helpers.rs
git commit -m "refactor(storage): typed PostFormat via reusable sqlx text-enum bridge (#572)"
```

Run `cargo xtask check` first. No `Co-Authored-By`.

---

### Task 3: `FormatToggle` component + replace the four `.j-seg` blocks (web)

**Files:**

- Modify: `web/src/posts/component.rs` — add `FormatToggle`; replace the blocks
  at `:79`, `:556`, `:725`, `:1736`.
- Test (guard, not new unit): `web/src/profile/mod.rs:106` (`serde_qs`
  form-transport) stays green; `end2end/tests/posts.spec.ts:626` + `:125` stay
  green.

**Interfaces:**

- Consumes: `PostFormat::VARIANTS` + `get_message()` (Task 1);
  `common::render::PostFormat`.
- Produces:
  `#[component] pub fn FormatToggle(format: RwSignal<PostFormat>, #[prop(optional, into)] style: Option<&'static str>) -> impl IntoView`.

- [ ] **Step 1: Add the `FormatToggle` component** to
      `web/src/posts/component.rs` (near `ComposerFields`). The DOM contract
      (spec "DOM preservation") is pinned by the existing e2e; the body follows
      from it:

```rust
/// The `.j-seg` Markdown/Org format toggle, shared by every post editor. Renders one
/// button per user-selectable `PostFormat` (those with a `strum` editor message);
/// `Html` has none, so it is filtered out.
#[component]
pub fn FormatToggle(
    format: RwSignal<PostFormat>,
    /// Extra inline style for the `.j-seg` wrapper (e.g. spacing). Omitted when unset.
    #[prop(optional, into)]
    style: Option<&'static str>,
) -> impl IntoView {
    use strum::{EnumMessage, VariantArray};
    view! {
        <div class="j-seg" style=style>
            {PostFormat::VARIANTS
                .iter()
                .copied()
                .filter_map(|f| f.get_message().map(|label| (f, label)))
                .map(|(f, label)| {
                    view! {
                        <button
                            type="button"
                            class=move || {
                                if format.get() == f { "j-btn is-selected" } else { "j-btn" }
                            }
                            on:click=move |_| format.set(f)
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
```

- [ ] **Step 2: Replace the four inline `.j-seg` blocks** with
      `<FormatToggle .../>`:
  - `component.rs:79-106` (in `ComposerFields`' `show_seg` branch) →
    `<FormatToggle format=format />`.
  - `component.rs:556-583` (full toolbar) → `<FormatToggle format=format />`.
  - `component.rs:725-752` (compact toolbar, spaced) →
    `<FormatToggle format=format style="margin-top:10px" />`.
  - `component.rs:1736-1763` (edit page, spaced) →
    `<FormatToggle format=format style="margin-top:10px" />`.

  After: `rg -n 'class="j-seg"' web/src/posts/component.rs` must show exactly
  ONE occurrence (inside `FormatToggle`).

- [ ] **Step 3: Wasm clippy + host build**

Run: `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS (catches `must_use_candidate` etc. that `check`/`build` skip).
Run: `cargo nextest run -p web` (keeps `profile/mod.rs:106`'s `serde_qs` test
green — the derived-enum form transport) Expected: PASS.

- [ ] **Step 4: Drive the toggle e2e** (behavior unchanged is the acceptance)

Run: `cargo xtask e2e-local posts` (host runner, auto-seeds; ~3 min) Expected:
PASS — including `posts.spec.ts:626` "inline composer: format toggle switches
active button" and the `:125` Org-toggle click.

- [ ] **Step 5: Commit**

```bash
git add web/src/posts/component.rs
git commit -m "refactor(web): extract shared FormatToggle component, dedupe .j-seg toggle (#572)"
```

Run `cargo xtask check` first. No `Co-Authored-By`.

---

## Notes for the implementer

- The ADR draft `docs/adr/drafts/adopt-strum-retire-str-enum.md` and the
  ADR-0074 supersede note are already written; they are **promoted at ship**
  (`cargo xtask adr promote`), not in a task here.
- If Task 1's `parse_err_ty`/`parse_err_fn` or `EnumMessage` fails to compile
  against strum 0.28, that is the spike surfacing an assumption — fix the syntax
  against strum's docs before proceeding; do not work around it by reintroducing
  `StrEnum`.
- Task 2's bridge is the reusable artifact #607 depends on — keep
  `impl_text_column_enum!` general (no `PostFormat`-specific logic).
