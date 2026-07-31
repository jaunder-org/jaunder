# Spec — retire the quick-xml fork bridge and delegate AtomPub Atom I/O to `atom_syndication`

- Date: 2026-07-31
- Issues: [#199](https://github.com/jaunder-org/jaunder/issues/199),
  [#737](https://github.com/jaunder-org/jaunder/issues/737)
- Branch: `worktree-issue-199-issue-737-upstream-atom-entry`

Both issues land in **one branch and one PR**, a deliberate, user-approved
deviation from the 1-issue-1-branch-1-PR convention: #737 cannot compile until
#199's `[patch.crates-io]` is gone, because the patch pins `atom_syndication` to
a fork that predates the API #737 consumes.

## Background

`common/src/atompub/entry.rs` hand-rolls a `quick-xml` SAX reader and four
writers for Atom documents. The reason is recorded in that file's own module doc
(`entry.rs:1-9`):

> We do **not** reuse `atom_syndication`'s XML I/O, because its entry-level
> read/write traits are crate-private; it can only handle whole `<feed>`
> documents, while `AtomPub` exchanges _standalone_ `<entry>` documents.

That premise expired on 2026-07-30 with `atom_syndication` **0.12.10**, which
makes bare-entry I/O public: `Entry::read_from`, `impl FromStr for Entry`,
`Entry::write_to`, and `Entry::write_with_config`.

Separately, ADR-0043 forked `atom_syndication` and `rss` onto `quick-xml 0.41`
to clear RUSTSEC-2026-0194/0195, wiring the forks in via `[patch.crates-io]`
plus hermetic crane vendoring. Its stated exit condition — upstream releases
depending on quick-xml ≥ 0.41 — is now met on **both** crates:
`atom_syndication` 0.12.9+ and `rss` 2.1.0 (both verified against the crates.io
dependency API as requiring `^0.41`).

## Decisions

| #   | Decision                                                                                                                                                                 |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| D1  | Delegate **all three** Atom document writers plus the reader to upstream: entry read/write, `render_feed` (via `atom_syndication::Feed`), and `render_media_link_entry`. |
| D2  | Accept upstream's serialization **verbatim** — no post-processing to restore today's bytes.                                                                              |
| D3  | The three serializers return `Result<String, AtomPubError>`; call sites propagate with `?`.                                                                              |
| D4  | `set_draft` owns the `app` prefix in `entry.namespaces` (inserted on set, removed on clear); `set_j_slug` owns the `j` prefix (inserted on set — there is no clear).     |
| D5  | One new ADR records both halves; ADR-0043 flips to `superseded` pointing at it.                                                                                          |
| D6  | Upstream's stricter parsing is accepted as-is (see "Parse strictness"), with a test pinning each new rejection.                                                          |
| D7  | Local bar is `validate --no-e2e` plus a targeted `e2e-local atompub.spec.ts`; the full browser matrix is CI's.                                                           |
| D8  | `FeedMeta` and `MediaLinkEntry` carry `UtcInstant`, not RFC-3339 `String`.                                                                                               |
| D9  | Serialization failure gets its own `AtomPubError` variant mapping to **500**, so it cannot surface as a client 400.                                                      |

**D1's deciding factor** is that upstream's _embedded_ entry serializer
(`ToXml for Entry`, the no-`xmlns` form used inside a `<feed>`) is still
`pub(crate)` — verified at `toxml.rs:8` and `entry.rs:819-823`. Replacing only
`entry_from_xml`/`entry_to_xml` would force `render_feed` to keep a hand-rolled
entry writer purely for embedding, leaving two entry serializers in one module
that must be kept in agreement.

**D8** replaces `FeedMeta.updated_rfc3339: String` and
`MediaLinkEntry.{published,updated}_rfc3339: String` with
`common::time::UtcInstant` (ADR-0072/0063), which both producers already hold as
`DateTime<Utc>` and currently stringify via `.to_rfc3339()` (`posts.rs:189-191`,
`media.rs:55`). This deletes a stringify/reparse round trip rather than adding
one, and removes the only genuinely fallible step the serializers would
otherwise have. Timestamps are the one conspicuous non-newtype holdout on
`PostRecord`; migrating the **storage records** themselves is a separate
vertical with its own dual-backend sqlx risk and is filed as a follow-up issue
by the plan's first task, not done here.

**D9**: `impl From<AtomPubError> for HandlerError` maps unconditionally to
`BadRequest`, documented "A malformed AtomPub document supplied by the client is
a 400" (`server/src/atompub/mod.rs:208-213`). A bare `?` on `render_feed` inside
a GET handler would turn a server-side failure into a client 400. So
`AtomPubError` gains a serialization variant distinct from `Malformed`, and the
`HandlerError` conversion maps it to 500.

## Scope

### Part A — retire the fork bridge (#199)

Execute ADR-0043's "Exit / how to drop this" in full:

- Delete both `[patch.crates-io]` entries from the root `Cargo.toml`.
- Delete the `atom-fork` / `rss-fork` flake inputs, their `flake.lock` entries,
  and the `cargoVendorDir` / `overrideVendorGitCheckout` wrapper in `flake.nix`.
- Remove `jaunder-org` from `deny.toml`'s `[sources.allow-org].github`.
- Raise `common/Cargo.toml` to `atom_syndication = "0.12.10"` and `rss = "2.1"`,
  **keeping `default-features = false`** on `atom_syndication` (upstream's
  default feature set is `["builders"]`, which we do not use), and regenerate
  `Cargo.lock`.
- `common` keeps its **direct** `quick-xml = "0.41"` dependency — `service.rs`,
  `categories.rs`, `rsd.rs`, and `atompub/xml.rs` still drive `quick-xml` for
  the `app:`-namespace and RSD documents that `atom_syndication` does not model.

Archiving the two GitHub forks is repo administration outside this branch; it is
recorded in the ADR as a follow-up rather than performed here.

### Part B — delegate Atom I/O (#737)

- `entry_from_xml` → `Entry::read_from`, mapping `atom_syndication::Error` onto
  `AtomPubError::Malformed` via a `From` impl.
- `entry_to_xml` → `Entry::write_to`.
- `render_feed` → build an `atom_syndication::Feed` (id, title, updated, the RFC
  5005 paging links, `set_entries`, `set_namespaces`) and call its `write_to`.
- `render_media_link_entry` → build an `Entry` whose `Content` carries `src` and
  `content_type` with no value, then `write_to`.
- `FeedMeta`/`MediaLinkEntry` timestamp fields become `UtcInstant` (D8); the two
  producers pass `record.updated_at.into()` instead of `.to_rfc3339()`.
- `AtomPubError` gains the D9 serialization variant;
  `From<AtomPubError> for HandlerError` maps it to 500.
- Delete the now-unreachable machinery: `Parser`, `Acc`, `Field`, `build_entry`,
  `read_xhtml_content`, `resolve_ref`, `decode_text`, `local_name`,
  `local_name_end`, `attr_value`, `capture_link`, `append`, `trimmed`,
  `parse_dt`, and `xml.rs`'s `write_link` (its last caller is `entry.rs`;
  `write_text_element` and `write_empty_element` keep
  `service.rs`/`categories.rs` callers).
- Delete `impl From<quick_xml::Error> for AtomPubError` and
  `impl From<std::io::Error> for AtomPubError` (`mod.rs:49-59`):
  `entry_from_xml` is their only user, and the other serializers discard writer
  errors with `let _ =`. Leaving them would be uncovered production code.
- `post_entry_response` (`server/src/atompub/posts.rs:438-457`) currently
  returns `Response`, not `Result`; D3 changes it to
  `Result<Response, HandlerError>` and its two callers gain a `?`. The other
  five call sites (`posts.rs:204/271/541`, `media.rs:123/167`) are already
  inside `Result`-returning handlers.
- Keep the `is_draft` / `set_draft` / `j_slug` / `set_j_slug` helpers:
  `app:control/app:draft` is not modeled first-class by `atom_syndication`, and
  `j:` is jaunder's own. Their extension round-trip through upstream is
  verified: `extension_name("app:control")` yields `("app","control")`, children
  are keyed by local name, and `Extension::to_xml` writes the qualified
  `Extension.name`.
- Keep every currently-exported name in `common::atompub`.

### Out of scope

- Any change to `service.rs`, `categories.rs`, or `rsd.rs` beyond deleting
  `write_link`.
- Any change to the elisp client or to `end2end/tests/atompub.spec.ts`
  assertions.
- Migrating `PostRecord`/`MediaRecord` timestamps to `UtcInstant` — filed as a
  follow-up issue by the plan's first task.

## Known deltas, and why each is safe

Verified against every consumer before accepting D2. Consumers are
layout-insensitive: the e2e spec asserts substrings, and the elisp client parses
with `libxml-parse-xml-region`, matching by local name.

| Delta                                  | Today                                          | After                                             | Why safe                                                                                                                                                                                                                                                                                                                        |
| -------------------------------------- | ---------------------------------------------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| XML declaration                        | `<?xml version="1.0" encoding="utf-8"?>`       | `<?xml version="1.0"?>` + newline                 | UTF-8 is XML's default and responses carry a `Content-Type`; no consumer reads the decl.                                                                                                                                                                                                                                        |
| Element order                          | id, title, updated, published, …               | title, id, updated, authors, categories, links, … | Order is not significant in Atom.                                                                                                                                                                                                                                                                                               |
| `<content>` with no `type`             | defaults to `Some("text")`                     | stays `None`                                      | `wire_to_format(None, d)` and `wire_to_format(Some("text"), d)` both fall through to `d`.                                                                                                                                                                                                                                       |
| `<link>` with no `rel`                 | defaults to `"alternate"`                      | unchanged                                         | `impl Default for Link` already sets `rel: "alternate"`.                                                                                                                                                                                                                                                                        |
| Media-link `<content>`                 | self-closing `<content …/>`                    | paired `<content …></content>`                    | `Content::to_xml` always writes Start+End. elisp reads `dom-attr content 'src`; e2e never asserts the form.                                                                                                                                                                                                                     |
| Feed-embedded entries                  | carry no `xmlns:*`                             | redeclare `xmlns:j`, and `xmlns:app` when a draft | `to_xml_inner` writes `self.namespaces` regardless of `declare_xmlns`. Valid XML; the existing test checks only that the _default_ `xmlns` is not redeclared.                                                                                                                                                                   |
| xhtml literal `'` `"` `>`              | passed through verbatim                        | escaped to `&apos;` `&quot;` `&gt;`               | Upstream's `atom_xhtml` applies `escape()` to text events. Semantically identical once parsed; the body is served as HTML source.                                                                                                                                                                                               |
| xhtml surrounding whitespace           | trimmed                                        | preserved                                         | Irrelevant to HTML rendering. Title/summary are unaffected: `PostTitle::from` and `PostSummary::from_str` both trim at the domain boundary.                                                                                                                                                                                     |
| Unsupported entity ref (`&amp;bogus;`) | `Malformed` → 400                              | passed through literally as `&amp;bogus;`         | The only delta that **loosens**. `atom_text` resolves predefined entities, then char refs, then re-emits the reference verbatim. A non-conforming document now yields a post containing the literal text rather than a rejection — lenient ingest, consistent with R5's treatment of bad category terms and over-cap summaries. |
| Malformed-document error               | `Malformed("document has no <entry> element")` | `Malformed` wrapping `InvalidStartTag` / `Eof`    | Both are `AtomPubError::Malformed` → 400. Unchanged status mapping.                                                                                                                                                                                                                                                             |

**Not a delta**, contrary to an earlier draft of this spec: xhtml **entity
references** round-trip identically. Today's `read_xhtml_content` resolves
`&amp;` to `&` and then writes it with `BytesText::new`, which re-escapes
(`quick-xml` 0.41 `events/mod.rs:580-582` →
`Self::from_escaped(escape(content))`), so `b &amp; c` is already stored as
`b &amp; c`. Upstream re-emits the reference verbatim. Same result.

### Parse strictness (D6)

Upstream rejects three classes of input that today's reader silently tolerates,
and **accepts one that today's rejects**. All four are accepted as the new
behavior, pinned by tests, and documented here. The three narrowings:

1. An unparseable `<updated>`/`<published>` rejects the whole entry (400). Today
   `parse_dt` is `.ok()` and the bad value is ignored.
2. A `<title type="…">`/`<summary type="…">` outside `text|html|xhtml` rejects
   the whole entry. Today the attribute is ignored.
3. Namespace-prefixed atom elements (`<atom:title>`) route into `extensions`
   rather than being matched by local name, so the field is lost; a prefixed
   `<atom:entry>` **root** is rejected outright. Today `local_name()` matching
   accepts both.

**(2) does not touch org/markdown support.** The ADR-0023 format carrier is
`<content type="text/org">` / `text/markdown`, and `Content::from_xml` keeps
`type` as a raw `Option<String>` with no validation — only `Text::from_xml`
(backing `<title>`, `<summary>`, `<rights>`) parses into `TextType`. That split
is RFC 4287's, not upstream's: `atomTextConstruct` restricts `type` to
`text|html|xhtml`, while `atomContent` permits a media type.
`jaunder-atom.el:48-55` matches — it emits `<title>`/`<summary>` with no
attributes and attaches the media type only to `<content>`. The residual
exposure is a third-party client putting a media type on `<title>`, which is
non-conforming today and would newly 400.

The one loosening: an **unsupported entity reference** (`&bogus;`) is a 400
today and is now passed through literally into the field's text. Accepted as
lenient ingest, consistent with R5 elsewhere in `mapping.rs` — a single bad
reference no longer fails the whole entry.

(1) and (2) turn silently-wrong data into an honest 400. (3) is a genuine
interop narrowing — a conforming client using the prefixed form would now get a
400 — accepted because our own elisp client emits the default-namespace form and
preserving it would require the bespoke reader this work deletes. It is called
out in the ADR.

## Acceptance criteria

Each is observable — a reviewer can check it without reading intent.

**Part A**

- A1. `rg '\[patch.crates-io\]' Cargo.toml` returns nothing; neither
  `jaunder-org/atom` nor `jaunder-org/rss` appears in `Cargo.toml`, `flake.nix`,
  or `flake.lock`.
- A2. `deny.toml` has no `jaunder-org` entry under `[sources.allow-org]`.
- A3. `flake.nix` defines no `cargoVendorDir` / `overrideVendorGitCheckout`
  override.
- A4. `Cargo.lock` resolves `atom_syndication` to a registry `0.12.10` (or later
  0.12.x) and `rss` to a registry `2.1.x`, with exactly one `quick-xml`, at
  `>= 0.41`.
- A5. `common/Cargo.toml` still carries `default-features = false` on
  `atom_syndication`.
- A6. `cargo xtask validate --no-e2e` passes with the advisories check green and
  no `[advisories].ignore` entry for RUSTSEC-2026-0194/0195. (The ignore is
  already absent on `main`, so this is a regression guard; the load-bearing half
  is the gate.)

**Part B**

- B1. `rg 'quick_xml' common/src/atompub/entry.rs` returns nothing.
- B2. `entry_to_xml`, `render_feed`, and `render_media_link_entry` each return
  `Result<String, AtomPubError>`; `post_entry_response` returns
  `Result<Response, HandlerError>`; every call site propagates with `?`.
- B3. A round-trip test asserts an entry carrying title, summary, html content,
  two categories, links, published/updated timestamps, the draft marker, and a
  `j:slug` survives `entry_to_xml` → `entry_from_xml` with every field intact.
- B4. `entry_to_xml` on a draft entry with a slug emits both `xmlns:app` and
  `xmlns:j`; an entry with neither marker emits neither declaration;
  `set_draft(&mut e, false)` removes the `app` declaration along with the marker
  (D4).
- B5. Tests assert `AtomPubError::Malformed` for each D6 case: an unparseable
  `<updated>`, a `<title type="bogus">`, and a root element that is not
  `<entry>`; plus a test asserting a `<atom:title>`-prefixed entry does **not**
  populate the title (documenting the accepted narrowing).
- B6. A test asserts a literal `'` inside `<content type="xhtml">` round-trips
  as `&apos;`, pinning the accepted escaping delta.
- B6a. A test asserts an unsupported entity reference (`&bogus;`) in a `<title>`
  is accepted and lands in the title as the literal text `&bogus;`, pinning the
  one loosening delta. A surrogate char-ref (`&#xD800;`) still errors.
- B7. `render_feed` output contains the `<feed>` root with `xmlns`, the RFC 5005
  `self`/`first`/`previous`/`next` links when present and none of the optional
  ones when absent, and one `<entry>` per input entry.
- B8. `render_media_link_entry` output carries a `<content>` element with `type`
  and `src` attributes, both `rel="edit"` and `rel="edit-media"` links, and a
  `<title>` holding the **decoded** filename (#720).
- B9. `FeedMeta` and `MediaLinkEntry` expose `UtcInstant` fields;
  `rg '_rfc3339:' common/src server/src` returns nothing. (Match the
  field-declaration colon — a bare `_rfc3339` also catches `to_rfc3339()` call
  sites and test names, which this criterion is not about.)
- B10. A serialization failure maps to a 500, not a 400 — asserted on the
  `From<AtomPubError> for HandlerError` conversion (D9).
- B11. `common/src/atompub/mod.rs` declares no `From<quick_xml::Error>` and no
  `From<std::io::Error>` for `AtomPubError`.

**Both**

- C1. `cargo xtask validate --no-e2e` is green.
- C2. `cargo xtask e2e-local atompub.spec.ts` is green.
- C3. A new ADR draft exists in `docs/adr/drafts/` covering both halves, and
  `docs/adr/0043-quick-xml-fork-patch.md` has `Status: superseded` referencing
  it.
- C4. `common/src/atompub/entry.rs`'s module doc no longer claims
  `atom_syndication`'s entry I/O is crate-private.
- C5. A follow-up issue exists for migrating `PostRecord`/`MediaRecord`
  timestamps to `UtcInstant`.

## Appendix — disposition of the cold spec review

| Finding                        | Disposition                                                                                                                                                                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| F1                             | **Confirmed and folded.** Verified `BytesText::new` escapes (quick-xml 0.41 `events/mod.rs:580`). The "behavior fix" was false; D6 rewritten, the bogus repair line dropped, and the real (opposite-direction) `escape()` delta added as a row with test B6. |
| F2                             | Folded. B8 now expects a paired `<content>`; added as a delta row.                                                                                                                                                                                           |
| F3                             | Folded. C4 now names `entry.rs`, where the stale claim actually lives.                                                                                                                                                                                       |
| F4                             | Folded. `post_entry_response`'s signature change is called out in Scope and in B2.                                                                                                                                                                           |
| F5                             | Folded as D9 — a distinct error variant mapping to 500, asserted by B10.                                                                                                                                                                                     |
| H1                             | Folded as D8 — `UtcInstant` fields, with the record-level migration split out per C5.                                                                                                                                                                        |
| H2                             | Folded. D4 no longer claims a slug clear exists; B4 drops that clause.                                                                                                                                                                                       |
| H3                             | Folded as the "Parse strictness" section and D6, pinned by B5.                                                                                                                                                                                               |
| H4                             | Folded as two delta rows; impact is nil for title/summary (both domain newtypes trim) and cosmetic for xhtml bodies.                                                                                                                                         |
| H5                             | Folded. Both `From` impls are deleted; B11 asserts it.                                                                                                                                                                                                       |
| H6                             | Folded. `default-features = false` is explicit in Part A and asserted by A5.                                                                                                                                                                                 |
| H7                             | Folded into the feed-embedded-entries delta row.                                                                                                                                                                                                             |
| B5 fixture inconsistency       | Moot — the old B5 is gone with D6's rewrite; the new B5/B6 test distinguishable behavior.                                                                                                                                                                    |
| A5 already-true                | Folded — split into A6 with the regression-guard caveat stated.                                                                                                                                                                                              |
| rss 2.1.0 unverifiable offline | Resolved — crates.io reports `rss` 2.1.0 requiring `quick-xml ^0.41`.                                                                                                                                                                                        |
