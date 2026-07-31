# Spec — `ValidatedTextarea<T>`: shared chrome + wiring for multi-line validated fields (#568)

- Issue: jaunder-org/jaunder#568
- Milestone: Web: canonical Leptos CSR convergence
- ADR: `docs/adr/0065-client-side-domain-validation.md`

## Problem

ADR-0065's `ValidatedInput<T>` (`web/src/forms/component.rs`) abstracts the
client-validated-field pattern for a single-line `<input>` only. Multi-line
fields use the ADR-0065 **direct-bind** variant, hand-repeating the same block
at every site: `prop:value`, an `on:input` that sets value _and_ error, an
`on:blur` that touches, and a touched-gated inline error node.

Four `<textarea>` sites repeat it verbatim (the paths in the issue body predate
the vertical convergence — `web/src/pages/` no longer exists):

| Site             | Location                          | Field                                        |
| ---------------- | --------------------------------- | -------------------------------------------- |
| Compact composer | `web/src/posts/component.rs:551`  | `Field::<PostSummary>::optional()` (`:471`)  |
| Full composer    | `web/src/posts/component.rs:674`  | `Field::<PostSummary>::optional()`           |
| Post editor      | `web/src/posts/component.rs:1329` | `Field::<PostSummary>::optional()` (`:1171`) |
| Profile bio      | `web/src/profile/component.rs:78` | `Field::<Bio>::optional()` (`:22`)           |

It is latent shotgun surgery: a `Field` API change touches every site, and it is
easy to wire one submit button's disable-gate and forget another — exactly the
compact-composer gap caught in #545 review.

The four sites are also **not uniform**, which is why a naïve "mirror
`ValidatedInput`" component does not fit them:

- Three posts sites use a _sibling_ `<label class="j-field-label">` +
  `<textarea id=… class="j-field-val" rows=3 placeholder=…>`; profile uses a
  bare _wrapping_ `<label>` with no classes, no rows, no placeholder.
- The compact composer's label (`:550`) has **no `for=`** while its textarea
  carries `id="compose-summary"` (`:552`) — the label/control association is
  missing. This is a live a11y defect.
- The compact composer's textarea has **no `name`** at all.
- The compact and full composers both use the literal `id="compose-summary"`
  (`:552`, `:675`). This is a duplicated _literal_, **not** a live duplicate-id
  defect: they are the two arms of one `if compact {…} else {…}` (`:506`/`:612`)
  and can never co-render.

## Decisions

### D1 — Converge on one chrome (`j-form-*`), do not parameterise markup shape

All four sites adopt `ValidatedInput`'s existing chrome: a wrapping
`<label class="j-form-field">` (`display:flex; flex-direction:column; gap:7px`,
`server/assets/jaunder.css:878-883`), a `<span class="j-form-label">`, and the
`<p class="error">` node.

Rationale: all four sites are already visually _stacked_; what differs is the
control's own box, not the structure. Presentation variation, if a site ever
needs it, belongs in a modifier class on the wrapper — not in a structural prop.
Nothing blocks convergence: no e2e selector keys on
`j-field-label`/`j-field-val` (verified across `end2end/`), so the CSS classes
are the only difference.

The precise class deltas (`jaunder.css:859-869` vs `:884-908`):

|           | `.j-field-label` → `.j-form-label`                                                               | `.j-field-val` → `.j-form-input`                                                                                                                                                                     |
| --------- | ------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| identical | **entire rule body** — `font-size:13px`, `color:var(--ink-soft)`, `font-family:var(--font-meta)` | `font-size:13.5px`, `border:1px var(--border-style) var(--line)`, `border-radius:var(--radius-sm)`                                                                                                   |
| changes   | none — the label restyle is a **visual no-op**                                                   | gains `width:100%`, `min-height:38px`, `background:var(--surface)`, `color:var(--ink)`, `font-family:inherit`, `outline:none`, `:focus{border-color:var(--accent)}`; padding `7px 10px` → `8px 10px` |

**`width:100%` is the largest visible delta**, and it lands on the two
aside-mounted composers (`posts/component.rs:674`, `:1329`), where
`.j-field-val` currently leaves the textarea at its default cols-based width.
The compact composer (`:551`) already gets `width:100%` from the descendant rule
`.j-composer-body textarea` (`jaunder.css:379`), so it is unaffected on that
axis.

