# Spec — feed serialization surface adopts existing newtypes; ADR-0063 pervasiveness rule (issue #470)

- **Issue:** [#470](https://github.com/jaunder-org/jaunder/issues/470) — "types:
  feed serialization surface flattens existing newtypes
  (FeedItem.title/content_html); codify newtype pervasiveness in ADR-0063"
- **Milestone:** Domain-value type safety (newtypes)
- **ADRs:** amends **ADR-0063** (domain-value newtype convention). Relates to
  #402 (PostTitle/PostBody), #398 (RenderedHtml + its `from_trusted` static
  check).
- **Date:** 2026-07-16

## Problem

`common/src/feed/metadata.rs::FeedItem` carries two fields as bare primitives
whose values are, on the sole production path (`build_feed_items`,
`server/src/feed/regenerate.rs`), taken directly from a post's own newtype-typed
fields and flattened:

| Field                    | Declared         | Source                                   | Flatten                             |
| ------------------------ | ---------------- | ---------------------------------------- | ----------------------------------- |
| `FeedItem::title`        | `Option<String>` | `PostRecord.title: Option<PostTitle>`    | `p.title.clone().map(String::from)` |
| `FeedItem::content_html` | `String`         | `PostRecord.rendered_html: RenderedHtml` | `p.rendered_html.to_string()`       |

`FeedItem::tags` is already `Vec<TagLabel>`. `content_html` is the material
concern: `RenderedHtml` is ADR-0063 §1's _trust/provenance_ type ("came out of
our renderer"; the unescaped view sink accepts only `RenderedHtml`), and the
feed renderers are exactly the unescaped sink — flattening it to `String` at the
`FeedItem` boundary discards the transposition guard where it matters most (a
raw body could be assigned to `content_html` with nothing to object).

Both were **deliberately** excluded from the #402/#398 sweeps, justified in #402
as "a generic/ingested feed-item title, not our post's `PostTitle`." An audit of
**every** construction site refutes that: `FeedItem`/`FeedMetadata` have exactly
one production path each (both in `regenerate.rs`, sourced from
`PostRecord`/`SiteIdentity`); there is **no** ingestion path building a
`FeedItem` from foreign content (the lone XML-ingest, `entry_from_xml`, targets
`atom_syndication::Entry`, never `FeedItem`). The exclusion rested on a false
premise, and the governing ADR left a loophole that let it be written down.

## Decisions

### D1 — Re-type the two `FeedItem` fields to the newtypes they already carry

- `FeedItem::title: Option<PostTitle>`
- `FeedItem::content_html: RenderedHtml`

`FeedItem`/`FeedMetadata` derive only `Debug, Clone` (no serde) and are
transient in-memory inputs to the renderers — never themselves (de)serialized or
cached — so this needs neither `Deserialize` (which `RenderedHtml` deliberately
lacks) nor any wire-format change. `metadata.rs` imports
`crate::post_title::PostTitle` and `crate::render::RenderedHtml`.

### D2 — `build_feed_items` propagates, it does not flatten

`title: p.title.clone()` and `content_html: p.rendered_html.clone()` (both are
`Clone`). The `.map(String::from)` / `.to_string()` flattening is deleted. This
makes the field assignment transposition-checked: only an `Option<PostTitle>` /
`RenderedHtml` can land there. **No** `RenderedHtml::from_trusted` call is
introduced on the production path (the value is already a `RenderedHtml`), so
the `rendered-html-from-trusted` static check is untouched.

### D3 — Renderers convert only at the external-crate boundary

The three renderers hand text to **third-party builders** — `atom_syndication`
(`Text::plain`, `Content.value`), the `rss` crate
(`ItemBuilder::title/description`), and `serde_json::Value`. These are non-owned
types we cannot re-type, so the newtype is read out here via its trailer:

- `PostTitle`: `Deref<str>`/`AsRef<str>`/`Display` — e.g. `i.title.as_deref()`
  for the `Option`, `String::from(..)` / `.to_string()` where the external
  builder demands an owned `String`.
- `RenderedHtml`: `Deref<str>`/`AsRef<str>`/`Serialize` — read as `&str` via
  deref (`&*i.content_html` / `i.content_html.as_ref()`) for the
  `serde_json::json!` leaf (avoids relying on `json!`'s `From`-vs-`Serialize`
  leaf handling); `atom`/`rss` take `i.content_html.to_string()`.

This is the **only** sanctioned flatten (D5 of the ADR amendment): the value is
typed everywhere _we_ own it and decays solely into external builders.
Serialized output is **byte-identical** (both newtypes render as their inner
string).

### D4 — Test fixtures: `parse_post_title` helper; `from_trusted` inline in `cfg(test)`

