# Spec — issue #333: adopt maud for the `web` render layer

- Issue: [#333](https://github.com/jaunder-org/jaunder/issues/333)
- Milestone: 11 — "Web: canonical Leptos CSR convergence"
- Date: 2026-08-01
- Relates: ADR-0040 (leptos-CSR), ADR-0041 (projector + CSR client), ADR-0056
  (canonical co-located Leptos), ADR-0079 (rendered-html sanitization), ADR-0085
  (static gates enumerate, they do not search)

## Problem

`web`'s render layer builds HTML with `format!`/`push_str` plus a hand-rolled
`crate::html::escape_html`. Every interpolation of untrusted text must remember
to call it; forgetting is an XSS hole that compiles clean and looks like every
other line. There are 23 `escape_html` call sites across 5 files, and the
co-located render/component twin layout (ADR-0056) means the cost is paid again
in every vertical.

## Decision summary

Adopt **maud 0.27**, using its **`html!`** macro, as the render layer's markup
builder; delete `escape_html`; convert **all nine** render modules in this
cycle. Keep the trusted-HTML invariant carried by **one type**, and add two
**enumerating** static gates (ADR-0085) that can see inside macro bodies.

Full rationale: `docs/adr/drafts/web-render-html-macro.md` (numbered at ship by
`cargo xtask adr promote`, per ADR-0048).

## Resolved decisions

### D1 — Render coincidence is parse-equivalence; the CLS probe is the oracle

The reactive half never emits bytes: `rg 'to_html|render_to_string' web/src`
returns nothing, and under ADR-0040's CSR-only model a `#[component]` builds DOM
nodes rather than a string. So "byte-identical to the reactive twin" — asserted
in `topbar/markup.rs` and `avatar/markup.rs` — was never mechanically verified
and cannot be; those goldens compare a render fn to a literal a human
transcribed from reading the component.

(The premise is the _call-site_ fact above, not a Cargo feature claim.
`web/Cargo.toml:56` opens a comment declining to name `leptos/ssr` in `web`'s
own feature list, but it goes on to say `leptos_axum` hard-requires it "so
unification supplies it" — i.e. under `feature="server"` the feature _is_
enabled. That citation does not support D1 and is not used.)

What we owe the user is **no flash at CSR mount**, and that has a mechanical
oracle: `expectNoShiftAcrossMount` (`end2end/tests/layout-shift.ts:47`, #202)
holds the wasm so first paint stays the projector's, then re-measures after
`body[data-mounted]` at `tolerancePx: 0`. `timeline-cls.spec.ts` runs it over
four projector-painted routes (`:56-74`, `tolerancePx` at `:118`).

ADR-0041 is **not** amended. Its decision 2 (`0041-…:29-35`) says the delegation
renderers coincide because "both sides call the identical fn" — preserved here,
since both keep calling the same converted fn. Its decision 4 (`0041-…:46-54`,
byte-identity per URL across visitors) is the no-auth-branch CDN property —
untouched, since rendering stays deterministic.

### D2 — Accept maud's native escaping; re-pin the goldens

`escape_html` escapes `& < > " '` in all contexts. maud escapes `& < > "` in
text and `"` in attribute values, leaving `'` raw in both. So the only
divergence is the apostrophe: `O'Brien` renders `O'Brien` rather than
`O&#39;Brien`. Both parse to the identical text node, so D1's oracle is
unaffected; an apostrophe cannot move layout. Goldens are re-pinned once, in the
conversion.

Rejected: preserving today's bytes via `PreEscaped(escape_html(x))` at every
text slot — it reinstates the manual-escape discipline this issue removes and
routes ordinary text through the raw constructor.

### D3 — maud, `html!` syntax

**hypertext was chosen first and rejected on evidence.** Its appeal was syntax
parallelism with Leptos `view!` (both parse `rstml`), which would have paid off
at all nine twin sites. But hypertext validates element and attribute names at
compile time against its `hypertext_elements` registry, and three constructs
this tree actually emits are not in it:

- **SVG** — `icon/markup.rs` emits `<svg …><path d="…">`; hypertext ships HTML
  and (behind a feature) MathML only. There is no `svg` feature.
- **OpenGraph** — `app/render.rs:80-81` emits `<meta property="og:title">`;
  hypertext's `meta` defines `charset, content, http_equiv, media, name` plus
  globals, with no `property` and no RDFa extension.
- **An interpolated attribute _name_** — `app/render.rs:133,146` interpolate
  `DISCOVERY_MARKER_ATTR` as the attribute key (see D8).

The documented escape hatch — shadowing `hypertext_elements` with
`define_elements!` — means hand-maintaining an element/attribute registry, and
adding `property` to `meta` means redeclaring `meta` wholesale rather than
extending it. That is a standing maintenance cost incurred to satisfy a checker
whose guarantee we never asked for. Choosing hypertext's `maud!` syntax would
not have helped: the validation lives in the crate, not the syntax.

maud accepts arbitrary element and attribute names, so all three compile as
written. The spike found it equal to hypertext on every other axis — dual-target
compile, byte-identity on the goldens, two crates added.

**What this costs:** the `view!` parallelism is gone. A render fn's
`html! { div { … } }` reads differently from the `#[component]` beside it. That
was the whole reason to prefer hypertext, and it is given up deliberately — it
was a reading aid, never a correctness mechanism, and shared input syntax
implied nothing about output escaping, attribute order, or whitespace anyway.

### D4 — One trusted-markup type: `Markup`

`Markup` is the render layer's currency **and** its trusted-HTML carrier. A
single type, not two.

> **Why not `impl Render for RenderedHtml`:** both the trait (maud's) and the
> type (`common::render::RenderedHtml`, `common/src/render.rs:80`) are foreign
> to `web` — E0117, orphan rule. `Markup` is local to `web`, so
> `impl Render for Markup` is legal, and it keeps maud out of `common`.

- **Definition:** `pub struct Markup(maud::Markup)` in `web/src/html.rs` —
  maud's `Markup` is its `PreEscaped<String>` alias.

  > **Naming:** ours deliberately shadows maud's within `web`. `maud::Markup` is
  > **never imported**; the crate's glob-import ban (below) keeps that
  > enforceable, and `Markup` is the domain word every `web` reader should reach
  > for.

- **Traits:** `maud::Render` (so it composes inside `html!`), plus `Clone`
  (required — `home/component.rs:69` and `sidebar/component.rs:69` clone before
  `inner_html`), `Debug` and `PartialEq` (required — re-pinned goldens
  `assert_eq!` against string literals).
- **The one raw door:** `Markup::from_rendered_html(&RenderedHtml) -> Markup`,
  whose body is the crate's only `PreEscaped`, carrying an `// XSS SAFETY:`
  comment citing ADR-0079 (the value is sanitized by construction). Note
  `RenderedHtml` exposes no inherent `as_str()`, but it does implement
  `AsRef<str>` (`common/src/render.rs:107`), so the door reads through
  `as_ref()`. `common` is not modified.
- **Exits:** `as_str(&self) -> &str` and `into_string(self) -> String`. Both
  `pub`; `Markup` is re-exported from `web::app` (`pub use crate::html::Markup;`
  — it lives in the private `html` module) because `render_head`/`render_shell`
  (`web/src/app/render.rs:57,174`) are `pub` and consumed cross-crate at
  `server/src/projector/mod.rs:78-79`. The projector's use needs no gate: it
  receives a `Markup`, so trust is type-carried across that boundary.
- **No glob imports of maud.** Import `maud::{html, Render, PreEscaped}`
  narrowly; a glob would pull `maud::Markup` into scope and collide with ours.
- **`topbar`'s trusted slot takes `Markup`.** `topbar::render`'s `right`
  (`web/src/topbar/markup.rs:5`) is `&str` documented as trusted in prose today;
  it becomes `Markup`, with `Markup::empty()` for the `""` callers
  (`posts/render.rs:55,62,73`). No second wrapper type.

Because `Markup` is the only thing a render fn can return and the only thing
that composes into `html!` raw, a hand-built `String` cannot reach the output
without passing `from_rendered_html`. That is the compiler doing what a scanner
would otherwise have to.

### D8 — The one attribute maud cannot name

`app/render.rs:133,146` interpolate `DISCOVERY_MARKER_ATTR`
(`= "data-jaunder-discovery"`, `:94`) as the attribute **key**. maud requires
literal attribute names, so the const cannot be spliced there. The const is
`pub` and consumed by `csr/src/lib.rs:41` to build the selector that removes
those elements, so hardcoding the literal silently breaks a real cross-crate
drift guard.

Resolution: write the literal in the `html!` and add a unit test in
`app/render.rs` asserting `DISCOVERY_MARKER_ATTR == "data-jaunder-discovery"`,
so a change to the const fails a test rather than diverging from the markup. The
existing occurrence counts (`:200,209`) keep working unchanged.

### D5 — Two enumerating gates, token-scanned; plus one retrofit

`xtask/src/steps/rendered_html_from_trusted_check.rs:31-35` records that `syn`
does not descend into macro bodies — "the most plausible residual gap, since the
unescaped sink lives in `web`; none exists today." Converting nine render
modules into macro bodies is what makes it real.

Technique: `syn`'s `visit_macro` yields the `Macro` node; `.tokens` is a
`TokenStream` walkable through nested `Group`s. `xtask/Cargo.toml:21-22` already
provides `syn` (`visit`) and `proc-macro2` (`span-locations`). Comments are not
tokens, so prose cannot false-positive.

Both gates conform to ADR-0085: population read structurally, deny by default,
site-scoped entries with written reasons **and multiplicity**, hard failure on
unparseable input, unreadable classes stated in the module doc.

- **Raw-door gate** — `xtask/src/steps/raw_html_door_check.rs`. Population:
  every `PreEscaped` ident in the same `POLICED_ROOTS` the existing gate uses
  (`rendered_html_from_trusted_check.rs:52-60`), including inside macro token
  streams. Allowlist keyed by **enclosing fn** with multiplicity, exactly one
  entry: `from_rendered_html`, multiplicity 1 — so a _second_ door added inside
  that same fn fails rather than being absorbed. (The gate scans author-written
  macro **invocation** tokens, not expansions, so `html!`'s own internal use of
  `PreEscaped` is invisible to it and needs no exemption.)
- **Sink gate** — `xtask/src/steps/html_sink_check.rs`. Population: every
  `inner_html` **or** `set_inner_html` ident anywhere in `web/src` — not only
  inside `view!`, so a `web_sys` `set_inner_html` or a builder-API call is
  inside the population, not silently outside it. Five sites today
  (`posts/component.rs:189,204,891`, `home/component.rs:69`,
  `sidebar/component.rs:69`) in **four** enclosing fns, so **four** entries with
  multiplicities: `PostDisplay` ×2 (`posts/component.rs:155`),
  `permalink_first_paint` ×1 (`:884`), `HomePage` ×1 (`home/component.rs:15`),
  `Sidebar` ×1 (`sidebar/component.rs:53`).
- **Retrofit** — `rendered_html_from_trusted_check` descends into macro token
  streams too.
- **Registration:** `steps` is an **inline module** in `xtask/src/lib.rs:23-45`
  (there is no `xtask/src/steps/mod.rs`); each new gate is declared there and
  invoked in **both** run lists — `xtask/src/lib.rs:396-415` (`check`, the
  existing gate at `:412`) and `:436-454` (`validate`, at `:452`). A gate wired
  into one only is a silent hole.

Rejected: a gate flagging `format!` literals containing tag-like text. It
decides violations by searching for anticipated spellings (ADR-0085 principle
3); `format!("<{tag}>")`, a `push_str` chain, or `concat!` all evade it.

### D6 — Security is tested as a property, not as bytes

One contract test pushes `' " & < > </script>` through a text slot and an
attribute slot and asserts, without a golden literal and without an HTML parser:

- **Text slot:** the rendered output contains no `<` and no `&` that the payload
  contributed — concretely, rendering the hostile payload yields the same
  _count_ of `<` as rendering a benign payload of the same length, and the
  output contains no `<script`.
- **Attribute slot:** the attribute's delimiting quote does not occur inside the
  emitted value.

Stated this way it survives an escaping-_style_ change that remains safe
(`&#39;` vs `'`), which a golden literal would not. A test that cries wolf
teaches its readers to re-bless it, and one day they re-bless a real escape.

### D7 — `web/src/html.rs` is rewritten, not deleted

It loses `escape_html` and gains `Markup`. Its module doc — today entirely about
escaping (`html.rs:1-7`) — is rewritten, and must **carry forward** the
invariant it is currently the sole record of: _plain-string building only, no
leptos reactivity, so `reactive_graph` never sits on the public request path_
(the #173 escape, ADR-0040). maud preserves this: it is a compile-time macro
producing a string, with no reactive runtime.

## Acceptance criteria

**Conversion**

- A1. All nine render modules build markup with `html!`: `app/render.rs`,
  `posts/render.rs`, `timeline/render.rs`, `home/render.rs`, `icon/markup.rs`,
  `sidebar/markup.rs`, `topbar/markup.rs`, `avatar/markup.rs`,
  `taglist/markup.rs`. Mechanically: no `format!`, `write!`, or `push_str`
  remains in those nine files outside `#[cfg(test)]`, and the now-unused
  `use std::fmt::Write` imports (`sidebar/markup.rs:2`, `taglist/markup.rs:1`)
  are gone. (`write!` and `push_str` are both named because `sidebar` and
  `taglist` build with a mix of the two — `write!` at
  `sidebar/markup.rs:50,62,70` and `taglist/markup.rs:20,26`, `push_str` at
  `sidebar/markup.rs:55,68,76` and `taglist/markup.rs:32,34` — not with
  `format!`.)
- A2. `crate::html::escape_html` no longer exists; `rg 'escape_html' web/src`
  returns no hits.
- A3. Every fn in those nine modules **whose return value is HTML** returns
  `Markup`; none returns `String`. The HTML-returning set is exactly:
  `app::render_head`, `app::render_shell`, `app::render_discovery`,
  `posts::render_body`, `posts::permalink_article`, `posts::render_posts`
  (`:107`), `posts::render_timeline_page` (`:228` — its `chrome: &str` parameter
  also becomes `Markup`), `posts::render_post_article`,
  `posts::render_post_inner`, `posts::render_post_content`,
  `timeline::render_load_more`, `home::render_masthead`, `home::render_hero`,
  `icon::render`, `sidebar::render_sidebar`, `topbar::render`, `avatar::render`,
  `taglist::render`. Non-markup helpers stay `String` and are unchanged:
  `posts::format_post_time` (`posts/component.rs:164`), `app::feed_label`
  (`:156`), and `avatar::avatar_parts`' tuple (`avatar/component.rs:4`).
- A4. `maud` is added to the workspace dependency table and referenced as
  `maud.workspace = true` in `web/Cargo.toml` (matching the crate's existing
  convention); `Cargo.lock` updated; `cargo deny check` passes.

**Trust and gates**

- A5. `PreEscaped` appears exactly once in `web/src` outside `html!` expansions
  — in `Markup::from_rendered_html` — carrying an `// XSS SAFETY:` comment.
- A6. `topbar::render`'s `right` parameter is `Markup`, not `&str`.
- A7. `raw_html_door_check` exists, is invoked in **both** `xtask/src/lib.rs`
  run lists, and **fails** on a `PreEscaped` added inside an `html!` body —
  proven by a unit test over its pure `violations`-style fn, as the existing
  gate does (`rendered_html_from_trusted_check.rs:93`).
- A8. `html_sink_check` exists on the same terms, with **four** allowlist
  entries keyed by enclosing fn with multiplicity (`PostDisplay` ×2,
  `permalink_first_paint` ×1, `HomePage` ×1, `Sidebar` ×1 — five sites, four
  fns), and **fails** on a sixth `inner_html` added without an entry, on a
  second sink inside a ×1 fn, and on a `set_inner_html` outside any `view!`.
- A9. `rendered_html_from_trusted_check` descends into macro token streams,
  proven by a unit test where a `from_trusted` inside a `view!` body is detected
  — it is not detected today, so the test must fail before the change and pass
  after.
- A10. Both new gates fail hard on an unparseable file (unit-tested), and each
  module doc contains a stated unreadable-classes section. The latter is an
  **existence** check — a reviewer can confirm the statement is present, not
  that it is exhaustive (ADR-0085:166-169 asks for the statement).

**Behavior**

- A11. `expectNoShiftAcrossMount` green at `tolerancePx: 0` on all four
  projector-painted routes.
- A12. The D6 escaping-contract test exists and passes, asserting the two
  properties as stated (no golden literal, no HTML-parser dependency added).
- A13. Twin goldens re-pinned, and the "byte-identical to the reactive X"
  comments removed or corrected at these enumerated sites:
  `topbar/markup.rs:21,33`, `avatar/markup.rs:19,39`, `posts/render.rs:14,142`,
  `posts/component.rs:184`, `taglist/markup.rs:10`, `sidebar/markup.rs:41-43`,
  `icon/markup.rs:1`. **Excluded deliberately:** `app/render.rs:37`, which
  asserts the per-visitor CDN property (ADR-0041 decision 4) — a different
  claim, still true.
- A14. `web/src/html.rs`'s module doc is rewritten and restates the "no leptos
  reactivity in the render layer / #173 escape" invariant;
  `web/src/lib.rs:22-23`'s description of the module matches.
- A15. A unit test in `app/render.rs` pins `DISCOVERY_MARKER_ATTR` against the
  literal now written in the `html!` (D8), so the `csr/src/lib.rs:41` drift
  guard still fails loudly.
- A16. `cargo xtask validate` green, including the wasm32 clippy pass and the
  coverage gate.

**Separable concerns filed as issues (not done here)**

- A17. **Done** — [#778](https://github.com/jaunder-org/jaunder/issues/778)
  files the rebuild of `rendered_html_from_trusted_check`'s `ALLOWED_FNS` as
  site-scoped entries with multiplicity; it exempts by _function_ today
  (`:77-88`), an ADR-0085 principle-4 region-scoped exemption. Pre-existing and
  orthogonal to this decision.

## Risks

- **Coverage gate.** `markup.rs`/`render.rs` are host-compiled and _are_ in the
  coverage denominator (unlike wasm-only `component.rs`, ADR-0070). If `html!`
  expansion attributes generated branches to source lines, the stateless
  coverage gate (ADR-0050) could fail on lines nobody wrote. **Verified
  empirically on the first converted module before the remaining eight
  proceed.**
- **Dual-target compile.** Must build host (`feature="server"`) and wasm32
  (`feature="csr"`). The spike confirmed both; the gate re-confirms in-tree.
- **Leptos attribute interop.** `inner_html=` must accept whatever `Markup`
  exposes; if leptos requires `String`/`Oco`, the five sites call
  `.into_string()`. Confirmed at the first converted `inner_html` site.
- **Intermediate-commit compilability.** Render fns call each other across
  modules (`posts/render.rs:163` interpolates `avatar::render`; `:48` takes
  `render_masthead`; `app/render.rs:182` takes `render_sidebar`), so a module
  converted alone is a type error in an untouched caller. Each conversion commit
  must therefore carry its callers' call-site fixes — the plan sequences this.
- **Maintenance.** maud 0.27.0 shipped 2026-06-10 and is multi-maintainer, so
  the churn risk that drove the `Markup` newtype under the hypertext plan is
  much reduced. The newtype is kept regardless: it is what makes the
  trusted-HTML invariant type-carried, which was always its primary job.