**Consequence:** the wrapping `<label>` gives _implicit_ label association, so
`for=` and `id=` are not emitted and cannot drift. The compact composer's
missing-`for=` defect disappears as a side effect rather than needing its own
fix.

### D2 — Shared chrome, two thin shells (approach "(c)")

`ValidatedInput` and a new `ValidatedTextarea` would otherwise be
character-for-character identical in four of their six emitted pieces (wrapper
label, label span, help span + `aria-describedby` id derivation, error node) and
in their entire wiring closure. Only the control element differs.

Therefore:

- **`Labelled`** — a private, **non-generic** component in
  `web/src/forms/component.rs` owning the wrapper `<label class=field_class>`,
  the `<span class="j-form-label">`, the optional help span, and the
  touched-gated error node. It takes two erased signals —
  `error: Signal<Option<String>>` and `touched: Signal<bool>` — the optional
  `help` text and its `help_id`, plus `children` for the control, and performs
  the gating itself. It takes no `name`: the only thing `name` fed was the help
  span's id, which now arrives ready-made.
- **`ValidatedInput<T>`** and **`ValidatedTextarea<T>`** — the two public
  shells, each carrying only props meaningful for its control. Each passes
  `error=field.error` and one `Signal::derive(move || field.is_touched())`.

**Why erased rather than generic over `T`.** Taking the whole `Field<T>` would
be the tidier signature — one prop instead of two — and it was the first choice.
It is rejected because `Labelled` is the repo's **first generic component with
children**: all 13 existing generic-component call sites are self-closing
(`auth/component.rs:47`, `site:85`, `:94`, `backup:118`, `:128`, `invites:44`,
`:53`, `registration:91`, `:100`, `email:41`, `password_reset:27`, `:87`). A
generic tag with a close tag must match its opening generics **token-for-token**
— rstml compares them structurally and rejects a mismatch
(`rstml-0.12.1/src/node/parse.rs:241-253`), and `syn`'s `Punctuated` equality
treats `<T,>` and `<T>` as different (`syn-2.0.119/src/punctuated.rs:417-426`).
Every generic tag in this repo is leptosfmt-formatted **with** a trailing comma,
and `cargo xtask check` runs the formatter in fix mode — so a formatter pass
could unbalance a hand-matched pair after the code already compiled. The erasure
costs one `Signal::derive` line per shell and buys a construct the repo (and its
formatter) already handles everywhere.

The gate still lives in exactly one place, which is what matters — see AC2.

**`help` must be declared `#[prop(optional_no_strip)]` on `Labelled`.** Leptos
turns a plain `#[prop(optional)]` on an `Option<_>` into typed-builder
`strip_option` (`leptos_macro-0.8.17/src/component.rs:1033`), so the generated
setter takes the _inner_ type and no `Option`-accepting setter exists at all.
Both shells hold `help` as `Option<&'static str>` and forward it, so the
stripped setter would be a type error. `optional_no_strip`
(`component.rs:1003-1006`) keeps the setter taking `Option<_>`.

