# Plan — feed newtype pervasiveness (issue #470)

**Spec:**
[`2026-07-16-issue-470-feed-newtype-pervasiveness.md`](../specs/2026-07-16-issue-470-feed-newtype-pervasiveness.md)
(the "what/why" — this plan is the "how"). **For agentic workers:** drive with
`jaunder-iterate`; delegate a task via `jaunder-dispatch` if useful. Gate each
commit with `cargo xtask check` (`jaunder-commit`); **no `Co-Authored-By`
trailer**.

## Review header

- **Goal.** Make `FeedItem` hold the newtypes its values already are
  (`title: Option<PostTitle>`, `content_html: RenderedHtml`) so the feed
  construction path is transposition-checked; codify the pervasiveness rule in
  ADR-0063. See spec §D1–D5.
- **Scope — in:** `common/src/feed/{metadata,rss,atom,json}.rs`,
  `common/src/test_support.rs`, `server/src/feed/regenerate.rs`,
  `docs/adr/0063-domain-value-newtype-convention.md`.
- **Scope — out:** every field with no existing newtype (`*_url`, `summary`,
  `permalink`, `FeedMetadata.title`, `FeedMeta`/`MediaLinkEntry`); `FeedItem.id`
  (an ID-newtype `PostId`, owned by the #457/#472 ID sweep — **not** this
  issue); any wire/output change; `RenderedHtml`/`derive_post_metadata` API;
  enforcement gate.
- **Tasks.**
  1. Thread `PostTitle`/`RenderedHtml` through `FeedItem`, its builder, the
     three renderers, and their fixtures (one atomic compile-green commit).
  2. Amend ADR-0063 with the pervasiveness rule + external-type carve-out
     (docs).
- **Key risks / decisions.**
  - The struct field change breaks compilation until every consumer is fixed →
    Task 1 is **one commit**, not splittable.
  - No new runtime behavior: the existing `rss`/`atom`/`json` tests are the
    regression guard; output must stay **byte-identical**.
  - No `from_trusted` on the production path (propagate a `.clone()`); fixtures
    use `from_trusted` **inline in `#[cfg(test)]`** (excluded from the static
    check) → zero allowlist/gate change.
  - **Merge coordination:** #457/#472 re-types `FeedItem.id` → `PostId` in the
    same `metadata.rs`/`regenerate.rs`; expect a trivial conflict at ship,
    resolve by keeping both field-type changes.

## Global constraints

- Output byte-identical: `PostTitle` and `RenderedHtml` both render as their
  inner string (`Display`/`Serialize`); a title-less post still omits the title.
- `common` is host + wasm dual-target; `server` is server-only. Verify with
  `--all-features --all-targets` (per repo gotcha: default check skips
  server-gated code).
- Gate = `cargo xtask check` (fmt + clippy + Nix coverage/tests) before each
  commit.

---

## Task 1 — Thread `PostTitle`/`RenderedHtml` through the feed types

One atomic commit (the crate will not compile until all edits land together).

### Files / interfaces

**`common/src/feed/metadata.rs`** — struct + imports + test fixture.

Add imports near the top (after the existing `use crate::tag::TagLabel;`):

```rust
use crate::post_title::PostTitle;
use crate::render::RenderedHtml;
```

Change the two `FeedItem` fields:

```rust
    pub title: Option<PostTitle>,
    // ...
    pub content_html: RenderedHtml,
```

In the `#[cfg(test)] mod tests` `item(...)` helper, build the fixtures via the
newtypes (the test mod already has `use super::*`, so `PostTitle`/`RenderedHtml`
are in scope; add the fixture import):

```rust
    use crate::test_support::parse_post_title;
    // ...
        FeedItem {
            id,
            title: Some(parse_post_title("t")),
            permalink: "/p".into(),
            summary: None,
            content_html: RenderedHtml::from_trusted("<p>c</p>"),
            // ...
        }
```

**`common/src/test_support.rs`** — add the shared fixture (convention: one place
a test title literal is built; used by ≥2 modules). `PostTitle` is infallible
(`From<String>`, no `FromStr`), so it does **not** use the `parse().expect()`
shape:

```rust
use crate::post_title::PostTitle;

/// Build a [`PostTitle`] from `title` for tests — the single place a test title
/// literal is wrapped. `PostTitle` is infallible (trimming `From<String>`), so this
/// cannot fail.
#[must_use]
pub fn parse_post_title(title: &str) -> PostTitle {
    PostTitle::from(title.to_owned())
}
```

(No `expect`, so it is unaffected by the module's
`#![expect(clippy::expect_used)]`.)

**`server/src/feed/regenerate.rs`** — `build_feed_items`: propagate, don't
flatten. Replace the two field lines (and rewrite the now-inverted comments):

```rust
            // FeedItem.title carries the post's PostTitle unflattened (#470).
            title: p.title.clone(),
            permalink: p.permalink(),
            summary: p.summary.clone(),
            // FeedItem.content_html carries the post's RenderedHtml unflattened (#470);
            // renderers read it out via Deref/Display at the external-crate boundary.
            content_html: p.rendered_html.clone(),
```

