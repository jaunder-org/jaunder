# Spec — `RenderedHtml` newtype (issue #398)

## Goal

Give the finished, ready-to-emit HTML of a post its own type, `RenderedHtml`,
instead of a bare `String`. The type marks the **provenance** boundary "this
HTML came out of `common::render::render()`", and the one place in the UI that
emits HTML **unescaped** is made to accept only a `RenderedHtml` — never a
`String`. Dropping a raw `body` (or any other `String`) into that sink then
becomes a **compile error**.

This is a type-safety change (ADR-0063, trust/safety-boundary axis). It hardens
the boundary against _new_ holes; it does not change runtime behavior.

## Scope boundary — provenance, not sanitization

`render()` performs **no** HTML sanitization: `PostFormat::Html` is a raw
passthrough, and the Markdown/Org paths pass embedded raw HTML through
untouched. So the honest guarantee `RenderedHtml` carries is **"produced by
`render()`"**, **not** "safe / XSS-free". The type's doc comment must say so
plainly and must **not** claim "safe to emit unescaped".

The live stored-XSS hole this exposes (a post body containing `<script>` is
served verbatim) is **out of scope here** and tracked separately in **#445**
("Rendered post HTML is emitted unsanitized (stored XSS)", P1), which is
`blocked-by` #398 so its scrubber can attach to the single mint point this issue
establishes.

## The path being typed (verified)

| Stage               | Site                                                                                                                     | Today                         | After                                 |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------ | ----------------------------- | ------------------------------------- |
| **Mint**            | `common::render::render(&str, &PostFormat)` (`common/src/render.rs:56`)                                                  | `-> String`                   | `-> RenderedHtml`                     |
| Mint calls          | `storage::post_service` lines 75, 160, 322                                                                               | 3 call sites                  | unchanged shape                       |
| **Store**           | `CreatePostInput` / `UpdatePostInput` (`storage/src/posts.rs:196,218`)                                                   | `rendered_html: String`       | `RenderedHtml`                        |
| **Record**          | `PostRecord` / `PostRevisionRecord` (`storage/src/posts.rs:56,120`)                                                      | `String`                      | `RenderedHtml`                        |
| **DB read**         | `build_post_record` (`storage/src/helpers.rs:165`)                                                                       | moves `String`                | rebuild via `from_trusted`            |
| **SQL bind**        | `sqlite/posts.rs:92`, `postgres/posts.rs:92`                                                                             | `.bind(&input.rendered_html)` | `.bind(input.rendered_html.as_ref())` |
| **Wire**            | `PostResponse` (`web/src/posts/mod.rs:184`), `TimelinePostSummary` (`web/src/posts/listing.rs:40`)                       | `String`                      | `RenderedHtml`                        |
| **View**            | `PostView.rendered_html` (`web/src/render/mod.rs:413`); fed at `render/mod.rs` (`:355`,`:396`) **and** `pages/ui.rs:301` | `&'a str`                     | `&'a RenderedHtml`                    |
| **Emit**            | `render_post_content` (`web/src/render/mod.rs:497`) → `inner_html=`                                                      | raw `{body}`                  | `Display` of `&RenderedHtml`          |
| **Read (non-emit)** | `web/src/media/mod.rs:154` — `post.rendered_html.contains(&url)`                                                         | `str::contains`               | `.as_ref().contains(&url)`            |

To make the **emit sink** reject a `String` (acceptance), the type must reach
`PostView`; `PostView` borrows from the wire DTOs, so the DTOs must own a
`RenderedHtml`, so the type must survive the server→client **serde** round-trip.

## Design decisions

### 1. Type home and shape

- `pub struct RenderedHtml(String);` in `common::render`, beside `render()`, so
  the only _new-value_ constructor stays private to that module.
- Derives: `Clone, Debug, PartialEq, Eq` (and `Hash` if needed). **Not**
  `StrNewtype` — that macro emits `TryFrom<String>` / `From` / `Deref<str>` /
  `Deserialize`, i.e. exactly the parse-from-any-string constructors the issue
  forbids.
- **Reading trailer** (ADR-0063): `Display` (emit interpolation), `AsRef<str>`
  (SQL binds), and `Deref<Target = str>` (so `str` methods like `.contains()`
  work without `.as_ref()`). The construction prohibition is what protects the
  boundary: deliberately **no** `From`, `TryFrom<String>`, `FromStr`,
  `Deserialize` — a raw `String` can never become a `RenderedHtml`. `Deref` is a
  _reading_ convenience and is one-way (it coerces `RenderedHtml → &str`, never
  the reverse), so it does not weaken the sink. (Post-review addition; the
  original draft omitted `Deref`.)

### 2. Mint discipline — tight

`render()` is the only door that creates a **new** `RenderedHtml`. Persistence
and transport reconstruct an already-minted value, so they get a small number of
**named, documented, greppable** "trusted rebuild" doors — no general
string→`RenderedHtml` constructor exists:

- **One rebuild door:** a single `pub` constructor whose name states the
  contract, `RenderedHtml::from_trusted(String)` — doc: "reconstruct a value
  previously produced by `render()` and round-tripped through our own storage or
  wire; the caller asserts that provenance." Used by both the DB and wire paths
  so there is exactly one rebuild constructor to audit.
