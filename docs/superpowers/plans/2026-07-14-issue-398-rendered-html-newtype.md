# Plan — `RenderedHtml` newtype (issue #398)

**Spec:**
[`../specs/2026-07-14-issue-398-rendered-html-newtype.md`](../specs/2026-07-14-issue-398-rendered-html-newtype.md)
(the "what/why"; this plan is the "how"). **Issue:** #398. **Follow-up filed:**
#445 (stored-XSS sanitization, blocked-by #398).

## Review header

**Goal.** Replace the bare `String` that carries a post's rendered HTML with a
`RenderedHtml` newtype minted only by `render()` (plus one named trusted-rebuild
door), so the unescaped `inner_html` sink accepts only `RenderedHtml` and a raw
`String` there is a compile error.

**Scope — in:** the type in `common::render`; threading through storage
records/inputs + both SQL backends; the two web wire DTOs; `PostView` + the emit
path; the `media/mod.rs` non-emit reader; all test-fixture churn; a
`compile_fail` doctest locking the construction guarantee. **Out:** HTML
sanitization (#445); any change to emitted markup; new ADRs (ADR-0063 already
covers this).

**Tasks.**

1. Introduce `RenderedHtml` + trailer
   (`Display`/`AsRef`/`Serialize`/`from_trusted`) and its guarantee doctests, in
   `common::render` — not yet wired into `render()`.
2. Flip the **storage** side: `render() -> RenderedHtml`, storage record/input
   fields, `build_post_record`, both backend binds, `post_service`; feed the
   still-`String` web DTOs via a temporary `.as_ref().to_string()`. Fix
   storage/server fixtures.
3. Flip the **web** side: wire DTOs → `RenderedHtml` (+ `deserialize_with`),
   remove the temporary conversion, `PostView: &RenderedHtml`, emit via
   `Display`, `media` reader via `.as_ref()`. Fix web fixtures.
4. Honest doc-comments + final gate: correct the "Sanitized HTML rendering"
   comments to reference #445; `cargo xtask validate --no-e2e`.

**Key risks / decisions.**

- **Cross-crate ripple.** Retyping `render()`'s output breaks storage _and_ web
  at once. Tasks 2/3 stay independently green via a **temporary**
  `.as_ref().to_string()` at the single server-side DTO-construction site
  (`web/src/posts/server.rs`), removed in Task 3. No escape-hatch is added to
  the _type_.
- **Test churn is the bulk of the diff** (~109 struct-literal fixtures + 6
  `PostView` `&str` literals + 1 atompub field mutation). A one-line
  `from_trusted`-backed test helper cuts verbosity; the mechanical sweep is a
  good `jaunder-dispatch` delegation.
- **Guarantee proof.** The boundary is enforced _structurally_ — the workspace
  compiling proves no `String` reaches the sink and no construction door exists
  beyond `render()`/`from_trusted`. `compile_fail` doctests **document** the
  "can't construct from a `String`" guarantee (matching how `macros/` expresses
  the `StrNewtype` guarantee — `macros/src/lib.rs`, `str_newtype.rs:97`), but
  note the Nix gate runs `cargo llvm-cov nextest`, which does **not** execute
  doctests — so they're runnable/documentary (`cargo test -p common --doc`), not
  gate-enforced. The behavioral guard is the existing XSS-boundary test.

**For agentic workers.** Execute with `jaunder-iterate`; delegate the Task-2/3
fixture sweeps via `jaunder-dispatch`. Tick checkboxes in real time.

## Global constraints

- Rust. Gate each committed task with `cargo xtask check` (fmt + clippy + Nix
  coverage/tests) — run it clean _before_ committing (`jaunder-commit`). No
  `Co-Authored-By` trailer. Serialize edit→gate→commit (no edits mid-gate).
- Storage tests follow the dual-backend template (`CONTRIBUTING.md` backend
  parity); don't put tests in ADR-0019 dialect files.
- `RenderedHtml`'s field stays private to `common::render`; the **only** mint
  doors are `render()` (same module → constructs directly) and the
  `pub fn from_trusted`. Do **not** add
  `From`/`TryFrom`/`Deref`/`FromStr`/`Deserialize`.
- **Conversion hygiene (per "genericize our signatures").** Prefer making _our_
  new APIs generic to kill caller boilerplate — `from_trusted` takes
  `impl Into<String>` so fixtures skip `.to_string()`; the emit path uses
  `Display` (no `.to_string()`). Every residual `.as_ref()`/`.to_string()` in
  the diff must be either eliminated or briefly annotated with why it's needed.
  The only legitimate residuals are the two SQL binds
  (`.bind(input.rendered_html.as_ref())`) and the `media` reader
  (`.as_ref().contains(&url)`) — both because `RenderedHtml` deliberately has no
  `Deref`; annotate each inline.

---

## Task 1 — `RenderedHtml` type + trailer + guarantee doctests

**Files:** `common/src/render.rs` (add type near the top, beside `PostFormat`);
tests in-file (`#[cfg(test)]`) per the crate convention. Not exported changes to
`render()` yet.

**Add** (`render()` still returns `String` in this task):