Rejected: adding `#[prop(optional)] rows: Option<u32>` to `ValidatedInput` as a
`<textarea>` discriminator (the issue's stated alternative). It creates a
prop-validity matrix the compiler cannot police —
`input_type`/`autocomplete`/`transform` are meaningless when `rows` is set,
`rows`/`placeholder` meaningless when it is not.

### D3 — `ValidatedTextarea<T>`'s prop surface

```rust
#[component]
pub fn ValidatedTextarea<T>(
    label: &'static str,
    name: &'static str,
    field: Field<T>,
    #[prop(default = 3)] rows: u32,
    #[prop(optional)] placeholder: Option<&'static str>,
    #[prop(default = "j-form-field")] field_class: &'static str,
    #[prop(default = "j-form-input")] class: &'static str,
    #[prop(optional)] help: Option<&'static str>,
) -> impl IntoView
where T: FromStr + 'static, T::Err: Display
```

No `id` prop: ids are not emitted (D1). No `transform`: nothing multi-line needs
live input massaging. `help` is exposed for symmetry with `ValidatedInput`,
since `Labelled` renders it anyway; no current site passes it.

**`aria-describedby`:** the attribute belongs on the _control_ in the shell,
while the help span it points at is rendered by `Labelled`. The shell derives
`format!("{name}-help")` **once** and passes it down as `Labelled`'s `help_id`
prop, so the pair is single-source and cannot drift; `Labelled` therefore needs
no `name` prop of its own.

_(Corrected at ship review. An earlier draft had both sides re-derive the id
independently, justified by "Leptos `children` is opaque so the id cannot be
handed down" — that reasoning was wrong: it describes passing a value from
`Labelled` **into** its children, whereas the id travels the other way, from the
shell **into** `Labelled` as an ordinary prop.)_

**`rows` default:** profile's bio has no `rows` today, so it renders at the
browser default (2). Under this spec it becomes 3. Intended; listed under known
visible changes.

### D4 — Adopt at all four textarea sites, plus profile's `display_name`

The three posts summaries and profile's bio adopt `ValidatedTextarea`.

Profile's `display_name` `<input>` (`web/src/profile/component.rs:57`) **also**
adopts `ValidatedInput` in this cycle. It is hand-wired today but needs nothing
`ValidatedInput` lacks
(`label="Display Name" name="display_name" field=dn_field`), and leaving it
would render the profile page half-converged — a `j-form-*` bio beside an
unstyled display-name input.

### D5 — e2e selectors move off ids and onto `name`

Since ids stop being emitted, the id-keyed selectors change to name-keyed:

- `#compose-summary` → `textarea[name="summary"]` —
  `end2end/tests/posts.spec.ts:43,69,87` and `end2end/tests/posts.ts:62`.
- `#edit-summary` → `textarea[name="summary"]` —
  `end2end/tests/posts.spec.ts:107,110,116`.

That is 7 uses across 2 files, so per the stated convention in
`end2end/tests/selectors.ts:4-7` ("the same high-frequency CSS selector strings
were literaled across many spec files… route those through `SEL`"), it gets a
**`SEL.postSummary` entry** rather than 7 inline literals.

The compact composer's textarea gains `name="summary"`, which it lacks today.
**This is required by D3's prop surface** (`name` is not optional), _not_ by the
selector swap: every current `#compose-summary` use drives `/posts/new` → the
**non-compact** branch (`posts/component.rs:612-695`), which already carries
`name="summary"` at `:676`. The compact composer renders only at `/app`
(`InlineComposer` `:765` → `cockpit/component.rs:102` → `app/component.rs:109`)
and no e2e drives its summary field at all. Adding `name` is inert for
submission either way: all composers dispatch typed `#[server]` args
(`posts/component.rs:508`, `:522`), not form-encoded bodies.

Unaffected and preserved: `textarea[name="bio"]` and
`input[name="display_name"]` (`end2end/tests/profile.spec.ts:9-10`),
`SEL.error = ".error"` (`end2end/tests/selectors.ts:18`),
`SEL.postBody = 'textarea[name="body"]'` (`:14` — the post body textarea is
`ComposerFields`, not a `Field`-validated control, and is out of scope).

### D6 — Caller-owned spacing wrappers stay (posts)

The three posts sites sit inside `<div style="margin-top:10px">` wrappers
(`posts/component.rs:549,670,1325`) that supply their spacing; `.j-form-field`
carries no margin. Those wrappers remain at the call sites.

### D7 — Profile adopts the standard card treatment

Profile's fields sit directly in `<div class="j-page">`
(`profile/component.rs:27`), and `.j-page` is padding only — no flex, no gap
(`jaunder.css:189-191`). Today the fields are bare inline label+control pairs
whose rhythm comes from line boxes; under D1 they become two flex-column blocks
that would sit flush against each other, against `<p>"Username: …"</p>` (`:54`),
and against the Update button. Convergence therefore forces a container
decision.

Profile adopts the card shape every other settings form in the app already uses
— site settings (`site/component.rs:77-116`), backup settings
(`backup/component.rs:107-165`), and login (`auth/component.rs:42-66`):

```
<div class="j-card">
    <div class="j-card-head"><div>
        <h2>"Profile"</h2>
        <div class="j-sub">"Your display name and bio."</div>
    </div></div>
    <div class="j-form-body">
        <p>"Username: " {username}</p>            // unchanged, first child
        <ValidatedInput<DisplayName> … />
        <ValidatedTextarea<Bio> … />
    </div>
    <div class="j-form-actions">
        <button …>"Update Profile"</button>
    </div>
</div>
```

The **generic** `j-form-body`/`j-form-actions` (`jaunder.css:872-877`,
`:909-916`) are used, not bespoke per-vertical classes: site and backup only
define their own because they need bespoke layout (backup's body is a two-column
grid), and profile needs none. Retaining the page's existing
`<Topbar title="Profile" sub="Your details" />` (`:25`) alongside a card head
that also names the page matches site (`:15` + `:80`) and backup (`:15`).

The `Username:` line stays as-is and becomes the first child of the body: it is
data, not chrome, whereas every other card's `j-sub` is a static description.

**`DefaultPostFormatControl` (`:138-194`) also becomes its own card**, with a
`j-card-head` ("Default Post Format"), its `<select>` in a `j-form-body`, and
its existing Save button in a `j-form-actions`. This is a deliberate scope
addition: the control is not a `Field<T>` site and is otherwise untouched by
this issue, but leaving it as loose `j-field-label`/`j-field-val` markup
directly beneath a card would reproduce exactly the half-converged-page wart
that D4 exists to avoid. Its behaviour, ids, and the
`select#default-post-format` e2e selector (`profile.spec.ts:124`) are unchanged.

### D8 — Amend ADR-0065 and the `forms` module doc

ADR-0065 goes stale in two places and both are corrected in this cycle
(doc-only):

- **Coverage boundary (`:72-80`).** The bullet is explicitly headed "(ADR-0056,
  superseding 0055 — no `target_arch` gating)" and claims "`<ValidatedInput<T>>`
  is a `#[component]`, host-compiling as dead-but-exempt". **ADR-0056 is itself
  superseded by ADR-0070** ("web verticals split host/wasm at the file level",
  `docs/adr/0056-…:3-4`), and the code follows ADR-0070:
  `web/src/forms/mod.rs:9-10` gates `mod component` on `target_arch = "wasm32"`,
  so the components never host-compile. The ADR already contradicts itself —
  `:41` says "wasm-only `<ValidatedInput<T>>`". Re-point the bullet at ADR-0070
  and correct the claim.
- **Rendering: component or direct bind (`:57-64`).** It names "the post
  compose/edit forms" as the canonical direct-bind example — precisely the sites
  this issue converts. Replace the example, keeping direct-bind documented as
  still-valid for the sites #450 covers.

`web/src/forms/mod.rs:6-7` ("…and the `ValidatedInput` widget") is updated to
name both widgets.

### D9 — Out of scope

- The **six remaining hand-wired `<input>` sites** — posts `slug_override` ×2
  (`posts/component.rs:658`, `:1295`), audiences name ×2
  (`audiences/component.rs:176`, `:246`), backup destination
  (`backup/component.rs:63`), sessions label (`sessions/component.rs:94`). These
  need chrome `ValidatedInput` still cannot express (e.g. backup's placeholder
  and bespoke classes — see the comment at `backup/component.rs:47-49`). They
  remain **#450**'s job, and #450 is unchanged by this work. (Profile's
  `display_name` was the seventh such site; D4 converts it, because it alone
  needs nothing `ValidatedInput` lacks.)
