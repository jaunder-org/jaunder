# Spec — #503: render-only component props take domain newtypes (`Avatar`)

- Issue: [#503](https://github.com/jaunder-org/jaunder/issues/503)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (pervasive newtype use; reactive-store carve-out),
  [ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md) (the UI
  file layout that overtook part of this issue)
- Date: 2026-07-19

## Problem

The issue (filed 2026-07-17) asked four render-only components — `Avatar`,
`AudienceHeader`, `Chip`, `Dot` — to take domain newtypes instead of `String`,
so the stringification happens once inside the component rather than at every
call site. Since it was filed, the **ADR-0070 UI reorg** (`web/src/ui.rs` split
into `web/src/ui/{avatar,icon,taglist,topbar,mod}.rs`) landed and changed the
ground:

- **`Chip` and `Dot` no longer exist.** Tag rendering moved into `TagList`
  (`web/src/ui/taglist.rs`), which **already holds `TagLabel`** and stringifies
  once at the render boundary (`taglist.rs:77`); the pure twin uses
  `escape_html(&tag.display)`. The issue's intent (convert once inside the
  component) is **already satisfied** for tags. The `Dot`/`proto` role collapsed
  into `SidebarSource` with an already-`&'static str`, presentational `proto` (a
  CSS custom-property name) — not a domain value.
- **`AudienceHeader(name: String)`** (`web/src/audiences/mod.rs:449`) has one
  caller (`:440`) that passes `initial_name` — bound at `:436` from
  `row.name().get_untracked()` — sourced from `AudienceSummary.name`, which is
  **deliberately `String`** per the ADR-0063 reactive-store `PatchField`
  carve-out documented at `mod.rs:42-52` (same reason `audience_id` stays
  `i64`). Internally it only uses `name` to seed
  `ValidatedField::<AudienceName>::prefilled(&name)` (wants `&str`). Typing the
  prop `AudienceName` would push a **fallible `String → AudienceName` parse to
  the reactive-store edge the carve-out exists to keep primitive** — strictly
  worse. (This overrides the issue's stated premise that `AudienceHeader`
  callers "hold `AudienceName`"; post-reorg the caller holds a carve-out
  `String`.)

The only genuine remaining target is **`Avatar(name: String)`**
(`web/src/ui/avatar.rs:47`). Its three call sites do not uniformly hold a
`Username`:

| Site              | Current arg                                      | Upstream type                                                                                                                      |
| ----------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `pages/ui.rs:180` | `post.username.to_string()`                      | `Username`                                                                                                                         |
| `pages/ui.rs:491` | `username.map(String::from).unwrap_or_default()` | `Option<Username>` (composer may be anonymous → empty avatar)                                                                      |
| `pages/ui.rs:975` | `username.clone()`                               | raw `String` from the wasm auth marker (`marker_username_on_boot() -> Option<String>`, ADR-0044), also rendered directly at `:977` |

The `None`/empty path is reachable: `CreatePostPage` (`posts.rs:44`) omits the
`username` prop, so the composer renders an empty avatar chip.

## Decision

**Scope #503 to `Avatar` only** (per the pragmatic scope approved in the design
interview). `AudienceHeader` stays `String` (carve-out); `Chip`/`Dot`/`TagLabel`
are obsoleted by the reorg — no change.

### `Avatar` prop becomes `Username` (required)

Change the reactive component signature from `name: String` to `name: Username`
(a required prop). The component body reverts to `avatar_parts(&name)` (deref
coercion `&Username → &str` via the `StrNewtype`-generated `Deref<Target = str>`).
The pure `render(name: &str, size)` twin and `avatar_parts(&str)` are untouched —
the newtype buys type safety at the component boundary, not free rendering.