(No `from_trusted` introduced — `p.rendered_html` is already a `RenderedHtml`.)

**`common/src/feed/rss.rs`** — convert at the `rss`-crate builder boundary:

```rust
                .title(i.title.clone().map(String::from))   // was: i.title.clone()
                .link(Some(i.permalink.clone()))
                .description(Some(i.content_html.to_string()))  // was: Some(i.content_html.clone())
```

In its `#[cfg(test)] mod tests`, update the `item(...)` helper and add imports
(`use crate::render::RenderedHtml;`,
`use crate::test_support::parse_post_title;`):

```rust
            title: title.map(parse_post_title),          // was: title.map(str::to_string)
            // ...
            content_html: RenderedHtml::from_trusted("<p>hi</p>"),  // was: "<p>hi</p>".into()
```

**`common/src/feed/atom.rs`** — convert at the `atom_syndication` boundary:

```rust
                title: Text::plain(i.title.clone().map(String::from).unwrap_or_default()),  // was: i.title.clone().unwrap_or_default()
                // ...
                    value: Some(i.content_html.to_string()),  // was: Some(i.content_html.clone())
```

(`.map(String::from)` yields `Option<String>`, so `unwrap_or_default()` produces
the same owned `String` `Text::plain` already receives — no assumption about its
bound, and `PostTitle` need not be `Default`.) Update the test `item()` helper
and imports the same way as `rss.rs`.

**`common/src/feed/json.rs`** — read `content_html` as `&str` for the `json!`
leaf (avoids depending on `json!`'s `From`-vs-`Serialize` leaf handling), and
convert the title into the `serde_json::Value`:

```rust
                "content_html": &*i.content_html,   // was: i.content_html
                // ...
            if let Some(t) = &i.title {
                o["title"] = Value::String(t.to_string());  // was: t.clone()
            }
```

Update the `item(...)` / `item_with_summary(...)` helpers and imports as above
(`title.map(parse_post_title)`,
`content_html: RenderedHtml::from_trusted("<p>hi</p>")`).

### Test / verify

No new behavior — the existing renderer tests are the regression guard; they
must pass with assertions unchanged (byte-identical output).

```
cargo nextest run -p common feed          # EXPECT PASS (rss/atom/json/metadata tests, incl. titleless)
cargo nextest run -p server feed::regenerate   # EXPECT PASS
cargo check -p common --all-features --all-targets   # EXPECT clean (dual-target)
cargo check -p server --all-features --all-targets   # EXPECT clean
```

### Commit

`cargo xtask check` clean (incl. the `rendered-html-from-trusted` step green, no
allowlist change), then commit (`jaunder-commit`):
`types(feed): FeedItem holds PostTitle/RenderedHtml, unflattened (#470)`.

---

## Task 2 — Amend ADR-0063 with the pervasiveness rule

Docs-only; edit the existing ADR **in place** (not a new draft — amending, not
authoring). No status change (stays `proposed`), so the README table is
untouched.

### Files

**`docs/adr/0063-domain-value-newtype-convention.md`** — add a new numbered rule
after §4 (Boundary rule), e.g.:

> ### 5. Use an existing newtype everywhere its value appears
>
> Once a domain newtype exists, **every** field, argument, return, DTO, **and
> serialization/DTO surface** that carries that value is typed as the newtype.
> Flattening it to a primitive requires **express owner approval**, recorded in
> the issue/spec — it is not a discretionary per-site call.
>
> This is distinct from §1. §1's "consistency alone is not sufficient
> justification" governs whether to **introduce a new** type; it must **not** be
> cited to leave an **existing** newtype's value as a primitive (e.g. calling a
> field "a generic serialization surface"). Adoption of an existing type is
> mandatory, not consistency-optional. The §4 boundary enumeration (`#[server]`,
> CLI, storage) is non-exhaustive: internal serialization/DTO surfaces hold the
> newtype too.
>
> **Sole carve-out — external types.** Handing the inner value to a type we do
> not own (e.g. `atom_syndication`, the `rss` crate, `serde_json::Value`) is the
> one sanctioned flatten; read the value out via
> `Deref`/`AsRef`/`Display`/`Serialize` at that boundary. The newtype must still
> be held on every surface we define up to that point.

Add a bullet to **Consequences** noting the rule closes the #402-style exclusion
and that `FeedItem` (#470) is the first correction.

### Test / verify

```
prettier -w docs/adr/0063-domain-value-newtype-convention.md   # before staging (repo prose convention)
cargo xtask check   # markdown/adr lints in the static set; EXPECT clean
```

### Commit

`docs(adr): ADR-0063 — newtypes used pervasively; external-type carve-out (#470)`.

---

## Self-review

- Every spec AC maps to a task: AC-fields/AC-propagate/AC-output/AC-fixture →
  Task 1; AC-adr → Task 2; AC-gate → both commits' `cargo xtask check`.
- No placeholders; all edits are complete Rust against the current source lines.
- Out-of-scope fields explicitly enumerated; `FeedItem.id` deferral
  cross-referenced to #457/#472 with a merge-coordination note.