- **DB rehydration:** `build_post_record` calls `from_trusted` on the DB string.
- **Wire:** `RenderedHtml` implements `Serialize` (reading out is always safe).
  It does **not** implement `Deserialize`. The DTO `rendered_html` fields carry
  `#[serde(deserialize_with = "…")]` pointing at a thin helper that reads a
  `String` and delegates to `from_trusted`.

Mint sites after this change (the complete auditable set): `render()` (creates
new HTML) and `RenderedHtml::from_trusted` (round-trip rebuild; the wire
`deserialize_with` helper delegates to it). Both are greppable by name — nothing
is hidden behind a blanket derive.

**Enforcement (post-review addition).** Because `from_trusted` is `pub` and
cross-crate, Rust visibility cannot confine it — a future call laundering an
untrusted string into "trusted" HTML would compile. A bespoke xtask static check
`rendered-html-from-trusted` (a `syn` AST pass modelled on
`server-fn-registrar`/`proffered-invite-code`) pins every **non-test**
`from_trusted` mention to an allowlist of enclosing functions
(`build_post_record`, `deserialize_rendered_html`); a new site fails the gate
until it is added with justification. This turns the "few, auditable doors"
guarantee from a convention into an enforced invariant.

### 3. Threading

- Storage: the four struct fields become `RenderedHtml`. `CreatePostInput`/
  `UpdatePostInput` bind into SQL via `AsRef<str>` at **both** backends
  (`sqlite/posts.rs:92`, `postgres/posts.rs:92`). `PostRecord` is rebuilt from
  the DB string via `from_trusted` in `build_post_record` — the **only** DB-read
  door. `PostRevisionRecord` has **no** Rust build/read path (its field is
  written DB-to-DB by an `INSERT … SELECT`; never constructed or read in Rust),
  so retyping its field to `RenderedHtml` is inert and needs no rebuild door.
- Web DTOs: `PostResponse` / `TimelinePostSummary` fields become `RenderedHtml`
  with the `deserialize_with` attribute; server-side construction
  (`web/src/posts/server.rs`) moves the `RenderedHtml` straight from
  `PostRecord` (no conversion).
- View: `PostView.rendered_html: &'a RenderedHtml` (fed at
  `render/mod.rs:355,396` and `pages/ui.rs:301`); `render_post_content`
  interpolates it via `Display`, so the raw-HTML sink is now typed. The one
  non-emit reader, `media/mod.rs:154`, switches to `.as_ref().contains(&url)`.
- Callers of `render()` (3 in `post_service`) are unchanged in shape (they
  already move the result into a `*PostInput`).

### 4. Test surface

The test-literal churn is the **dominant cost** of this change (~100+ edit
sites), and is the intended friction — a raw `String` can no longer be dropped
into the `rendered_html` slot. Sites:

- The existing XSS-boundary test
  `test_perform_post_creation_org_title_rendered_once` (`post_service.rs:1043`)
  must still pass; its `record.rendered_html.contains(…)` becomes
  `record.rendered_html.as_ref().contains(…)`.
- `render()`'s own unit tests (`render.rs:480,486,655`) compare against
  `String`; adjust via `.as_ref()` / `.to_string()`.
- **~109 struct-literal sites** build `PostRecord`/`*PostInput`/DTOs with a
  `rendered_html:` `String` literal (`.to_string()`/`format!`). Concentrated in
  `server/tests/storage/mod.rs` (**67**), plus `feed_worker.rs` (7),
  `web/posts/mod.rs` (4), `feed_regenerate.rs` (4), `feed_handlers.rs` (3), and
  the `web_posts`/`web_tags`/`web_media`/`projector`/`backup_fixture` tests.
  These rebuild via `RenderedHtml::from_trusted` (they are fixtures, not
  `render()` calls). Includes a field **mutation**
  `p.rendered_html = "…".to_string()` at `server/src/atompub/posts.rs:620` that
  must assign a `RenderedHtml`.
- **6 `PostView` `&str` literal sites** in `web/src/render/mod.rs` (827, 881,
  902, 1204, 1223, 1249) pass `"<p>b</p>"`/`.into()` for `rendered_html`; with
  the field now `&'a RenderedHtml` each must bind an owned `RenderedHtml` (via
  `render()`/`from_trusted`) to a local and borrow it.

A test-only ergonomic helper (e.g. a `from_trusted`-backed `rendered_html!`/
`fixture` shim) may be worth adding to cut this churn — the plan decides.

## Acceptance (from the issue)

- `rendered_html` is `RenderedHtml`, constructible only via `render()` (plus the
  single named trusted-rebuild door `from_trusted`, used by the DB and wire
  round-trips).
- The unescaped view sink takes `RenderedHtml`; emitting a raw `String`/`body`
  there is a **compile error**.
- `cargo xtask validate --no-e2e` clean; the XSS-boundary test still passes.

## Non-goals

- HTML sanitization / closing the stored-XSS hole → **#445**.
- Any change to what `render()` outputs, or to the emitted markup.
- No new ADR: ADR-0063 already defines the convention and names `RenderedHtml`.