> **Amendment (merge-halt reconsideration).** An earlier revision of this spec
> made the prop `Option<Username>` with an empty-string fallback
> (`name.as_deref().unwrap_or("")`) to preserve the prior
> `unwrap_or_default()` behavior at the composer/sidebar sites. That was
> reconsidered at the ship merge-halt: an avatar **always** represents a real
> signed-in user, so `Option<Username> → ""` modeled a state that does not
> occur and rendered a blank `j-av` circle for it. Verifying the call sites
> showed the "no username" branch is **never actually rendered** — the composer
> avatar lives only in the `compact` branch (rendered solely by
> `InlineComposer`, which always passes a real `Username`); the full-page
> `CreatePostPage` composer (`compact = false`) has no avatar at all; the
> sidebar marker always parses. So the honest type is required `Username`, and
> the genuinely-absent cases render **no avatar** (never a blank one).

### Call-site conversions (each supplies a real `Username`)

- **Post header** (`pages/ui.rs`, `PostDisplay` authored arm) →
  `name=post.username.clone()` (drop `.to_string()`; `post.username` is a
  `Username`, still borrowed elsewhere so the clone is required).
- **Composer** (`pages/ui.rs`, `PostCreateForm` compact branch) →
  `{username.map(|u| view! { <Avatar name=u size=36 /> })}`. The prop stays
  `#[prop(optional)] username: Option<Username>` (the non-compact
  `CreatePostPage` legitimately omits it — no avatar there); `InlineComposer`
  always passes a real `Username`, so the avatar always renders. `None` renders
  nothing rather than a blank chip.
- **Authed sidebar footer** (`pages/ui.rs`, `authed_sidebar`) → an extracted,
  commented local `let avatar_name = username.parse::<Username>().ok();` (the
  `view!` macro rejects a turbofish inline, and a local keeps the intent comment
  out of leptosfmt's reach), rendered `{avatar_name.map(|u| view! { <Avatar
  name=u size=28 /> })}`. The marker is stored from a validated username at
  login so the parse effectively always succeeds; a malformed marker
  deliberately shows no avatar. This site adds a parse rather than a
  stringification — accepted as the cost of typing the boundary without also
  typing the auth marker (a separate concern).

### No new ADR

The component-signature choice sits within existing conventions (ADR-0063
pervasive newtype use; the reactive-store carve-out that keeps `AudienceHeader`
and `AudienceSummary.name` on `String` is already documented). Nothing novel to
record.

## Acceptance criteria (observable)

1. **`Avatar`'s `name` prop is `Username`** — not `String`
   (`web/src/avatar/component.rs`, relocated from `ui/avatar.rs` by the #527
   reorg). It imports/uses `common::username::Username`.
2. **No username stringification at the `Avatar` call sites.** In
   `web/src/pages/ui.rs`, no `<Avatar …>` instantiation calls `.to_string()`,
   `String::from`, or `.unwrap_or_default()` on the username; each supplies a
   real `Username` (`post.username.clone()`; the composer's `username` via
   `Option::map`; the sidebar's `username.parse::<Username>().ok()` via
   `Option::map`).
3. **Rendered markup is byte-identical for a given username.** The pure `render`
   twin and `avatar_parts` are unchanged; the reactive `Avatar` renders exactly
   as the old `String` avatar did for the same value. Where there is genuinely
   no username the avatar is **absent** (not a blank chip). The existing
   `avatar_matches_reactive_component_markup` and `avatar_parts_empty_name`
   tests still pass unchanged.
4. **`AudienceHeader` remains `name: String`** — the reactive-store carve-out is
   preserved; no fallible parse is introduced at `audiences/mod.rs:440`.
5. **`Chip`/`Dot`/`TagLabel` unchanged** — no attempt to re-add removed
   components or to make `TagLabel` `IntoRender` (out of scope).
6. **Gate green** — `cargo xtask check` passes (static + clippy + coverage),
   including the web `--all-features --all-targets` server-gated build.

## Out of scope (recorded)

- Typing the wasm auth marker (`marker_storage`) as `Username` end-to-end —
  would make `:975` infallible but touches ADR-0044; a separate issue if
  desired.
- Threading the real `current_user` username into `CreatePostPage`'s composer so
  it stops showing an empty avatar for a signed-in user — a pre-existing quirk,
  not this issue.
- Making `TagLabel` implement Leptos `IntoRender`/`IntoAttributeValue` to remove
  the render-boundary `.to_string()` at `taglist.rs:77`.