````rust
use std::fmt;
use serde::{Serialize, Serializer};

/// HTML **produced by [`render`]**. This is a *provenance* marker, not a safety
/// guarantee: `render` does **no** sanitization (see #445), so this type means
/// "came out of our renderer", NOT "safe/XSS-free". Its value is structural — the
/// unescaped view sink accepts only `RenderedHtml`, so a raw `String`/body cannot
/// reach it by accident.
///
/// The only ways to obtain one are [`render`] (mints new HTML) and
/// [`RenderedHtml::from_trusted`] (rebuilds a value already produced by `render`
/// and round-tripped through our own storage or wire). There is deliberately no
/// `From`/`TryFrom`/`Deref`/`FromStr`/`Deserialize`.
///
/// Constructing one from an arbitrary string does not compile:
/// ```compile_fail
/// let _ = common::render::RenderedHtml("<p>x</p>".to_string()); // private field
/// ```
/// ```compile_fail
/// let _: common::render::RenderedHtml = "<p>x</p>".to_string().into(); // no From
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedHtml(String);

impl RenderedHtml {
    /// Rebuild a `RenderedHtml` from a string the caller asserts is prior
    /// [`render`] output round-tripped through our own store or wire. This is the
    /// single trusted-rebuild door; grep it to enumerate every rebuild site.
    /// Takes `impl Into<String>` so callers (esp. fixtures) don't need `.to_string()`.
    #[must_use]
    pub fn from_trusted(html: impl Into<String>) -> Self {
        Self(html.into())
    }
}

