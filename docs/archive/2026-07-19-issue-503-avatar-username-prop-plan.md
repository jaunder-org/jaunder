# Plan — #503: `Avatar` prop takes `Username`

> Note: an earlier revision targeted `Option<Username>`; the prop was changed to
> required `Username` at the ship merge-halt (see the spec's Decision
> "Amendment"). This plan reflects the shipped design.

- Spec:
  [2026-07-19-issue-503-avatar-username-prop.md](../specs/2026-07-19-issue-503-avatar-username-prop.md)
- Issue: [#503](https://github.com/jaunder-org/jaunder/issues/503)
- For agentic workers: drive with **jaunder-iterate**; a task may be delegated
  via **jaunder-dispatch**. Tick checkboxes in real time.

## Review header

**Goal.** Type the `Avatar` component's `name` prop as required `Username`
instead of `String`, dropping the `.to_string()`/`String::from`/`unwrap_or_default()`
stringification at its call sites. An avatar always represents a real signed-in
user, so the type is required (no empty fallback); the genuinely-absent cases
render no avatar. Restores type-safety at the component boundary (ADR-0063
pervasive newtype use).

**Scope.**

- **In:** `web/src/avatar/component.rs` (component signature + body + import);
  the three `<Avatar>` call sites in `web/src/pages/ui.rs`.
- **Out:** `AudienceHeader` (stays `String` — reactive-store carve-out);
  `Chip`/`Dot`/`TagLabel` (obsoleted by the ADR-0070 reorg); the pure
  `render`/`avatar_parts` twins (stay `&str`, untouched); typing the auth
  marker. See the spec's "Out of scope".

**Tasks (one line each).**

1. Retype `Avatar.name` → `Username`; convert the three call sites (real
   `Username`, `Option::map`-guarded where the source is optional/fallible).
   (Atomic — one commit; the prop and its callers must compile together.)

**Key risks / decisions.**

- **`#[expect(clippy::needless_pass_by_value)]` stays fulfilled.** The body uses
  `avatar_parts(&name)` (borrows), so `needless_pass_by_value` still fires and
  the `#[expect]` is still satisfied — no unfulfilled-expectation clippy error.
  Verify at the gate.
- **No visible behavior change.** The pure `render` twin and `avatar_parts(&str)`
  are untouched; the reactive `Avatar` renders byte-identically to the old
  `String` path for the same value (`avatar_parts(&name)` via `&Username → &str`
  deref coercion). The "no username" branch is never rendered in practice
  (composer avatar is compact-only via `InlineComposer`, which always passes a
  `Username`; `CreatePostPage`'s full form has no avatar; the marker always
  parses), so making it required loses nothing.
- **Sidebar uses `.parse()`.** `Username: FromStr` (`common/src/username.rs`),
  so `username.parse::<Username>().ok()` borrows the marker `String` (still
  rendered raw as the footer label) with no clone, via an extracted local (the
  `view!` macro rejects the turbofish inline).
- **Coverage.** `#[component]` bodies are coverage-exempt (ADR-0050), so the
  `Option::map` render guards need no new test; `avatar_parts("")` is already
  covered by `avatar_parts_empty_name`.

## Global constraints

- Rust; edits via structured Edit, not shell.
- Gate: `cargo xtask check` (fmt + clippy + Nix coverage/tests) must pass clean
  before the commit — the pre-commit hook runs it. Because this touches
  `#[cfg(feature = "server")]`-adjacent web code, the gate's
  `--all-features --all-targets` build is the real check (a default
  `cargo check` would skip server-gated paths — see the "default check skips
  server-gated web code" hazard).
- No `Co-Authored-By` trailer (**jaunder-commit**).
- Review base is `wt-base-issue-503`; diff with `git diff main...HEAD`.

---

## Task 1 — `Avatar.name: Username` + call sites

**Files / interfaces**

- `web/src/avatar/component.rs` (the reactive component's home after the #527
  `ui/` dissolution)
  - Add `use common::username::Username;`.
  - Change the component signature:
    ```rust
    #[component]
    pub fn Avatar(name: Username, #[prop(default = 38)] size: u32) -> impl IntoView {
    ```
  - The body stays `let (initials, hue) = avatar_parts(&name);` (`&Username →
    &str` deref coercion via the `StrNewtype`-generated `Deref<Target = str>`).
  - Leave the `#[expect(clippy::needless_pass_by_value)]` attribute in place
    (still fulfilled — see Key risks). Leave `avatar_parts`, the pure `render`
    twin (in `markup.rs`), and all `#[cfg(test)]` tests unchanged.

- `web/src/pages/ui.rs` (`use common::username::Username;` already present)
  - Post header (`PostDisplay` authored arm):
    ```rust
    <Avatar name=post.username.clone() size=38 />
    ```
    (was `name=post.username.to_string()`; the clone is required — `post.username`
    is borrowed elsewhere in the fn.)
  - Composer (`PostCreateForm` compact branch; prop stays
    `#[prop(optional)] username: Option<Username>`):
    ```rust
    {username.map(|u| view! { <Avatar name=u size=36 /> })}
    ```
    (was `name=username.map(String::from).unwrap_or_default()`. Renders no avatar
    when absent; `InlineComposer` always passes a real `Username`.)
  - Authed sidebar footer (`authed_sidebar`; `username: String`, also rendered
    as the footer label) — extract a commented local before the `view!`:
    ```rust
    let avatar_name = username.parse::<Username>().ok();
    ```
    then render it guarded:
    ```rust
    {avatar_name.map(|u| view! { <Avatar name=u size=28 /> })}
    ```
    (was `name=username.clone()`. `.parse()` borrows; the raw `String` still
    renders as the label. A malformed marker deliberately shows no avatar.)

**Verify — gate + twin test**

- `cargo xtask check` — expect PASS (fmt clean, clippy clean incl. the
  `#[expect]`, Nix coverage/tests green). This compiles the web crate
  `--all-features --all-targets`, so the server-gated `PostCard`/composer paths
  are built.
- Targeted twin test (sanity, subset of the above):
  `cargo nextest run -p web avatar` — expect PASS
  (`avatar_matches_reactive_component_markup`, `avatar_parts_empty_name`, and
  the `avatar_parts_*` tests unchanged and green — proves markup is
  byte-identical).
- Confirm the acceptance greps hold on the diff:
  - no `to_string()` / `String::from` / `unwrap_or_default()` on a username at
    the `<Avatar>` sites;
  - `Avatar`'s signature reads `name: Username`.

**Commit** (after the gate is green — **jaunder-commit**; verify `git status` is
clean of hook-restaged files first):

```
web: Avatar takes Option<Username> not String (#503)

Retype the render-only Avatar's `name` prop from `String` to
`Option<Username>`, moving the empty-name fallback inside the component and
dropping the per-call-site stringification at its three call sites
(post header, composer, authed sidebar). The pure `render` twin and
`avatar_parts` stay `&str`, so rendered markup is byte-identical.

AudienceHeader stays `String` (reactive-store PatchField carve-out); the
Chip/Dot tag work the issue named was already absorbed by the ADR-0070 UI
reorg (TagList holds TagLabel).
```

## Self-review checklist

- [x] Every spec acceptance criterion (1–6) maps to Task 1's edits or verify
      step.
- [x] No placeholder / TODO left in the diff.
- [x] Diff is exactly: 1 signature + 1 import in `avatar/component.rs`, 3 call
      sites (+ 1 extracted `avatar_name` local) in `pages/ui.rs`. Nothing in
      `audiences/`, `taglist.rs`, or the ADR set (no new ADR).
- [x] `cargo xtask check` green (+ `validate --no-e2e` green); `git status`
      clean. Note: the sidebar binds an extracted, commented local
      `let avatar_name = username.parse::<Username>().ok();` before the `view!`
      (the macro rejects a turbofish inline, and a local keeps the intent
      comment out of leptosfmt's reach) and renders it `{avatar_name.map(|u|
      view! { <Avatar name=u size=28 /> })}` — a deliberate-no-avatar-on-
      malformed-marker choice surfaced by the standards review; the composer
      site is likewise `Option::map`-guarded.
