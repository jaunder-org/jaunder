# Spec — issue #572: shared `FormatToggle` component (dedupe `.j-seg` toggle) + first pass of `StrEnum` retirement

/ Issue: jaunder-org/jaunder#572 · Labels: `dx`, `web` · Milestone: none /
Worktree: `.claude/worktrees/issue-572-format-toggle-component` / Related: #607
(retire `StrEnum` — migrate the remaining enums + delete the / macro; this issue
does `PostFormat` first and does not wait on it). / ADR:
`docs/adr/drafts/adopt-strum-retire-str-enum.md` (supersedes the still-proposed
/ ADR-0074), promoted at ship.

## Problem

Two coupled problems, resolved together:

1. **Toggle duplication (the issue as filed).** The `.j-seg` Markdown/Org
   format-toggle button block is hand-rolled in **four** places in
   `web/src/posts/component.rs`, each byte-identical except an optional spacing
   style:

   | Site                                        | Line (pre-change) | `.j-seg` wrapper                              |
   | ------------------------------------------- | ----------------- | --------------------------------------------- |
   | `ComposerFields` (rendered when `show_seg`) | 79                | `<div class="j-seg">`                         |
   | `PostCreateForm` full composer toolbar      | 556               | `<div class="j-seg">`                         |
   | `PostCreateForm` compact toolbar            | 725               | `<div class="j-seg" style="margin-top:10px">` |
   | `EditPostPage`                              | 1736              | `<div class="j-seg" style="margin-top:10px">` |

   Each renders two `<button>`s (Markdown, Org) whose `class` closure toggles
   `is-selected` on `format.get()` and whose `on:click` calls `format.set(...)`.