- Add `common::test_support::parse_post_title(&str) -> PostTitle` (via
  `PostTitle::from(s.to_owned())` — `PostTitle` is infallible, no `FromStr`, so
  it does **not** use the `parse().expect()` shape of the other helpers),
  matching the shared-fixture convention (used by ≥2 test modules). The four
  feed `#[cfg(test)]` modules (`metadata`, `json`, `rss`, `atom`) build titles
  through it.
- `RenderedHtml` test values use `RenderedHtml::from_trusted("<p>…</p>")`
  **inline** in those `#[cfg(test)]` modules. The `rendered-html-from-trusted`
  check excludes `#[cfg(test)]` code, so no allowlist edit is needed. A
  `test_support` helper is deliberately **avoided** for `RenderedHtml`, because
  `test_support` is gated `#[cfg(any(test, feature = "test-support"))]` — not
  pure `cfg(test)` — and a `from_trusted` there could trip the static check.

### D5 — Amend ADR-0063 with a pervasiveness rule + external-type carve-out

Add a subsection (a new numbered rule, e.g. §5 "Use an existing newtype
everywhere its value appears"):

- **Rule.** Once a domain newtype exists, every field, argument, return, DTO,
  **and serialization/DTO surface** that carries that value MUST be typed as the
  newtype. Flattening it to a primitive requires **express owner approval**,
  recorded.
- **Close the loophole.** §1's "consistency alone is not sufficient
  justification" governs whether to **introduce a new** type; it must **not** be
  cited to leave an **existing** newtype's value as a primitive. (State this
  explicitly — it is the exact misreading #402 used.)
- **Extend §4.** The boundary rule's enumeration (`#[server]`, CLI, storage) is
  non-exhaustive; internal serialization/DTO surfaces hold the newtype too.
- **Carve-out.** The one sanctioned flatten is handing the inner value to an
  **external, non-owned** type (e.g. `atom_syndication`, `rss`,
  `serde_json::Value`), read out via `Deref`/`AsRef`/`Display`/`Serialize`.

ADR-0063 status stays **proposed** (unchanged); this is a content amendment, not
a status transition. Edit the existing `docs/adr/0063-*.md` in place (no new
draft — amending an existing ADR, not authoring one), and re-run `prettier -w`
on it before staging.

## Non-goals / carve-outs

- **Fields with no existing newtype stay `String`:**
  `FeedMetadata.{title, canonical_url, self_url, hub_url, description}`,
  `FeedItem.{permalink, summary}`, and the `FeedMeta`/`MediaLinkEntry` fields
  (no `Url`/`Summary`/`SiteTitle`/`Filename`/`Mime` newtype exists). Introducing
  those types is separate future work.
- **No enforcement gate.** The pervasiveness rule is a data-flow property not
  cheaply syn-AST-checkable; it is enforced by the ADR + code review, not an
  xtask step (owner-approved this call). No change to the existing
  `rendered-html-from-trusted` check.
- **No wire/output change**, no `RenderedHtml` API change, no
  `derive_post_metadata` change.

## Acceptance criteria (observable)

1. **AC-fields.** `FeedItem::title: Option<PostTitle>` and
   `FeedItem::content_html: RenderedHtml` in `common/src/feed/metadata.rs`.
2. **AC-propagate.** `build_feed_items` assigns `p.title.clone()` /
   `p.rendered_html.clone()` with no `String::from`/`.to_string()` flatten; the
   `rendered-html-from-trusted` check remains green with no allowlist change.
3. **AC-output.** Existing `rss`/`atom`/`json` renderer tests pass unchanged in
   assertion: a titled post still emits `<title>Hello</title>` / a `"title"`
   key; a title-less post still omits it; rendered HTML is still emitted
   verbatim (byte-identical output).
4. **AC-fixture.** `common::test_support::parse_post_title` exists and is used
   by the feed test modules; `RenderedHtml` test values use inline
   `from_trusted`.
5. **AC-adr.** ADR-0063 carries the pervasiveness rule, the "consistency-only
   governs introduction not adoption" clarification, the §4 extension, and the
   external-type carve-out.
6. **AC-gate.** `cargo xtask validate --no-e2e` clean (host static + clippy +
   coverage), no coverage regression.

## Risks / notes

- **Hollow-typing worry:** the newtype is flattened into external builders
  anyway — but the win is real: `build_feed_items`'s field assignment becomes
  transposition-checked (can't put a `PostBody`/raw string in `content_html`),
  which is the ADR-0063 §1 guarantee, preserved up to the external boundary.
- **`unwrap_or_default` on title:** `atom.rs:32` currently relies on
  `String: Default`; `PostTitle` is not `Default`, so switch to
  `i.title.as_deref()` (yields `Option<&str>`) before defaulting — a
  compile-forced, mechanical change.
- **Future ingestion (#282):** if RSS/Atom _ingestion_ later builds a `FeedItem`
  from foreign content, that title is still a post-equivalent display title; the
  pervasiveness rule + `PostTitle::from` (trimming, infallible) remain correct.
  No pre-emptive accommodation here.