impl fmt::Display for RenderedHtml {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RenderedHtml {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// Reading out is always safe; deliberately NO `Deserialize` (the wire uses a
// `deserialize_with` helper that routes through `from_trusted`).
impl Serialize for RenderedHtml {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
````

**Tests (in-file):**

```rust
#[test]
fn rendered_html_display_and_as_ref_expose_inner() {
    let h = RenderedHtml::from_trusted("<p>hi</p>");
    assert_eq!(h.to_string(), "<p>hi</p>");
    assert_eq!(h.as_ref(), "<p>hi</p>");
}

#[test]
fn rendered_html_serializes_as_the_raw_string() {
    let h = RenderedHtml::from_trusted("<b>x</b>");
    assert_eq!(serde_json::to_string(&h).unwrap(), "\"<b>x</b>\"");
}
```

**Run:** `cargo nextest run -p common render` → PASS.
`cargo test -p common --doc` → the two `compile_fail` doctests PASS (they fail
to compile, as asserted) — a manual check; nextest/the Nix gate don't run
doctests, so this documents the guarantee the way `macros/` does rather than
gating it. Workspace still builds (`RenderedHtml` is pub-exported, so not dead
code).

**Commit** after `cargo xtask check` is clean.

---

## Task 2 — Flip the storage side (workspace stays green via a temporary bridge)

**Files:**

- `common/src/render.rs` — `pub fn render(...) -> RenderedHtml` (construct
  `RenderedHtml(...)` internally). Fix unit tests at `render.rs:480,486,655` to
  compare via `.as_ref()` / `.to_string()`.
- `storage/src/posts.rs` — fields `rendered_html: RenderedHtml` on `PostRecord`
  (:56), `PostRevisionRecord` (:120), `CreatePostInput` (:196),
  `UpdatePostInput` (:218). (`PostRevisionRecord` has no Rust build/read path —
  inert retype.)
- `storage/src/helpers.rs` — `build_post_record` (:165) wraps the DB string via
  `RenderedHtml::from_trusted(rendered_html)`.
- `storage/src/sqlite/posts.rs:92` and `storage/src/postgres/posts.rs:92` —
  `.bind(input.rendered_html.as_ref())`.
- `storage/src/post_service.rs` — the 3 callers (75, 160, 322) are unchanged in
  shape (they move `render()`'s result straight into a `*PostInput`).
- **Temporary bridge:** `web/src/posts/server.rs` — where `PostResponse` /
  `TimelinePostSummary` are built from a `PostRecord`, feed their still-`String`
  `rendered_html` with `record.rendered_html.as_ref().to_string()`. Mark
  `// TODO(#398): drop in Task 3 when the DTO field becomes RenderedHtml`.
- **`web/src/media/mod.rs:154`** — this reads `PostRecord.rendered_html`
  directly (`Vec<PostRecord>`), so the storage flip breaks it _here_, not in
  Task 3: `post.rendered_html.as_ref().contains(&url)`.
- **Why the web crate still compiles in Task 2:** all three `PostView` feeds
  (`render/mod.rs:355,396`, `ui.rs:301`) borrow `rendered_html` from a **wire
  DTO** (`PostResponse`/`TimelinePostSummary`), which stays `String` until Task
  3 — so `PostView.rendered_html` stays `&str` and those sites are untouched
  here. Only the two direct `PostRecord` consumers above (`server.rs` bridge,
  `media`) need Task-2 edits.
- **Fixtures (storage/server):** every `rendered_html:` `String` literal
  building `PostRecord`/`*PostInput` → `RenderedHtml::from_trusted("…")` (no
  `.to_string()` — the door takes `impl Into<String>`); the atompub mutation
  `p.rendered_html = "…".to_string()` (`server/src/atompub/posts.rs:620`) →
  `RenderedHtml::from_trusted("…")`. ~67 in `server/tests/storage/mod.rs`, plus
  `feed_worker.rs`, `feed_regenerate.rs`, `feed_handlers.rs`,
  `backup_fixture.rs`, and any storage/server test touching these structs.
  Optionally a local
  `fn rh(s: &str) -> RenderedHtml { RenderedHtml::from_trusted(s) }` to shorten
  the path at high-density sites.
- Update the XSS-boundary assertion `record.rendered_html.contains(…)` →
  `record.rendered_html.as_ref().contains(…)` (`post_service.rs:1043`).

**Delegation:** the fixture sweep is mechanical — hand it to `jaunder-dispatch`
(bulk Edit sweep) while the driver does the production threading; verify by
compiling.

**Run:** `cargo xtask check` clean (fmt+clippy+coverage+tests build & pass), and
in particular `cargo nextest run -p server storage::` incl.
`test_perform_post_creation_org_title_rendered_once` → PASS.

**Commit** after `cargo xtask check` is clean.

---

## Task 3 — Flip the web side + remove the bridge

**Files:**

- `web/src/posts/mod.rs:184` (`PostResponse`) and `web/src/posts/listing.rs:40`
  (`TimelinePostSummary`) — field `rendered_html: RenderedHtml` with
  `#[serde(deserialize_with = "deserialize_rendered_html")]`. Add the helper
  once (e.g. in `web/src/render/mod.rs` or a shared web module), imported by
  both:

  ```rust
  fn deserialize_rendered_html<'de, D>(d: D) -> Result<RenderedHtml, D::Error>
  where
      D: serde::Deserializer<'de>,
  {
      // Trusted: the wire value is prior `render()` output sent by our own server.
      Ok(RenderedHtml::from_trusted(String::deserialize(d)?))
  }
  ```

  (`Serialize` is provided by the type.) Confirm both DTOs keep `PartialEq, Eq`
  (satisfied — `RenderedHtml: Eq`); no `Ord`/`Hash` needed.

- `web/src/posts/server.rs` — **remove** the Task-2 `.as_ref().to_string()`
  bridge; move the `RenderedHtml` straight from `PostRecord`.
- `web/src/render/mod.rs:413` — `PostView.rendered_html: &'a RenderedHtml`. The
  emit at `render_post_content` (:497) already interpolates `{body}` via
  `Display` — now typed. `PostView` feed sites: `render/mod.rs:355,396` and
  `pages/ui.rs:301` (`rendered_html: &post.rendered_html`) — type-compatible
  (the DTO field is now `RenderedHtml`, so these feeds now yield
  `&RenderedHtml`, matching `PostView`). (`media/mod.rs:154` was already fixed
  in Task 2 — it reads `PostRecord`.)
- **Fixtures (web):** the 6 `PostView` `&str` literals in
  `web/src/render/mod.rs` (827, 881, 902, 1204, 1223, 1249) — bind an owned
  `RenderedHtml` (`let body = RenderedHtml::from_trusted("<p>b</p>");`) and pass
  `&body`. Plus any `web/posts/mod.rs` / `web_posts` / `web_tags` / `web_media`
  / `projector` fixtures building the DTOs with `rendered_html:` literals →
  `RenderedHtml::from_trusted(…)`.

**Delegation:** web fixture sweep → `jaunder-dispatch`.

**Run:** `cargo xtask check` clean; `cargo nextest run -p web` PASS (projector /
byte-identical-paint tests still green — no markup changed).

**Commit** after `cargo xtask check` is clean.

---

## Task 4 — Honest doc-comments + final validate

**Files:**

- `storage/src/posts.rs:55,120` (and any sibling) — change "Sanitized HTML
  rendering of the `body`." to the honest provenance wording, e.g. "HTML
  produced by `render()` from the `body`. Not sanitized — see #445." Mirror on
  `PostResponse`/ `TimelinePostSummary` docs if they repeat the claim.

**Run:** `cargo xtask validate --no-e2e` → clean (the full pre-push gate:
static + clippy + coverage). Confirm
`test_perform_post_creation_org_title_rendered_once` passes; separately run
`cargo test -p common --doc` to check the `compile_fail` doctests (not in the
Nix gate).

**Commit** after the gate is clean.

## Self-review checklist

- [ ] Every `rendered_html` field/consumer from the spec's path table is
      threaded (records, inputs, both binds, DTOs, `PostView`, emit, `media`
      reader).
- [ ] No `From`/`TryFrom`/`Deref`/`FromStr`/`Deserialize` on `RenderedHtml`;
      mint doors are exactly `render()` + `from_trusted`.
- [ ] Task-2 temporary bridge in `server.rs` is removed in Task 3 (grep for the
      `TODO(#398)`).
- [ ] `compile_fail` doctests present (documentary, per repo convention);
      XSS-boundary test passes under the Nix gate.
- [ ] No "sanitized" claim survives in doc comments; #445 referenced.