- Adding a `placeholder` prop to `ValidatedInput` (that is #450's option 2).
- Any change to `ComposerFields` / the post _body_ textarea
  (`posts/component.rs:107-119`), which binds a plain `RwSignal<String>`
  (`:467`), not a `Field<T>`.

## Acceptance criteria

Each is observable — a reviewer can tell delivered from not.

**AC1.** `web/src/forms/` exports a `ValidatedTextarea` component with the D3
prop surface, gated `#[cfg(target_arch = "wasm32")]` alongside `ValidatedInput`,
and re-exported from `web/src/forms/mod.rs`.

**AC2.** A private `Labelled` component in `web/src/forms/component.rs` renders
the wrapper label, label span, optional help span, and touched-gated error node;
**both** `ValidatedInput` and `ValidatedTextarea` render their chrome through
it. Neither shell contains its own copy of the `<span class="j-form-label">`,
the help span, or the `<p class="error">` node, and neither writes its own
`is_touched()` gate.

**AC3.**
`rg 'error_for\(&v\)' web/src/posts/component.rs web/src/profile/component.rs`
returns **exactly two hits, both `slug_field`**. (It returns seven today:
profile `64`, `84`; posts `560`, `658`, `684`, `1295`, `1339`. Line numbers will
shift; the count and the identity of the survivors are the criterion.)

**AC4.** No `<textarea>` element bound to a `Field<T>` remains in `web/src`:
every `Field`-validated multi-line control is a `<ValidatedTextarea<_>>` call.

**AC5.** No `id="compose-summary"` / `id="edit-summary"` remains in `web/src`,
and no `for=` attribute is emitted for these fields — label association is
implicit via the wrapping `<label>`.

**AC6.** The compact composer's summary textarea carries `name="summary"`.

**AC7.** `end2end/tests/selectors.ts` gains a `postSummary` entry resolving to
`textarea[name="summary"]`, and no `#compose-summary` or `#edit-summary` string
remains under `end2end/`.

**AC8.** Profile renders two `j-card`s (D7) — the profile form and
`DefaultPostFormatControl` — each with a `j-card-head`, a `j-form-body`, and a
`j-form-actions`; and profile's `display_name` is a
`<ValidatedInput<DisplayName>>` call (D4). No `j-field-label` / `j-field-val`
string remains in `web/src/profile/component.rs`.

**AC9.** Behaviour is unchanged and proven by the **existing** e2e tests passing
unmodified except for the D5 selector swap:

- `posts.spec.ts:36` "authenticated user can create a post with a summary"
- `posts.spec.ts:63` "over-long post summary shows an inline error and gates
  submit" (touched-gated message + disable-until-valid)
- `posts.spec.ts:81` "clearing a post summary on edit persists as empty"
- `profile.spec.ts:82` "profile update persists a valid bio"
- `profile.spec.ts:104` "over-long bio shows an inline error and gates submit"
- `profile.spec.ts:151` "clearing the bio persists as empty"
- `profile.spec.ts:13` "profile update persists a valid display name"
- `profile.spec.ts:34` "over-long display name shows an inline error and gates
  submit"
- `profile.spec.ts:53` "clearing the display name persists as empty"
- `profile.spec.ts:127` "default post format round-trips through the typed
  dispatch"

The three bio tests are the ones that re-prove `ValidatedTextarea`'s own wiring
(touched-gated `.error` + disable-until-valid); the display-name three cover D4;
the format test covers `DefaultPostFormatControl`'s re-cardings (D7), whose
`select#default-post-format` selector (`profile.spec.ts:124`) must survive
unchanged.

**AC10.** ADR-0065's single-validation-source rule holds: the new component
routes validity through `Field::error_for` / `field_error::<T>`; no validation
rule is re-implemented client-side.

**AC11.** ADR-0065's coverage-boundary and direct-bind bullets are amended per
D8, and `web/src/forms/mod.rs`'s module doc names both widgets.

**AC12.** `cargo xtask validate` green. This is the whole verification surface:
the forms components are wasm-only and never host-compile, so there is no host
test for them. The gate's wasm-target clippy step
(`xtask/src/steps/static_checks.rs:228-247` —
`clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations`)
is what catches wasm-only breakage, and the e2e matrix is what catches
behavioural regressions.

## Known visible changes

All intended, none a regression:

- **The two aside-mounted posts summary fields gain `width:100%`** (D1) — the
  largest visual delta. They also gain a background, a focus ring,
  `min-height:38px`, and 1px more vertical padding.
- **The profile page is restyled into two cards** (D7) — bordered, titled panels
  with a button footer, matching site and backup settings. Its bio and display
  name gain form chrome they have none of today. This is the largest change in
  the cycle and is a deliberate scope addition, not a side effect.
- **Profile's bio grows from the browser-default 2 rows to 3** (D3).
- **`DefaultPostFormatControl`'s `<select>` gains `j-form-input`** in place of
  `j-field-val` (D7) — `width:100%`, `min-height:38px`, a background, and a
  focus border (`jaunder.css:894-908` vs `:864-869`).
- The label restyle (`j-field-label` → `j-form-label`) is a **no-op** — the two
  rule bodies are identical.
