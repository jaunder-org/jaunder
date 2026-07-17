# Spec — #505: `Topbar` title/sub props as `TextProp`

- Issue: [#505](https://github.com/jaunder-org/jaunder/issues/505)
- Milestone: Code quality ratchet
- Date: 2026-07-17

## Problem

The reactive `Topbar` component (`web/src/ui/topbar.rs:27-31`) declares its text
props as reactive signals:

```rust
#[prop(into)] title: Signal<String>,
#[prop(optional, into)] sub: Option<Signal<String>>,
```

A `&'static str` does **not** implement `Into<Signal<String>>`, so every call
site that passes a static literal must wrap it in `.to_string()` purely to
satisfy the prop type — e.g. `title="Posts".to_string()`. This boilerplate
appears at ~22 call sites across `web/src`, and the vast majority of them are
static labels that have no reason to be reactive.

## Decision

Switch both props to `leptos::TextProp`, the standard Leptos type for "a value
that is either a static or a reactive string":

```rust
#[prop(into)] title: TextProp,
#[prop(optional, into)] sub: Option<TextProp>,
```

`TextProp` (verified against leptos 0.8.20 `src/text_prop.rs`) provides `From`
impls for **all** the shapes the call sites use:

- `From<&'static str>` — static literals: `title="Posts"`
- `From<String>` — owned strings
- `From<F: Fn() -> S where S: Into<Oco<'static, str>>>` — bare reactive
  closures: `title=move || format!("Posts by {}", display_username())`
- `From<Signal<V, S>>` (and `ReadSignal`/`Memo`/`RwSignal`/…) — derived signals:
  `title=Signal::derive(move || format!("#{}", read_tag()))`

So both the static and the reactive call sites keep compiling; the
`#[prop(into)]` attributes stay (they drive the `.into()` conversion), and the
reactive callers lose nothing.

### Scope of change

1. **`web/src/ui/topbar.rs`** — change the two prop type annotations only. The
   component body (`{move || title.get()}`, `{move || s.get()}`) is unchanged:
   `TextProp::get()` returns `Oco<'static, str>`, which renders as a text node
   exactly as the previous `Signal::get() -> String` did. No import change —
   `TextProp` is already reachable via the existing `use leptos::prelude::*;`
   (re-exported through `text_prop::*`).

2. **Call sites** — drop the now-superfluous `.to_string()` from every static
   `title=…` / `sub=…` argument (~22 sites, listed in the issue). The three
   reactive `title=` callers (`posts.rs:465` closure, `posts.rs:1225` and
   `posts.rs:1421` `Signal::derive`) are left as-is except for dropping
   `.to_string()` on any _static_ `sub=` they carry.

### Non-goals / unaffected

- The **server projector twin**
  `render_topbar(title: &str, sub: Option<&str>, right: &str)` (`topbar.rs:11`)
  is a plain function and is **not** touched. Its byte-identical markup contract
  with the reactive `Topbar` is preserved: the rendered DOM (`<h1>{title}</h1>`,
  optional `<div class="j-sub">…</div>`) is unchanged by the prop-type swap, so
  the existing `topbar_*_matches_reactive_component_markup` tests continue to
  hold without modification.

## Acceptance

- No `title="…".to_string()` / `sub="…".to_string()` wrappers remain at `Topbar`
  call sites (grep for `<Topbar` shows bare string literals).
- Reactive titles (`posts.rs:465/1225/1421`) still update — verified by driving
  the affected pages (user timeline, tag feed) in the browser during
  `jaunder-iterate`.
- `cargo xtask validate --no-e2e` green (host static + clippy + coverage);
  targeted e2e for the reactive-title pages green.