2. **A shared component wants variant enumeration + labels, and that surfaced
   that `StrEnum` doesn't earn its keep.** Doing the toggle right (iterate the
   variants, so a future format is data not markup) needs a variant list and
   per-variant labels. `PostFormat` rides the bespoke `#[derive(StrEnum)]` macro
   (`macros/`, ADR-0074), which provides no enumeration. Investigation
   established that **`strum` 0.28 can do everything `StrEnum` does** —
   including the named, host-registrable parse error that ADR-0074 was written
   to secure — via `#[strum(parse_err_ty = …, parse_err_fn = …)]`. `StrEnum` is
   therefore ~300 lines of bespoke macro duplicating a standard crate already in
   the tree (`BackupMode` uses `strum`). ADR-0074 is still `Status: proposed`.
   Decision: **retire `StrEnum` in favor of `strum`**, starting with
   `PostFormat` here (the rest in #607). See the ADR draft.

## Change

### A. Migrate `PostFormat` off `StrEnum` onto `strum` (`common/src/render.rs`)

Adopt the `strum` stack (as `BackupMode` does for the mechanical derives) + a
`thiserror` error. **Two pieces have no in-repo precedent** —
`EnumMessage`-driven label/filtering (`BackupMode` hand-writes `label()`) and
`parse_err_ty` (`BackupMode` tolerates strum's default `ParseError`) — so treat
them as verify-on-compile, not proven by precedent. Replace
`#[derive(StrEnum)]` + `#[str_enum(...)]` with:

```rust
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Default,
    Serialize, Deserialize,
    strum::VariantArray, strum::AsRefStr,
    strum::Display, strum::EnumString, strum::EnumMessage,
)]
#[serde(into = "String", try_from = "String")]
#[strum(serialize_all = "snake_case")]
#[strum(parse_err_ty = InvalidPostFormat, parse_err_fn = post_format_parse_err)]
pub enum PostFormat {
    #[default]
    #[strum(message = "Markdown")]
    Markdown,
    #[strum(message = "Org")]
    Org,
    Html, // renderer-internal provenance (#445); no editor message → excluded from toggles
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("post format must be \"markdown\", \"org\", or \"html\"")]
pub struct InvalidPostFormat;

fn post_format_parse_err(_: &str) -> InvalidPostFormat { InvalidPostFormat }

// serde `into`/`try_from` proxy: serialize the token; deserialize an owned String
// through FromStr so the domain InvalidPostFormat message is preserved (below).
impl From<PostFormat> for String {
    fn from(format: PostFormat) -> Self { format.as_ref().to_owned() }
}
impl TryFrom<String> for PostFormat {
    type Error = InvalidPostFormat;
    fn try_from(s: String) -> Result<Self, Self::Error> { s.parse() }
}

impl_text_column_enum!(PostFormat); // the reusable sqlx bridge — see A2
```

serde routes through an owned-`String` proxy (`into`/`try_from`), NOT the
derived enum (de)serializer. The spike confirmed a _derived_ enum decodes via
`serde_qs` 0.15 — but it yields serde's generic "unknown variant" message, and
`server/tests/web/web_posts.rs` (`create_post_rejects` / `update_post_rejects`
`invalid_format` cases) **assert the domain message** `post format must be …` at
the web boundary. Routing deserialize `String` → `FromStr` preserves that
message (it is what `StrEnum` hand-rolled its `Deserialize` for) and
single-sources the wire token in `as_str` (no `rename_all`).

Behavior that MUST be preserved (this is a wire/DB/error-compatible swap, not a
semantic change):

- **Wire/DB tokens unchanged:** `markdown` / `org` / `html`. strum
  `serialize_all = snake_case` drives `Display`/`AsRef`/`FromStr`; serde routes
  through `as_str` via the `String` proxy — one token source. `StrEnum` produced
  the same single-word tokens.
- **serde form transport + error message preserved:**
  `web/src/profile/mod.rs:106`'s `serde_qs` test and the `web_posts.rs`
  `invalid_format` cases (which assert the `post format must be …` message) MUST
  stay green; a JSON round-trip is added (AC3). The proxy keeps the deserialize
  error message identical to `StrEnum`'s (`InvalidPostFormat`).
- **`Display` unchanged:** `PostFormat::Markdown.to_string() == "markdown"`
  (strum `Display` via `serialize_all`). The existing `render.rs` test stays
  green.
- **`FromStr` unchanged, same error:** `"html".parse::<PostFormat>()` →
  `Ok(Html)`; an unknown token → `Err(InvalidPostFormat)`. `EnumString` +
  `parse_err_ty` restores the exact `Err` type; `parse_err_ty` also auto-derives
  `TryFrom<&str>`. `storage/src/user_config.rs:44` (`.ok()`) is unchanged.
- **`InvalidPostFormat` unchanged:** same name, same `common::render` path, same
  message, still `impl std::error::Error`. `host/src/error.rs`'s
  `validation_from!(… InvalidPostFormat …)` and the
  `from_common_validation_sources_*` test keep working untouched;
  `storage/src/posts.rs`'s re-export is unaffected.
- **New surface for the toggle:** `PostFormat::VARIANTS` (`VariantArray`,
  declaration order) and per-variant `get_message()` (`EnumMessage`):
  `Some("Markdown")`, `Some("Org")`, `None` for `Html`.

`StrEnum` itself is **not** deleted — the other enums still use it (that's
#607).

### A2. Reusable sqlx bridge for strum text-enums (`common`), + typed bind/decode

`PostFormat` becomes a first-class stored value (like the `StrNewtype` newtypes,
#438), via a **reusable** `macro_rules!` in `common` — not a per-type
hand-written bridge — so #607 reuses it for the other stored enums. `sqlx`'s own
`#[derive(Type)]` does not fit: for Postgres it maps a plain enum to a _native_
DB enum type, but the token is stored as **TEXT** in both backends. The macro
lifts the `String`-delegating shape `RenderedHtml` uses (`render.rs:143`) into
one reusable definition:

```rust
// Requires: $ty: AsRef<str> (strum AsRefStr) + FromStr<Err: Error + Send + Sync + 'static>.
macro_rules! impl_text_column_enum {
    ($ty:ty) => {
        #[cfg(feature = "sqlx")]
        const _: () = {
            impl<DB: sqlx::Database> sqlx::Type<DB> for $ty where String: sqlx::Type<DB> {
                fn type_info() -> <DB as sqlx::Database>::TypeInfo { <String as sqlx::Type<DB>>::type_info() }
                fn compatible(t: &<DB as sqlx::Database>::TypeInfo) -> bool { <String as sqlx::Type<DB>>::compatible(t) }
            }
            impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for $ty where for<'a> &'a str: sqlx::Encode<'q, DB> {
                fn encode_by_ref(&self, buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>)
                    -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                    let s: &str = self.as_ref();
                    <&str as sqlx::Encode<'q, DB>>::encode_by_ref(&s, buf)
                }
            }
            impl<'r, DB: sqlx::Database> sqlx::Decode<'r, DB> for $ty where &'r str: sqlx::Decode<'r, DB> {
                fn decode(v: <DB as sqlx::Database>::ValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
                    Ok(<&str as sqlx::Decode<'r, DB>>::decode(v)?.parse()?)
                }
            }
        };
    };
}
```

Because the bridge is the **full** `Type`/`Encode`/`Decode`:

- **Write:** `storage/src/posts.rs:1915` `.bind(format.as_str())` →
  **`.bind(format)`** — a typed bind, gate-clean (no `.as_str()`/`.as_ref()`
  strip).
- **Read:** the post row query in `storage/src/posts.rs` (and the `sqlite/` +
  `postgres/` variants, per ADR-0053 dual-backend) decodes the `format` column
  **as `PostFormat`**; `storage/src/helpers.rs:201-203`'s manual
  `.parse::<PostFormat>().map_err(sqlx::Error:: Decode)` is deleted (`format`
  arrives already typed), retiring the fallible special-case its own comment
  (`:199`) flags.

This is a declarative gap-filler (no crate covers dual-backend TEXT enums),
distinct from the `StrEnum` proc-macro that duplicated `strum`.

### B. `FormatToggle` component (`web/src/posts/component.rs`)

```rust
#[component]
pub fn FormatToggle(
    format: RwSignal<PostFormat>,
    /// Extra inline style for the `.j-seg` wrapper (e.g. spacing). Omitted when unset.
    #[prop(optional, into)] style: Option<&'static str>,
) -> impl IntoView {
    use strum::{EnumMessage, VariantArray};
    view! {
        <div class="j-seg" style=style>
            {PostFormat::VARIANTS
                .iter()
                .copied()
                .filter_map(|f| f.get_message().map(|label| (f, label)))
                .map(|(f, label)| view! {
                    <button
                        type="button"
                        class=move || if format.get() == f { "j-btn is-selected" } else { "j-btn" }
                        on:click=move |_| format.set(f)
                    >
                        {label}
                    </button>
                })
                .collect_view()}
        </div>
    }
}
```

- **Spacing** → optional `style` passthrough. Two spaced sites pass
  `style="margin-top:10px"`; the other two omit it. `Option<&'static str>` (not
  a `""` default) so an unset wrapper renders `<div class="j-seg">` with **no**
  `style` attribute — byte-identical to today.
- The variant list is compile-time-fixed, so a static `.map().collect_view()`
  (not `<For>`) is correct; only the per-button `class` closure is reactive.
- `Html` carries no editor message → `get_message()` is `None` → filtered out. A
  future editor format is added by giving a variant a `#[strum(message = "…")]`;
  it then appears automatically (strum's default-include, opt-out-by-omission).

### C. Replace the four `.j-seg` blocks

Each site renders `<FormatToggle format=format />` (the two spaced sites add
`style="margin-top:10px"`). `ComposerFields` renders it in its `show_seg`
branch.

## DOM preservation (the e2e contract)

The rendered DOM must be identical to today's so the existing e2e stays green
(`end2end/tests/posts.spec.ts:626` "inline composer: format toggle switches
active button", plus the `.j-seg button:has-text("Org")` click at line 125): a
`div.j-seg` containing, per selectable format (order `Markdown`, `Org`), a
`<button type="button">` with the format's label text, `class` =
`"j-btn is-selected"` when it is the selected format else `"j-btn"`, and
`on:click` setting `format`.

## Acceptance criteria (observable)

1. **Single source of truth (markup).** `.j-seg` toggle markup appears in
   exactly one place in `component.rs` — `FormatToggle`; the four inline blocks
   are gone, each site renders `<FormatToggle .../>`. _Verify:_
   `rg -n 'class="j-seg"' web/src/posts/component.rs` → exactly one occurrence,
   inside `FormatToggle`.
2. **Iterated, data-driven options.** `FormatToggle` renders one button per
   `PostFormat::VARIANTS` entry with an editor message; no per-variant hardcoded
   `<button>`. _Verify:_ code review — a single
   `.iter()...filter_map(get_message)...`.
3. **`PostFormat` wire/DB/error compatibility.** Tokens `markdown`/`org`/`html`
   unchanged across serde + `Display` + `FromStr`; `"html"` still parses;
   `InvalidPostFormat` keeps its name, path, message, and `Error` impl.
   _Verify:_ the five existing `render.rs` `PostFormat` tests (`:442-488`) stay
   green **with no edits** (they exercise only public
   `Display`/`Debug`/`FromStr`); `host/error.rs`'s
   `from_common_validation_sources_*` test stays green;
   `web/src/profile/mod.rs:106`'s `serde_qs` test stays green; and NEW tests
   assert (a) a serde/JSON round-trip of each token and (b) a
   `serde_qs::from_str("format=markdown")` decode + `format=bogus` rejection
   (locking the form-transport path).
4. **`PostFormat` is a typed stored value (reusable bridge).** A reusable
   `impl_text_column_enum!` `macro_rules!` in `common` gives `PostFormat` an
   `sqlx` `Type`/`Encode`/`Decode` bridge; the storage write binds it directly
   (`.bind(format)`, no `.as_str()`/`.as_ref()` strip) and the post row read
   decodes it as `PostFormat` (the `helpers.rs:201-203` manual parse is gone).
   _Verify:_ `rg -n '\.bind\(format' storage/src` shows `.bind(format)` (not
   `.as_str()`); the `sqlx-newtype-bind` gate passes; the post CRUD storage
   tests (sqlite + postgres) stay green; a store→load round-trip test asserts a
   `PostFormat` survives the DB.
5. **`StrEnum` scope untouched.** `PostFormat` no longer derives `StrEnum`; the
   macro and the other `StrEnum` enums are unchanged. _Verify:_
   `rg -n 'StrEnum' common/src` shows `PostFormat` gone from the list, macro
   intact.
6. **Behavior unchanged (toggle).** Per selectable format the button shows its
   label, carries `j-btn is-selected` iff selected (else `j-btn`), `on:click`
   selects it; order Markdown, Org; Html not rendered. _Verify:_
   `posts.spec.ts:626`/`:125` green.
7. **Spacing preserved.** Compact composer + edit-page render `margin-top:10px`;
   the other two render no wrapper `style`. _Verify:_ two call sites pass the
   style, two omit it.
8. **Coverage.** The `thiserror` error path (unknown token →
   `InvalidPostFormat`) and `get_message`/`VARIANTS` are exercised by tests;
   coverage gate green. _Verify:_ `render.rs` tests cover parse-failure + each
   variant's message.
9. **Gate green.** `cargo xtask validate --no-e2e` (static + clippy + coverage),
   wasm clippy clean
   (`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`), and
   the posts e2e pass.

## Out of scope / non-goals

- Migrating the remaining `StrEnum` enums (visibility, media, registration) or
  deleting the `StrEnum` macro — that is #607.
- Any change to `PostFormat`'s variants, wire tokens, or the DB representation
  (still a TEXT token); the migration is representation-compatible. (The storage
  _code_ does change — from a stringly `.bind(format.as_str())` + manual
  read-parse to a typed `Encode`/`Decode` bridge — but the stored bytes are
  identical, so no data migration.)
- Enabling a new format (`Html` stays message-less/filtered); this only makes
  enabling one later a one-attribute change.
- Restyling the toggle; `.j-btn`/`is-selected`/`.j-seg` are preserved verbatim.

## Decision record

The strum-over-`StrEnum` decision is recorded in the ADR draft
`docs/adr/drafts/adopt-strum-retire-str-enum.md`, which **supersedes the
still-proposed ADR-0074** and is promoted at ship. This spec's `PostFormat`
migration is that ADR's first implementation; #607 completes it.
