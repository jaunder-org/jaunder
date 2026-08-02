# ADR-0093: The web render layer builds markup with maud, not `format!`

- Status: accepted
- Date: 2026-08-01
- Issue: [#333](https://github.com/jaunder-org/jaunder/issues/333)

## Context

`web`'s pure render layer — the projector's half of the ADR-0041 seam and the
non-reactive twin of every ADR-0056 co-located component — builds HTML by
`format!`-ing strings and calling a hand-rolled `crate::html::escape_html` on
each interpolation. Twenty-three call sites across five files, and the rule that
keeps them safe is "remember." A forgotten call compiles, passes review as
easily as it fails it, and is an XSS hole.

Two candidates were prototyped (`render_avatar`, `render_topbar` in each):
**maud** 0.27 and **hypertext** 0.12. Both compile host and wasm32, both
reproduced the golden bytes, both add two crates. Both remove the manual escape
step and make the trusted-HTML slot explicit instead of a bare interpolation.

The prototype surfaced one apparent blocker: neither library escapes `'` in
text, where `escape_html` emits `&#39;`. Against ADR-0040's render-coincidence
requirement that looked disqualifying.

It was not, and understanding why decided the shape of everything else. **The
reactive half never emits bytes.** Nothing in `web/src` calls `to_html` or
`render_to_string`, and under ADR-0040's CSR-only model a `#[component]` builds
DOM nodes. So "byte-identical to the reactive twin" — asserted in
`topbar/markup.rs` and `avatar/markup.rs` — was never mechanically checked and
cannot be. Those goldens compare a render fn to a literal a human transcribed
from reading the component.

What the coincidence requirement actually protects is a **flash-free first
paint**, and that does have a mechanical oracle: `expectNoShiftAcrossMount`
(`end2end/tests/layout-shift.ts`, #202) holds the wasm so first paint stays the
projector's, then re-measures after mount at `tolerancePx: 0`, across the four
projector-painted routes.

The second force is ADR-0085. Moving the render layer into macro bodies collides
with a limitation the `from_trusted` gate documents about itself: `syn` does not
descend into macro invocations, which it calls "the most plausible residual gap,
since the unescaped sink lives in `web`; none exists today." Converting nine
render modules to macros is exactly what makes that gap real.

The third force decided which library, and it only appeared once we tried to
plan the conversion in detail. **hypertext was chosen first, then rejected on
evidence.** Its appeal was syntax parallelism with Leptos `view!` — both parse
`rstml`, so a render fn and the `#[component]` beside it would have read alike
at all nine ADR-0056 twin sites. But hypertext validates element and attribute
names at compile time against its `hypertext_elements` registry, and three
constructs this tree actually emits are absent from it: **SVG**
(`icon/markup.rs` emits `<svg><path>`; hypertext ships HTML and, behind a
feature, MathML — there is no `svg` feature), **OpenGraph** (`app/render.rs`
emits `<meta property="og:title">`; hypertext's `meta` has no `property` and no
RDFa extension), and an **interpolated attribute name** (`DISCOVERY_MARKER_ATTR`
used as the attribute key). The escape hatch — shadowing `hypertext_elements`
with `define_elements!` — means hand-maintaining an element/attribute registry,
and extending `meta` means redeclaring it wholesale. Choosing hypertext's
`maud!` syntax would not have helped: validation lives in the crate, not the
syntax.

maud accepts arbitrary element and attribute names, so all three compile as
written.

## Decision

**Adopt maud (`html!`) as the render layer's markup builder. Delete
`escape_html`. Keep the trusted-HTML invariant carried by types, and make the
gates that guard it able to see inside macro bodies.**

1. **Coincidence is parse-equivalence, and the CLS probe is its oracle.** Byte
   goldens are kept as cheap host-side regression detection for the render fns —
   not as evidence about the reactive twin, and their comments say so. maud's
   escaping is accepted as-is: `O'Brien` renders `O'Brien`, not `O&#39;Brien`;
   both parse to the same text node and an apostrophe cannot move layout.

   This does not amend ADR-0041. Its decision 2 byte-identity ("both sides call
   the identical fn") describes the delegation renderers and survives untouched,
   because both sides keep calling the same converted fn. Its decision 4
   byte-identity ("identical per URL for every visitor" — the no-auth-branch CDN
   property) is equally untouched, because rendering stays deterministic.

2. **maud over hypertext, on compile-time permissiveness.** The `view!`-parallel
   syntax is given up deliberately: it was a **reading aid, not a correctness
   mechanism** — shared input syntax implies nothing about output escaping,
   attribute order, or whitespace, and anyone who infers otherwise will ship a
   divergence. What decided it is that hypertext's element/attribute validation
   rejects SVG, `og:` meta properties, and interpolated attribute names, and its
   escape hatch is a hand-maintained element registry. maud renders all three as
   written.

3. **One trusted-markup type carries the invariant: `Markup`.** A crate-local
   newtype in `web/src/html.rs` over the rendered `String`, implementing maud's
   `Render` so it composes inside `html!`. It is both the render layer's
   currency and its trusted-HTML carrier — not two types. It deliberately
   shadows `maud::Markup` inside `web`, which is never imported.

   It wraps `String` rather than `maud::Markup` because `PreEscaped` implements
   neither `PartialEq` nor `Eq`, which the pinned render goldens need. The
   invariant is unchanged — the field is rendered markup, and only the three
   constructors can mint one.

   The raw door is a single constructor,
   `Markup::from_rendered_html(&RenderedHtml)`, whose body holds the crate's
   only author-written `PreEscaped` (with an `// XSS SAFETY:` comment citing
   ADR-0079, which establishes that the value is sanitized). Trusted-`&str`
   parameters — `topbar::render`'s `right` slot, previously `&str` and marked
   trusted only in prose — become `Markup`.

   The obvious-looking alternative, `impl Render for RenderedHtml`, **cannot
   compile**: trait and type are both foreign to `web` (E0117). Implementing it
   in `common` instead would drag maud into `common` and move the raw door out
   of the crate the sink gate polices.

4. **A hand-built `String` therefore cannot reach the output unescaped.** Render
   fns return `Markup`, and only `Markup` composes raw into `html!`, so the sole
   path from an untyped string to unescaped output is `from_rendered_html`. That
   is the compiler enforcing what would otherwise be a scanner's job.

   **Where that guarantee actually lives — and where it does not.** It lives in
   maud's `Render` dispatch (every other type is escaped on interpolation) and
   in the single `PreEscaped` door. It does **not** live in how convertible
   `Markup` is to a string. `Markup` therefore implements `AsRef<str>`,
   `From<Markup> for String` and `PartialEq<&str>`/`PartialEq<String>` freely:
   verified that adding even `Display` leaves `html!` emitting a nested `Markup`
   raw, because trait selection picks the `Render` impl regardless. Withholding
   those conversions would buy no safety and cost every call site — an earlier
   draft of this ADR implied otherwise and was wrong.

   `Display` is nonetheless **not** implemented, for a narrower and non-security
   reason worth stating plainly: its absence is the only mechanical brake on
   `format!("<div>{markup}</div>")` — the hand-built-HTML idiom this ADR retires
   — returning to the render layer. That is housekeeping, not a vulnerability
   boundary, and it should be re-litigated on those terms if the friction ever
   outweighs it.

5. **Two enumerating gates, scanning token streams.** `syn`'s `visit_macro`
   yields the `Macro` node and `.tokens` is a walkable `TokenStream`, so a
   token-level scan sees inside `html!` and `view!` bodies — and because
   comments are not tokens, prose cannot false-positive. A **raw-door gate**
   governs every author-written `PreEscaped` (one allowlist entry:
   `from_rendered_html`, multiplicity 1, so a second door inside that same fn
   fails rather than being absorbed); a **sink gate** governs every `inner_html`
   _and_ `set_inner_html` in `web/src` — not only those written inside a
   `view!`, so a `web_sys` or builder-API call falls inside the population
   rather than silently outside it (five sites in four enclosing fns, each entry
   stating its multiplicity and why its value is trusted). The existing
   `from_trusted` gate is retrofitted with the same descent.

   Scanning invocation tokens rather than expansions is what keeps the raw-door
   gate honest here: `html!` expands to `PreEscaped` internally, and an
   expansion-aware scan would drown in self-inflicted hits.

   Both conform to ADR-0085: population read structurally, deny by default,
   site-scoped entries with written reasons, hard failure on unparseable input,
   unreadable classes stated in the module doc.

6. **Security is tested as a property.** One contract test pushes
   `' " & < > </script>` through a text slot and an attribute slot and asserts
   the output is structurally inert: the payload contributes no `<` to the text
   slot (and no `<script`), and does not terminate the attribute it sits in.
   Expressed in forbidden characters rather than expected bytes, so it needs no
   HTML parser and survives an escaping-_style_ change that remains safe. Not a
   golden literal — a test that cries wolf teaches its readers to re-bless it,
   and one day they will re-bless a real escape.

**Rejected: a gate that flags `format!` literals containing tag-like text.** It
decides violations by searching for anticipated spellings — ADR-0085 principle 3
— and `format!("<{tag}>")`, a `push_str` chain, or `concat!` all walk past it.
The enforceable version polices the _sink_ and the _door_, both finite and
structurally readable, and leaves "ways to build a string" to the type system.

**Rejected: preserving today's exact bytes** by wrapping each text slot as
`PreEscaped(escape_html(x))`. It protects bytes whose difference is unobservable
in the DOM, at the cost of reinstating the manual-escape discipline and routing
every ordinary text slot through the raw constructor.

## Consequences

**What this commits us to.** New markup in `web` is written in `html!`, returns
`Markup`, and reaches the DOM only through an allowlisted sink. Adding an
`inner_html` or a second use of the raw door costs a reviewed allowlist entry —
the ADR-0085 friction, accepted deliberately. It also commits every render fn
and the `#[component]` beside it to reading in **different** syntaxes; that is
the priced cost of decision 2, not an oversight to be fixed later by
re-litigating the library.

**What it creates.** [#778](https://github.com/jaunder-org/jaunder/issues/778)
is done, and it went further than this paragraph anticipated. The `from_trusted`
gate exempted by _function_ — an ADR-0085 principle-4 region-scoped exemption —
and the obvious repair was to give it the multiplicity its two siblings carry.
That turned out to be the wrong key rather than a smaller version of the right
one, so all three gates moved to in-source per-site markers and the central
allowlists were deleted; see
[the marker ADR](drafts/gate-exemptions-in-source-markers.md). And one soft spot
the type system does not cover: `DISCOVERY_MARKER_ATTR` cannot be spliced as an
attribute _name_ under any compile-time markup macro, so the literal is written
in the `html!` and a unit test pins it against the const, keeping the `csr`
drift guard loud.

**What it rules out.** Hand-built HTML strings in `web`; `escape_html` as a
crate-wide primitive; and any future claim that a host-side golden proves
something about the reactive twin. Under CSR the twin has no bytes to compare
against, and a test asserting otherwise is documentation of a belief, not a
check.

**What it does not claim.** maud's escaping is not verified to match Leptos's
DOM construction byte-for-byte — that comparison does not exist. The claim is
narrower and testable: the two parse to equivalent DOM, and
`expectNoShiftAcrossMount` at `tolerancePx: 0` is what would catch us being
wrong.
