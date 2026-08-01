# maud Render Layer Implementation Plan (issue #333)

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `format!`/`write!`/`push_str` + `escape_html` in `web`'s nine
render modules with maud's `html!`, carrying trusted HTML in one type and
enforcing the raw door and the DOM sink with enumerating static gates.

**Architecture:** A crate-local `Markup` newtype over `maud::Markup` becomes the
render layer's currency and its only trusted-HTML carrier. Render fns return
`Markup`; `Markup::from_rendered_html` holds the crate's single author-written
`PreEscaped`. Two new xtask gates scan macro **token streams** so
`html!`/`view!` bodies are visible.

**Tech Stack:** Rust, maud 0.27 (`html!`), leptos CSR, `syn` + `proc-macro2`
(xtask gates), `cargo nextest`, Playwright (e2e).

**Spec:** `docs/superpowers/specs/2026-08-01-issue-333-html-macro-adr.md` —
referenced by decision (D1–D8) and criterion (A1–A17); not restated here. **ADR
draft:** `docs/adr/drafts/web-render-html-macro.md`.

## Global Constraints

- maud joins the **workspace** dependency table; `web/Cargo.toml` references it
  as `maud.workspace = true` (crate convention). (A4)
- Dual-target: host under `feature="server"`, wasm32 under `feature="csr"`.
- **Never glob-import maud.** Our `Markup` deliberately shadows `maud::Markup`;
  `use maud::*` would collide. Import `maud::{html, Render, PreEscaped}`
  narrowly.
- Author-written `PreEscaped` may appear **exactly once** in `web/src`, in
  `Markup::from_rendered_html`, with an `// XSS SAFETY:` comment. (A5)
- No `Co-Authored-By` trailer on any commit.
- Run `cargo xtask check` before each commit so the pre-commit gate passes clean
  (**jaunder-commit**). In this worktree:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-333-html-macro-adr -- cargo xtask check`
- `steps` is an **inline module** at `xtask/src/lib.rs:23-45` — there is no
  `xtask/src/steps/mod.rs`. Declare new gates there and invoke them in **both**
  run lists: `xtask/src/lib.rs:396-415` (`check`) and `:436-454` (`validate`).

### Conversion invariant — every commit compiles

Render fns call each other across modules, and `Markup` has no `Display`, so
converting a module alone is a type error in an untouched caller. **Each
conversion task fixes its own callers in the same commit.** The dependency
edges:

| Converting                   | Callers to fix in the same commit                                  |
| ---------------------------- | ------------------------------------------------------------------ |
| `avatar::render`             | `posts/render.rs:163`                                              |
| `icon::render`               | `sidebar/markup.rs:53,65`                                          |
| `home::render_masthead`      | `posts/render.rs:48` (and `render_timeline_page`'s `chrome` param) |
| `timeline::render_load_more` | `posts/render.rs:241`                                              |
| `taglist::render`            | `posts/render.rs:217`                                              |
| `sidebar::render_sidebar`    | `app/render.rs:182`                                                |
| `posts::render_body`         | `app/render.rs:183`                                                |

Tasks are ordered so each caller-fix is a one-line `.into_string()` /
`.as_str()` adaptation until that module's own conversion task lands.

---

## Review header

**Scope — in:** all nine render modules; `web/src/html.rs`; the five
`inner_html` sites; `server/src/projector/mod.rs`'s consumption of
`render_head`/`render_shell`; two new xtask gates + one retrofit; the ADR draft.

**Scope — out:** the `ALLOWED_FNS` site-scoping rebuild (filed in Task 1); any
change to `common::render::RenderedHtml` (the door reads it via `Display`);
anything in `csr` beyond leaving `DISCOVERY_MARKER_ATTR` intact.

**Tasks:**

1. File the separable concern (`ALLOWED_FNS` region-scoping) as an issue
2. Foundation — maud dep, `Markup`, escaping-contract test
3. Convert `avatar` — the risk probe (coverage, dual-target, golden re-pin)
4. Raw-door gate `raw_html_door_check`
5. Sink gate `html_sink_check`
6. Retrofit `rendered_html_from_trusted_check` to descend into macro tokens
7. Convert `icon` + `timeline` + `home`
8. Convert `topbar` + `taglist` — `right: Markup`
9. Convert `sidebar`
10. Convert `posts/render.rs` + its `inner_html` callers
11. Convert `app/render.rs` + the projector boundary
12. Delete `escape_html`; rewrite the `html.rs` module doc
13. Full `validate` + e2e CLS confirmation

**Key risks/decisions:** Task 3 is a **go/no-go probe** — it runs the coverage
gate before eight more modules are converted, because `markup.rs` is
host-compiled and in the denominator (ADR-0070) and `html!` expansion could
attribute uncovered branches to source lines. It also settles whether leptos
`inner_html=` takes `Markup` or needs `.into_string()`, which Tasks 7–11 depend
on.

---

### Task 1: File the separable concern

**Files:** none (tracker only).

**Interfaces:** Produces: an issue number referenced by the ADR's Consequences.

- [x] **Step 1: File the issue** via **jaunder-issues** → **#778**

Title: `Rebuild rendered-html-from-trusted's ALLOWED_FNS as site-scoped entries`

Body: `xtask/src/steps/rendered_html_from_trusted_check.rs:77-88` allowlists by
_enclosing function name_, so a second `from_trusted` added inside an
already-allowed fn passes silently — an ADR-0085 principle-4 region-scoped
exemption (`docs/adr/0085-static-type-safety-gates-enumerate.md:60-65`), the
same defect ADR-0085 records against `sqlx-newtype-bind` in #716. Fix: key
entries by site with a stated multiplicity, as #333's two new gates do. Label
`tooling`; milestone 11; note it was split out of #333.

- [x] **Step 2: Reference it** in the ADR draft's Consequences ("filed
      separately") — replace with the issue number.

> The draft pen is gitignored, so there may be nothing to stage. If `git add`
> reports nothing, skip the commit and carry the reference forward.

---

### Task 2: Foundation — maud dependency, `Markup`, escaping contract

> **[x] DONE — landed together with Task 3.** Two corrections found in
> execution, both of the "every commit must pass the gate" class:
>
> 1. **Task 2 is not commit-viable alone.** With no production caller, `Markup`
>    is dead code on the wasm target (the test module isn't compiled there), and
>    `wasm-clippy` denies warnings. A standalone `cargo clippy` passes because
>    it does _not_ deny — do not trust it as the dual-target proof. Tasks 2 and
>    3 are therefore one commit.
> 2. **`empty()` and `from_rendered_html()` are deferred to their first
>    callers** (Tasks 8 and 10) for the same reason. Their tests move with them.
>    This is better than it sounds: adding the raw door now costs its
>    `raw-html-door` allowlist entry in the _same_ commit, which is exactly
>    ADR-0085's friction.
>
> Also: `RenderedHtml` implements `AsRef<str>` (`common/src/render.rs:107`), so
> the door reads via `as_ref()`, not `to_string()`. And `maud::PreEscaped`
> implements neither `PartialEq` nor `Eq`, so `Markup` wraps the rendered
> `String` rather than `maud::Markup`.

**Files:**

- Modify: `Cargo.toml` (workspace dependency table, `[workspace.dependencies]`
  at `:27`)
- Modify: `web/Cargo.toml`
- Modify: `web/src/html.rs` — add `Markup`; **keep `escape_html`** until Task 12
- Test: in-file `#[cfg(test)]` in `web/src/html.rs`

**Interfaces:**

- Consumes: `common::render::RenderedHtml` (`common/src/render.rs:80`) via its
  `AsRef<str>` impl (`:107`). `common` is not modified.
- Produces — every later task depends on these exact names:

```rust
pub struct Markup(String);   // the rendered markup; minted only by the ctors below

impl Markup {
    /// Wrap a rendered `html!` fragment: `Markup::new(html! { … })`.
    pub(crate) fn new(markup: maud::Markup) -> Self;
    /// The empty fragment — for absent optional slots.
    pub fn empty() -> Self;
    /// The crate's ONE raw door (A5).
    pub fn from_rendered_html(html: &common::render::RenderedHtml) -> Self;
    pub fn as_str(&self) -> &str;
    pub fn into_string(self) -> String;
}

impl maud::Render for Markup { fn render_to(&self, buffer: &mut String); }
// derives: Clone, Debug, PartialEq, Eq
```

`Clone` is required by `home/component.rs:69` and `sidebar/component.rs:69`;
`Debug`/`PartialEq` by the re-pinned `assert_eq!` goldens. `Markup` is `pub`,
and Task 11 adds `pub use crate::html::Markup;` to `web/src/app/mod.rs` (it
lives in the private `html` module declared at `web/src/lib.rs:24`).

- [ ] **Step 1: Add the dependency**

Workspace `Cargo.toml`: `maud = "0.27"`. `web/Cargo.toml`:
`maud.workspace = true`. No non-default features.

- [ ] **Step 2: Write the failing tests** in `web/src/html.rs`

```rust
#[cfg(test)]
mod markup_tests {
    use super::Markup;
    use common::render::RenderedHtml;
    use maud::html;

    /// D6 text-slot property: a hostile payload contributes no `<` of its own and
    /// cannot open an element. Stated in forbidden characters, not expected bytes,
    /// so a safe change in escaping *style* does not fail this test.
    #[test]
    fn hostile_text_payload_contributes_no_markup() {
        let hostile = r#"' " & < > </script>"#;
        let benign = "aaaaaaaaaaaaaaaaaaa"; // same length, no metacharacters
        let render = |s: &str| Markup::new(html! { p { (s) } }).into_string();

        let hostile_out = render(hostile);
        let benign_out = render(benign);

        assert_eq!(
            hostile_out.matches('<').count(),
            benign_out.matches('<').count(),
            "payload contributed an angle bracket: {hostile_out}"
        );
        assert!(!hostile_out.contains("<script"), "{hostile_out}");
        assert!(!hostile_out.contains("</script"), "{hostile_out}");
    }

    /// D6 attribute-slot property: the payload cannot terminate the attribute it
    /// sits in, so it cannot introduce a sibling attribute.
    #[test]
    fn hostile_attribute_payload_cannot_terminate_the_attribute() {
        let hostile = r#"x" onerror="alert(1)"#;
        let out = Markup::new(html! { img alt=(hostile); }).into_string();
        let value = out
            .split_once("alt=\"")
            .expect("alt attribute present")
            .1
            .split_once('"')
            .expect("attribute terminator present")
            .0;
        assert!(!value.contains('"'), "raw quote survived in {out}");
        assert!(!out.contains("onerror="), "attribute broke out: {out}");
    }

    /// The raw door emits its value unescaped — that is its whole purpose.
    #[test]
    fn from_rendered_html_emits_unescaped() {
        let trusted = RenderedHtml::from_trusted("<p>Hi <em>there</em></p>");
        assert_eq!(
            Markup::from_rendered_html(&trusted).as_str(),
            "<p>Hi <em>there</em></p>"
        );
    }

    /// ...and composes raw when interpolated into a surrounding fragment.
    #[test]
    fn from_rendered_html_composes_raw_inside_html_macro() {
        let trusted = RenderedHtml::from_trusted("<em>x</em>");
        let inner = Markup::from_rendered_html(&trusted);
        assert_eq!(
            Markup::new(html! { div { (inner) } }).into_string(),
            "<div><em>x</em></div>"
        );
    }

    #[test]
    fn empty_renders_nothing() {
        assert_eq!(Markup::empty().as_str(), "");
    }

    #[test]
    fn markup_is_cloneable_and_comparable() {
        let m = Markup::new(html! { b { "x" } });
        assert_eq!(m.clone(), m);
    }
}
```

- [ ] **Step 3: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-333-html-macro-adr -- cargo nextest run -p web markup_tests`
Expected: FAIL — `Markup` not defined.

- [ ] **Step 4: Implement `Markup`**

To the signatures above. Two points the tests can't pin:

- `from_rendered_html`'s body is the single raw door and must carry:
  `// XSS SAFETY: RenderedHtml's invariant is established by sanitization (ADR-0079); this only inherits it.`
  It reads the value via `Display` (`html.to_string()`), since `RenderedHtml`
  has no `as_str()`.
- `impl Render for Markup` pushes `self.0.0` (the already-escaped string) into
  the buffer verbatim — a `Markup` is by definition already rendered.

- [ ] **Step 5: Run the tests, verify they pass** — Expected: PASS (6 tests).

- [ ] **Step 6: Confirm dual-target compile**

Run:
`devtool run --cwd <worktree> -- cargo clippy -p web --features csr --target wasm32-unknown-unknown`
Expected: exit 0. First in-tree proof maud builds for wasm.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock web/Cargo.toml web/src/html.rs
git commit -m "feat(web): add maud and the Markup trusted-markup newtype"
```

---

### Task 3: Convert `avatar` — the risk probe

> **[x] DONE — RISK PROBE PASSED.** `cargo xtask check` green: `wasm-clippy` ok
> and coverage **clean** — 23787 executable lines, 0 failures, 0 guard
> violations, 0 CRAP over threshold. `html!` expansion does **not** attribute
> uncovered branches to source lines, so the remaining eight conversions are
> cleared to proceed.
>
> Second finding, in favour of D2: the avatar golden needed **no re-pin** — maud
> reproduced the hand-written `format!` bytes exactly for this element. The
> re-pinning D2 budgets for may turn out to be narrower than expected; later
> tasks should still expect it where whitespace or attribute order is involved.

**Do this before any other conversion.** It is the go/no-go for the plan's two
unretired risks.

**Files:**

- Modify: `web/src/avatar/markup.rs`
- Modify (caller): `web/src/posts/render.rs:163` — `avatar::render(...)` is
  interpolated into a `format!`; adapt with `.into_string()` until Task 10
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Produces: `pub(crate) fn render(name: &str, size: u32) -> Markup` (was
  `-> String`). `avatar_parts` unchanged (`-> (String, u32)`, a non-markup
  helper — A3).

- [ ] **Step 1: Change the signature first, so the red is real**

Edit only the signature to `-> Markup` and leave the `format!` body. This is
what makes Step 3 a genuine failure rather than a test that quietly still passes
— `assert_eq!(html.as_str(), …)` against a `String` would have compiled and
passed.

- [ ] **Step 2: Re-pin the golden and correct its comments**

In `avatar_matches_reactive_component_markup` (`:38-49`), replace the comment
_"Must stay byte-identical to the reactive `Avatar` for size 38"_ and assert on
`.as_str()`. Also correct the doc comment at `:19-20` (A13 names `:19` and
`:39`):

```rust
    #[test]
    fn avatar_markup_is_stable() {
        // Renderer regression pin, not a claim about the reactive `Avatar`: under
        // CSR the component builds DOM nodes and emits no bytes to compare against.
        // Coincidence is proven by `expectNoShiftAcrossMount` (end2end).
        let (initials, hue) = avatar_parts("Mara Ek");
        assert_eq!(initials, "ME");
        assert_eq!(
            render("Mara Ek", 38).as_str(),
            format!(
                "<div class=\"j-av\" style=\"width:38px;height:38px;background:oklch(0.58 0.07 {hue});font-size:14px\">ME</div>"
            )
        );
    }
```

- [ ] **Step 3: Run it, verify it fails**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web avatar` Expected:
FAIL — the body returns `String` where `Markup` is declared.

- [ ] **Step 4: Convert the body to `html!` and fix the caller**

Body builds the same element with `html!`, wrapped in `Markup::new`. The
`font_size` integer arithmetic (`:24-26`) and its comment are unchanged — not
markup. Keep `#[must_use]`. At `posts/render.rs:163`, append `.into_string()`.

Re-pin the golden literal to maud's actual output if attribute spacing differs —
that is the expected one-time change (D2). Do **not** contort the `html!` to
reproduce the old bytes.

- [ ] **Step 5: Run it, verify it passes** — Expected: PASS (8 tests).

- [ ] **Step 6: RISK PROBE — run the coverage gate**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS,
including the Nix coverage check.

**If coverage fails on `html!`-expanded lines, STOP.** Do not convert further
modules. Read `.xtask/last-result.json`, record the failure on #333, and return
to the user — the spec's first risk has materialized and the plan needs re-work.

- [ ] **Step 7: Commit**

```bash
git add web/src/avatar/markup.rs web/src/posts/render.rs
git commit -m "refactor(web): build the avatar chip with maud"
```

---

### Task 4: Raw-door gate

**Files:**

- Create: `xtask/src/steps/raw_html_door_check.rs`
- Modify: `xtask/src/lib.rs` — declare in the inline `steps` module (`:23-45`)
  and invoke in **both** run lists (`:396-415`, `:436-454`), beside
  `rendered_html_from_trusted_check`
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Produces: `pub fn run(result: &mut CommandResult)`;
  `fn violations(source: &str) -> Result<Vec<(usize, String)>, String>`.

**Design (spec D5):** population = every `PreEscaped` ident under the same
`POLICED_ROOTS` as `rendered_html_from_trusted_check.rs:52-60`, **including
inside macro token streams**. Walk `syn::Macro`'s `.tokens` recursively through
`Group`s. Allowlist keyed by enclosing fn **with multiplicity** — one entry:
`from_rendered_html`, multiplicity 1. Because the scan sees author-written
_invocation_ tokens and not expansions, `html!`'s internal `PreEscaped` is
invisible and needs no exemption.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::violations;

    #[test]
    fn the_allowed_door_at_its_declared_multiplicity_passes() {
        let src = r#"
            impl Markup {
                pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                    // XSS SAFETY: inherited from sanitization.
                    Self(PreEscaped(html.to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    /// ADR-0085 principle 4: the entry is scoped to a site with a multiplicity, so
    /// a SECOND door inside the same allowed fn is a violation, not absorbed.
    #[test]
    fn a_second_door_inside_the_allowed_fn_is_a_violation() {
        let src = r#"
            impl Markup {
                pub fn from_rendered_html(html: &RenderedHtml) -> Self {
                    let _ = PreEscaped("<b>".to_string());
                    Self(PreEscaped(html.to_string()))
                }
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    /// The whole reason this gate exists: the render layer is macro bodies now.
    #[test]
    fn a_door_inside_an_html_macro_body_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { (PreEscaped(s)) } })
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "render_thing");
    }

    #[test]
    fn a_door_nested_deeper_in_macro_groups_is_detected() {
        let src = r#"
            fn render_thing(s: &str) -> Markup {
                Markup::new(html! { div { @if true { (PreEscaped(s)) } } })
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_mention_in_a_comment_is_not_a_token_and_does_not_trip() {
        let src = r#"
            /// See `PreEscaped` for the raw door.
            fn render_thing() -> Markup { Markup::empty() }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn unparseable_source_is_a_hard_error() {
        assert!(violations("fn broken( {").is_err());
    }
}
```

- [ ] **Step 2: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p xtask raw_html_door`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the scanner**

Every branch is pinned by a test above. Write the body to
`fn violations(source: &str) -> Result<Vec<(usize, String)>, String>`.

Two things the tests can't pin, which must be written:

- **Module doc** stating unreadable classes (A10, ADR-0085:166-169): at minimum
  a `use … as` rename evades ident matching, and a door reached through a
  re-exported alias is invisible.
- `run()` mirrors `rendered_html_from_trusted_check::run` — same
  `POLICED_ROOTS`, same `#[cfg(test)]`/`#[test]` exemption, hard failure on a
  missing scan root.

- [ ] **Step 4: Run them, verify they pass** — Expected: PASS (6 tests).

- [ ] **Step 5: Wire into both run lists and run for real**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS — one
door, in `from_rendered_html`.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/raw_html_door_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): gate the raw-HTML door, scanning macro token streams"
```

---

### Task 5: Sink gate

**Files:**

- Create: `xtask/src/steps/html_sink_check.rs`
- Modify: `xtask/src/lib.rs` (inline `steps` module + **both** run lists)
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Produces: `pub fn run(result: &mut CommandResult)`;
  `fn violations(source: &str) -> Result<Vec<(usize, String)>, String>`.

**Design (spec D5):** population = every `inner_html` **or** `set_inner_html`
ident anywhere under `web/src` — deliberately **not** restricted to `view!`
bodies. Five sites in **four** enclosing fns, so four entries with
multiplicities:

| Entry                                            | Multiplicity | Sites          |
| ------------------------------------------------ | ------------ | -------------- |
| `PostDisplay` (`web/src/posts/component.rs:155`) | 2            | `:189`, `:204` |
| `permalink_first_paint` (`:884`)                 | 1            | `:891`         |
| `HomePage` (`web/src/home/component.rs:15`)      | 1            | `:69`          |
| `Sidebar` (`web/src/sidebar/component.rs:53`)    | 1            | `:69`          |

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::violations;

    /// `PostDisplay` legitimately holds TWO sinks; its entry says so.
    #[test]
    fn an_allowlisted_fn_at_its_declared_multiplicity_passes() {
        let src = r#"
            fn PostDisplay(view: PostView) -> AnyView {
                if a {
                    view! { <article inner_html=inner></article> }.into_any()
                } else {
                    view! { <div inner_html=inner_content></div> }.into_any()
                }
            }
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn an_unlisted_sink_is_a_violation() {
        let src = r#"
            fn sneaky(html: String) -> AnyView {
                view! { <div inner_html=html></div> }.into_any()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].1, "sneaky");
    }

    /// The population is the sink, not the syntax that reaches it: a `web_sys`
    /// call outside any `view!` must NOT self-exempt (ADR-0085 principle 3).
    #[test]
    fn set_inner_html_outside_a_macro_is_in_the_population() {
        let src = r#"
            fn direct(el: &web_sys::Element, html: &str) {
                el.set_inner_html(html);
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn exceeding_a_declared_multiplicity_is_a_violation() {
        let src = r#"
            fn Sidebar() -> AnyView {
                view! {
                    <div inner_html=anon_html.clone()></div>
                    <aside inner_html=anon_html.clone()></aside>
                }.into_any()
            }
        "#;
        assert_eq!(violations(src).unwrap().len(), 1);
    }

    #[test]
    fn a_comment_mentioning_inner_html_does_not_trip() {
        let src = r#"
            /// Injected via `inner_html` so the paint coincides.
            fn harmless() {}
        "#;
        assert_eq!(violations(src).unwrap(), vec![]);
    }

    #[test]
    fn unparseable_source_is_a_hard_error() {
        assert!(violations("fn broken( {").is_err());
    }
}
```

- [ ] **Step 2: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p xtask html_sink`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement the scanner**

Same shape as Task 4; scan root is `web/src` only. Each of the four entries
carries a written reason naming why its value is trusted (it is a `Markup`, or a
`RenderedHtml`). Module doc states unreadable classes (A10) — at minimum a sink
reached through a helper taking the element and the string as parameters, which
`syn` cannot attribute.

- [ ] **Step 4: Run them, verify they pass** — Expected: PASS (6 tests).

- [ ] **Step 5: Wire into both run lists and run for real**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS — five
sites, four entries, multiplicities matching.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/html_sink_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): enumerate and gate the inner_html sinks"
```

---

### Task 6: Retrofit `rendered_html_from_trusted_check`

**Files:**

- Modify: `xtask/src/steps/rendered_html_from_trusted_check.rs` (the `Scanner`
  visitor; the module doc at `:31-41`)
- Test: in-file `#[cfg(test)]`

**Interfaces:** unchanged public shape (`run`, `violations`).

- [ ] **Step 1: Write the failing test**

```rust
    /// #333: the render layer is macro bodies now, so the limitation this gate
    /// documented ("syn does not descend into macro bodies") is no longer
    /// acceptable. A `from_trusted` inside a `view!` must be seen.
    #[test]
    fn from_trusted_inside_a_macro_body_is_detected() {
        let src = r#"
            fn sneaky(s: &str) -> AnyView {
                view! { <div inner_html=RenderedHtml::from_trusted(s).to_string()></div> }.into_any()
            }
        "#;
        let hits = violations(src).unwrap();
        assert_eq!(hits.len(), 1, "macro body must be scanned: {hits:?}");
        assert_eq!(hits[0].1, "sneaky");
    }
```

- [ ] **Step 2: Run it, verify it fails**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p xtask rendered_html_from_trusted`
Expected: FAIL — `hits.len()` is 0. **This failure is the point**: it
demonstrates the gap the spec claims exists (A9).

- [ ] **Step 3: Add macro-token descent**

Extend `Scanner` to visit `syn::Macro` and walk `.tokens` through nested
`Group`s, applying the same `EXEMPT_QUALIFIERS` and test-exemption rules. Every
existing test must still pass.

- [ ] **Step 4: Update the module doc**

Rewrite the limitation at `:31-41`: macro bodies are **no longer** an accepted
limitation. Keep the honest remaining limits (`use … as` rename; a same-named
`from_trusted` on an unrelated type) and cite #333.

- [ ] **Step 5: Run the module's whole test set** — Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/rendered_html_from_trusted_check.rs
git commit -m "fix(xtask): scan macro bodies for the from_trusted door"
```

---

### Task 7: Convert `icon` + `timeline` + `home`

**Files:**

- Modify: `web/src/icon/markup.rs`, `web/src/timeline/render.rs`,
  `web/src/home/render.rs`
- Modify (callers, same commit): `web/src/sidebar/markup.rs:53,65`
  (`icon::render` inside `write!`), `web/src/posts/render.rs:48,241`
  (`render_masthead`, `render_load_more`), `web/src/home/component.rs:59` (feeds
  `inner_html` at `:69`)
- Test: in-file `#[cfg(test)]` in each

**Interfaces:**

- Produces:
  - `icon::markup::render(path: &str, size: u32) -> Markup`
  - `timeline::render::render_load_more(has_more: bool) -> Markup`
  - `home::render::render_masthead() -> Markup`; private
    `render_hero() -> Markup`
- Also changes here (forced by `render_masthead`'s caller):
  `posts::render:: render_timeline_page`'s `chrome` parameter becomes `Markup`
  (was `&str`, `posts/render.rs:228`).

- [ ] **Step 1: Change the three signatures first, so the reds are real**

- [ ] **Step 2: Re-pin the goldens and correct the comments**

Assert on `.as_str()` in each module's tests. Correct `icon/markup.rs:1`'s
"matching the reactive `Icon`" claim (A13). `render_load_more(false)` must still
produce the empty fragment — express it as `Markup::empty()`.

`icon::render` interpolates `path` **unescaped** today (`:8`); its inputs are
the `Icons` constants. Under `html!` the attribute is escaped normally; SVG path
data has no HTML metacharacters, so the golden should be unchanged. If it
differs, re-pin and say why in the commit body.

- [ ] **Step 3: Run them, verify they fail**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p web icon timeline home`
Expected: FAIL — bodies return `String` where `Markup` is declared.

- [ ] **Step 4: Convert all three and fix the callers**

`render_hero`'s output composes into `render_masthead` as an interpolated
`Markup`, not string concatenation. `sidebar/markup.rs`'s two `write!` sites
take `.as_str()`/`.into_string()` until Task 9. `posts/render.rs:48` passes a
`Markup` for `chrome`; `:241` adapts `render_load_more`.

- [ ] **Step 5: Run them, verify they pass** — Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add web/src/icon/markup.rs web/src/timeline/render.rs web/src/home/render.rs \
        web/src/home/component.rs web/src/sidebar/markup.rs web/src/posts/render.rs
git commit -m "refactor(web): build icon, load-more, and masthead markup with maud"
```

---

### Task 8: Convert `topbar` + `taglist`

**Files:**

- Modify: `web/src/topbar/markup.rs`, `web/src/taglist/markup.rs`
- Modify (callers, same commit): `web/src/posts/render.rs:55,62,73` (pass
  `Markup::empty()`), `:217` (`taglist::render`), `web/src/home/render.rs:28`
  (the `topbar::render(` call; its buttons are built at `:31-32`)
- Test: in-file `#[cfg(test)]` in each

**Interfaces:**

- Produces:
  - `topbar::markup::render(title: &str, sub: Option<&str>, right: Markup) -> Markup`
    — **`right` changes from `&str` to `Markup`** (A6, D4)
  - `taglist::markup::render(tags: &[TagSummary], ctx: &TagCtx) -> Markup`

- [ ] **Step 1: Re-pin the goldens and correct the comments**

`topbar/markup.rs`'s two tests (`:20-40`) carry _"Must stay byte-identical to
the reactive `Topbar`"_ — A13 names `:21` and `:33`. Replace with the
renderer-regression framing from Task 3, assert on `.as_str()`, and pass
`Markup::empty()` for `right`. Correct the doc comment at `:1-3` describing
`right` as "trusted HTML" — that claim now lives in the type.
`taglist/markup.rs`'s three tests re-pin the same way; correct
`taglist/markup.rs:10`'s byte-identity claim (A13).

- [ ] **Step 2: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web topbar taglist`
Expected: FAIL — `Markup::empty()` passed to a `&str` parameter (a genuine red
before any body changes).

- [ ] **Step 3: Convert both and update the call sites**

`topbar::render`'s optional `sub` becomes an `@if let` branch inside `html!`
rather than a pre-built `sub_html` string. `taglist::render` replaces its
`write!` loop (`:20,26`) with `@for`, and the `use std::fmt::Write;` import at
`:1` goes. At `home/render.rs:28` the buttons become a `Markup` built with
`html!`.

- [ ] **Step 4: Run them, verify they pass** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/topbar/markup.rs web/src/taglist/markup.rs \
        web/src/posts/render.rs web/src/home/render.rs
git commit -m "refactor(web): build topbar and tag list with maud; type the trusted slot"
```

---

### Task 9: Convert `sidebar`

**Files:**

- Modify: `web/src/sidebar/markup.rs` — builds with a mix of **`write!`**
  (`:50,62,70`, via `use std::fmt::Write as _;` at `:2`) and **`push_str`**
  (`:55,68,76`), not `format!`
- Modify (caller, same commit): `web/src/app/render.rs:182`
- Modify: `web/src/sidebar/component.rs:60,69` (the `inner_html` site + its
  comment; A13 names `sidebar/markup.rs:41-43`)
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Produces: `sidebar::markup::render_sidebar(active_key: &str) -> Markup`

- [ ] **Step 1: Change the signature first, so the red is real**

- [ ] **Step 2: Re-pin the two goldens**
      (`sidebar_renders_brand_public_nav_sources_and_empty_foot`,
      `sidebar_active_class_absent_for_non_home_route`) to `.as_str()`, and
      correct the byte-identity comment at `:41-43`.

- [ ] **Step 3: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web sidebar`
Expected: FAIL — body returns `String` where `Markup` is declared.

- [ ] **Step 4: Convert to `html!` and fix the caller**

The nav-item loop that `write!`s becomes `@for`; the active-class conditional
becomes an attribute expression, not string assembly. Remove
`use std::fmt::Write as _;` (A1 — an unused import fails clippy).
`app/render.rs:182` adapts until Task 11.

- [ ] **Step 5: Run them, verify they pass** — Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add web/src/sidebar/markup.rs web/src/sidebar/component.rs web/src/app/render.rs
git commit -m "refactor(web): build the sidebar with maud"
```

---

### Task 10: Convert `posts/render.rs` and its `inner_html` callers

The largest module (601 lines incl. tests) and the one holding the trusted-HTML
interpolation the whole design is built around.

**Files:**

- Modify: `web/src/posts/render.rs`
- Modify: `web/src/posts/component.rs:183-204,890-891` (three `inner_html` sites
  and their comments; A13 names `:184`)
- Modify (caller, same commit): `web/src/app/render.rs:183`
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Consumes: `Markup::from_rendered_html` — this is where the raw door is _used_,
  for the `rendered_html` field at `posts/render.rs:132`.
- Produces (all `-> Markup`): `render_body`, `permalink_article`, `render_posts`
  (`:107`), `render_timeline_page` (`:228`), `render_post_article`,
  `render_post_inner`, `render_post_content`.
- **Unchanged:** `format_post_time(ts: UtcInstant) -> String` (A3 — a time
  label, consumed at `posts/component.rs:164`) and the `test_fixtures` helpers
  `sample_post`, `sample_summary`, `one_post_page`.

- [ ] **Step 1: Change the seven signatures first, so the red is real**

- [ ] **Step 2: Re-pin the module's goldens and correct the comments**

Assert on `.as_str()` throughout. A13 names `posts/render.rs:14` and `:142`;
`posts/component.rs:184` likewise. The fixtures at `:269,290,345,533,553,580`
keep calling `RenderedHtml::from_trusted` — test code, exempt from the
`from_trusted` gate.

- [ ] **Step 3: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web posts::render`
Expected: FAIL — bodies return `String` where `Markup` is declared.

- [ ] **Step 4: Convert to `html!`; update the three sinks and
      `app/render.rs:183`**

The trusted body becomes `(Markup::from_rendered_html(view.rendered_html))`
inside the `html!` — the door is _called_ here but _defined_ in `html.rs`, so
Task 4's gate stays green with its single entry. `render_posts`' `push_str` loop
(`:110`) becomes `@for`. Update the `inner_html` sites per what Task 3
established; the `.clone()` at the home/sidebar sites is why `Markup: Clone`
exists.

- [ ] **Step 5: Run them, verify they pass** — Expected: PASS.

- [ ] **Step 6: Run both gates**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS — one
door; four sink entries at their multiplicities.

- [ ] **Step 7: Commit**

```bash
git add web/src/posts/render.rs web/src/posts/component.rs web/src/app/render.rs
git commit -m "refactor(web): build post markup with maud"
```

---

### Task 11: Convert `app/render.rs` and the projector boundary

**Files:**

- Modify: `web/src/app/render.rs`
- Modify: `web/src/app/mod.rs` — add `pub use crate::html::Markup;` (a **new**
  re-export; `Markup` is not in `render`, so it does not join the `:7` list)
- Modify: `server/src/projector/mod.rs:41,78-79`
- Test: in-file `#[cfg(test)]` in `app/render.rs`

**Interfaces:**

- Produces: `web::app::render_head(seed: &PageSeed) -> Markup`,
  `web::app::render_shell(seed: &PageSeed) -> Markup`, `web::app::Markup`
  (re-export), private `render_discovery(seed: &PageSeed) -> Markup`.
  `feed_label` (`:156`) stays `-> String` — a label, not markup (A3).

- [ ] **Step 1: Change the signatures first, so the red is real**

- [ ] **Step 2: Re-pin the module's goldens** at `:266,270,305,312,320-325` to
      `.as_str()`.

**Do not touch `app/render.rs:37`'s comment** — A13 excludes it deliberately: it
asserts the per-visitor CDN property (ADR-0041 decision 4), a different claim
that is still true.

- [ ] **Step 3: Add the D8 drift-guard test**

```rust
    /// D8: maud cannot splice a const as an attribute NAME, so the literal is
    /// written in the `html!`. This keeps `csr/src/lib.rs:41`'s selector honest —
    /// changing the const now fails here instead of silently diverging.
    #[test]
    fn discovery_marker_attr_matches_the_literal_written_in_the_markup() {
        assert_eq!(DISCOVERY_MARKER_ATTR, "data-jaunder-discovery");
    }
```

- [ ] **Step 4: Run them, verify they fail**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web app::render`
Expected: FAIL — bodies return `String` where `Markup` is declared.

- [ ] **Step 5: Convert and update the projector**

`render_shell` composes `posts::render::render_body(seed)` (`:183`) as an
interpolated `Markup`. The `<meta property="og:title">` pair (`:80-81`) writes
as-is — maud accepts arbitrary attribute names. The discovery links (`:133,146`)
write the literal `data-jaunder-discovery`, pinned by Step 3. At
`server/src/projector/mod.rs:78-79` the two values become `Markup`; call
`.into_string()` where the response body is assembled. Trust is type-carried
across that boundary, so no gate entry is needed (D4).

- [ ] **Step 6: Run them, verify they pass**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p web -p server app::render projector`
Expected: PASS — including the occurrence counts at `:200,209`.

- [ ] **Step 7: Commit**

```bash
git add web/src/app/render.rs web/src/app/mod.rs server/src/projector/mod.rs
git commit -m "refactor(web): build the page shell with maud; hand the projector Markup"
```

---

### Task 12: Delete `escape_html`; rewrite the `html.rs` module doc

**Files:**

- Modify: `web/src/html.rs`
- Modify: `web/src/lib.rs:22-23` (the comment describing the `html` module)

**Interfaces:** removes `pub(crate) fn escape_html`. Nothing may still call it.

- [ ] **Step 1: Delete `escape_html` and its test**

Remove the fn (`html.rs:10-24`) and `escape_replaces_markup_metacharacters`
(`:30-33`). Task 2's contract test is its successor.

- [ ] **Step 2: Rewrite the module doc** (A14)

The current doc (`:1-7`) is entirely about escaping and names the five caller
files. Replace it with a doc for `Markup`, and **carry forward the invariant
this module is the sole record of**: plain-string building only, no leptos
reactivity, so `reactive_graph` never sits on the public request path (the #173
escape, ADR-0040). State that maud preserves it — a compile-time macro producing
a string, with no reactive runtime. Update `web/src/lib.rs:22-23` to match.

- [ ] **Step 3: Verify nothing references it**

Run: `rg 'escape_html' web/src` — Expected: no hits (A2).

- [ ] **Step 4: Run the full host test set**

Run: `devtool run --cwd <worktree> -- cargo nextest run -p web` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/html.rs web/src/lib.rs
git commit -m "refactor(web): retire escape_html in favour of maud auto-escaping"
```

---

### Task 13: Full gate + e2e coincidence confirmation

**Files:** none (verification only), unless a failure demands a fix.

- [ ] **Step 1: Run the full local gate**

Run (background — long/cold):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-333-html-macro-adr -- cargo xtask validate`
Expected: PASS — static checks, clippy (host **and** wasm32), both new gates,
the retrofitted gate, coverage, and all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos. (A16)

- [ ] **Step 2: Confirm the coincidence oracle specifically**

Confirm `timeline-cls.spec.ts` passed — four projector-painted routes at
`tolerancePx: 0` (A11). This is what D1 rests the byte-change argument on; do
not accept a skip or a retry-pass. If it fails, the escaping/whitespace change
moved something and the plan's core assumption is wrong — report on #333 rather
than loosening the tolerance.

- [ ] **Step 3: Audit the acceptance criteria**

Walk A1–A17 and confirm each. A1 mechanically:

```
rg 'format!|write!|push_str|fmt::Write' \
  web/src/app/render.rs web/src/posts/render.rs web/src/timeline/render.rs \
  web/src/home/render.rs web/src/icon/markup.rs web/src/sidebar/markup.rs \
  web/src/topbar/markup.rs web/src/avatar/markup.rs web/src/taglist/markup.rs
```

Expected: no hits outside `#[cfg(test)]`.

- [ ] **Step 4: Hand off to jaunder-ship** — no commit of its own unless Step 1
      required fixes.

---

## Self-Review

**Spec coverage:** A1→T7-T11 (+T13 audit, incl. the `fmt::Write` imports);
A2→T12; A3→T7,T10,T11 (the HTML-returning set enumerated in the spec, incl.
`render_posts` and `render_timeline_page`); A4→T2; A5→T2,T4; A6→T8; A7→T4;
A8→T5; A9→T6; A10→T4,T5; A11→T13; A12→T2; A13→T3,T7,T8,T9,T10 (T11's exclusion
honored); A14→T12; A15→T11 Step 3; A16→T13; A17→T1. D1→T13 Step 2; D2→the
re-pins; D3→all conversions; D4→T2; D5→T4-T6; D6→T2; D7→T12; D8→T11. No gap.

**Placeholders:** none — every implementation step has tests pinning its
branches, or an explicit note where the tests can't (the `// XSS SAFETY:` text,
`Render`'s verbatim push, both gates' unreadable-classes docs).

**Type consistency:** `Markup::new` / `empty` / `from_rendered_html` / `as_str`
/ `into_string` used identically in Tasks 2–12. Every render fn returns
`Markup`; the three deliberate `String` survivors (`format_post_time`,
`feed_label`, `avatar_parts`' tuple) are named in both the task interfaces and
A3.

**Ordering:** every conversion task lists its callers under **Files** and fixes
them in the same commit, per the Conversion invariant table — no intermediate
commit leaves `-p web` unbuildable.

**Red steps:** each conversion task changes the signature as its _first_ step,
so the "verify it fails" step is a genuine type error rather than a comment edit
that would have compiled and passed.
