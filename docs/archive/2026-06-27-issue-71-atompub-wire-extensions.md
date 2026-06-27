# AtomPub Wire Extensions + Server-Side Org Canonicalization (issue #71) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one blog mix Org/Markdown/HTML per entry over AtomPub (format in the standard `atom:content` `type` as a media type), expose the server slug as a read-only `j:slug` on every entry, advertise capabilities via `j:extension` in the service document, and normalize every ingested Org body to one canonical form (strip `#+TITLE:`, preserve everything else).

**Architecture:** A `format_wire` seam (two pure functions) is the single, reversible coupling point for format↔wire. `j:slug` reuses the existing `atom_syndication` extensions map exactly as `app:draft` does, emitted by `write_entry` with a conditional `xmlns:j`. Capability discovery adds a `j:extension` child to the service document. Org canonicalization adds one pure `canonicalize_org_body` applied at the two storage seams (`perform_post_creation`/`perform_post_update`), so web and AtomPub converge on one stored body.

**Tech Stack:** Rust, `atom_syndication` (Entry/Extension/ExtensionMap), `quick-xml` (Writer), `orgize`, `axum`, `rstest` test templates.

## Global Constraints

- **Spec (binding):** `docs/superpowers/specs/2026-06-16-emacs-blogging-frontend-design.md` — section **"Unit B"**; plus **ADR-0023** (wire extensions) and **ADR-0024** (server-side org canonicalization), both `accepted`. ADR-0015 already marked superseded for its token scheme. **No new ADR is required.**
- **No legacy data exists** (user-confirmed 2026-06-27): canonicalization applies on the write path only; **no backfill / migration / strip-on-next-write provision** is written.
- **Format wire mapping (ADR-0023):** `Org`↔`text/org`, `Markdown`↔`text/markdown`, `Html`↔`html` token (NOT `text/html`). Incoming: lenient — `text/org`→Org, `text/markdown`[`;param`]→Markdown, `html`/`xhtml`/`text/html`→Html, bare `text`/absent/unknown→account `default_post_format`.
- **`j:slug`:** namespace `https://jaunder.org/ns/atompub`, read-only, emitted on **every** entry (draft/scheduled/live); incoming `j:slug` ignored; `xmlns:j` declared only when emitted (mirroring `xmlns:app`).
- **Org canonicalization (ADR-0024):** strip **only** recognized headers (`#+TITLE:` → title column); **preserve unrecognized `#+FOO:` verbatim**; byte-deterministic and idempotent. No `#+KEYWORDS:`/`#+DESCRIPTION:` parsing (that is follow-on **#77**, already filed; #71 blocks it).
- **Backend coverage via one common test** (`#[apply(backends)] #[tokio::test] async fn NAME(#[case] backend: Backend)`) for storage/integration; pure functions get in-file `#[cfg(test)]` unit tests. Server test crate package is **`jaunder`**; Postgres `case_2` panics under bare nextest (expected) — controller's Nix gate covers both.
- **No `Co-Authored-By` trailers.** **Worktree only / never `main`:** commit on `worktree-issue-71-atompub-wire-extensions`; review against `wt-base-issue-71`.
- **Per-task gate:** `cargo xtask check --no-test` while iterating; `cargo xtask validate --no-e2e` before each commit; full `cargo xtask validate` (with e2e) at the ship boundary.

## Separable concerns surfaced during investigation

None new. Full Org header-block parsing (raw-Org web authoring) is follow-on **#77** (already filed; #71 blocks it). No new GitHub issues filed by this plan.

## File map

- **Modify** `common/src/atompub/mod.rs` — add `J_NS` constant.
- **Modify** `common/src/atompub/entry.rs` — `j_slug`/`set_j_slug` helpers; `write_entry` + `render_feed` emit `j:slug` and conditionally declare `xmlns:j`.
- **Modify** `server/src/atompub/mapping.rs` — `format_wire` seam (`wire_to_format`/`format_to_wire`); wire into `entry_to_post_fields`/`post_to_entry`; set `j:slug` on outgoing entries.
- **Modify** `common/src/atompub/service.rs` — `ServiceDocument` gains an extension marker; `render_service_document` emits `<j:extension>` + `xmlns:j`.
- **Modify** `common/src/render.rs` — add `canonicalize_org_body`.
- **Modify** `storage/src/post_service.rs` — apply `canonicalize_org_body` at both `derive_post_metadata` call sites (creation line ~307, update line ~195).
- **Test** in-file (`mapping.rs`, `entry.rs`, `service.rs`, `render.rs`, `post_service.rs`) + `server/tests/atompub/atompub_posts.rs`, `server/tests/atompub/atompub_service.rs`.

---

## Task 1: `format_wire` seam — per-entry format media types

**Files:**
- Modify: `server/src/atompub/mapping.rs` (`entry_to_post_fields` 33–77, `post_to_entry` 92–96, in-file tests from line 139)
- Test: in-file `#[cfg(test)]` in `mapping.rs`; `server/tests/atompub/atompub_posts.rs`

**Interfaces:**
- Produces:
  ```rust
  fn wire_to_format(content_type: Option<&str>, default: PostFormat) -> PostFormat;
  fn format_to_wire(format: PostFormat) -> &'static str;
  ```
  `format_to_wire`: `Org=>"text/org"`, `Markdown=>"text/markdown"`, `Html=>"html"`. `wire_to_format`: split off any `;`-parameter and trim/lowercase the media type, then `text/org=>Org`, `text/markdown=>Markdown`, `html|xhtml|text/html=>Html`, else (`text`, absent, unknown) `=>default`.

- [x] **Step 1: Write failing unit tests** in `mapping.rs` `mod tests`:
```rust
#[test]
fn format_wire_round_trips_every_format() {
    for f in [PostFormat::Org, PostFormat::Markdown, PostFormat::Html] {
        let wire = format_to_wire(f.clone());
        assert_eq!(wire_to_format(Some(wire), PostFormat::Markdown), f, "round-trip {wire}");
    }
}
#[test]
fn wire_to_format_is_lenient() {
    let d = PostFormat::Html; // distinctive default
    assert_eq!(wire_to_format(Some("text/org"), d.clone()), PostFormat::Org);
    assert_eq!(wire_to_format(Some("text/markdown"), d.clone()), PostFormat::Markdown);
    assert_eq!(wire_to_format(Some("text/markdown; variant=GFM"), d.clone()), PostFormat::Markdown);
    assert_eq!(wire_to_format(Some("html"), PostFormat::Org), PostFormat::Html);
    assert_eq!(wire_to_format(Some("xhtml"), PostFormat::Org), PostFormat::Html);
    assert_eq!(wire_to_format(Some("text/html"), PostFormat::Org), PostFormat::Html);
    assert_eq!(wire_to_format(Some("text"), d.clone()), d.clone());      // bare text → default
    assert_eq!(wire_to_format(None, d.clone()), d.clone());              // absent → default
    assert_eq!(wire_to_format(Some("application/x-weird"), d.clone()), d); // unknown → default
}
```

- [x] **Step 2: Run → FAIL** — `cd <worktree> && cargo nextest run -p jaunder -p common format_wire wire_to_format` (functions undefined). (mapping.rs is in the `jaunder` crate.)

- [x] **Step 3: Implement the seam** in `mapping.rs`:
```rust
/// The wire `atom:content` `type` for a post format (ADR-0023). `Html` uses the
/// `html` token (markup), NOT `text/html` (which would mean escaped text).
fn format_to_wire(format: PostFormat) -> &'static str {
    match format {
        PostFormat::Org => "text/org",
        PostFormat::Markdown => "text/markdown",
        PostFormat::Html => "html",
    }
}

/// Lenient inverse: never fails, falls back to `default` for `text`/absent/unknown
/// so reading is robust to any client. Tolerates a media-type parameter.
fn wire_to_format(content_type: Option<&str>, default: PostFormat) -> PostFormat {
    let Some(ct) = content_type else { return default };
    let base = ct.split(';').next().unwrap_or(ct).trim().to_ascii_lowercase();
    match base.as_str() {
        "text/org" => PostFormat::Org,
        "text/markdown" => PostFormat::Markdown,
        "html" | "xhtml" | "text/html" => PostFormat::Html,
        _ => default,
    }
}
```

- [x] **Step 4: Wire into `entry_to_post_fields`** — replace the `let format = if matches!(ctype, Some("html" | "xhtml")) {...} else {default_format}` block with `let format = wire_to_format(ctype, default_format);`.

- [x] **Step 5: Wire into `post_to_entry`** — replace the `match post.format { Html=>("html",..), Markdown|Org=>("text",..) }` with `let content_type = format_to_wire(post.format.clone()); let body = post.body.clone();`.

- [x] **Step 6: Update existing in-file tests + add coverage.** `post_to_entry_org_format_becomes_text_content` → assert content type `text/org`; `post_to_entry_markdown_format_becomes_text_content` → `text/markdown`; keep `..._html_format_becomes_html_content` = `html`. Add `entry_to_post_fields_text_org_is_org` and `..._text_markdown_is_markdown` (content type → format). Keep `entry_to_post_fields_text_content_uses_default_format` (bare `text` → default).

- [x] **Step 7: Add an AtomPub integration test** in `server/tests/atompub/atompub_posts.rs` (`#[apply(backends)]`): POST an entry with `type="text/org"`, GET it back, assert the stored format is Org (the round-tripped member shows `type="text/org"`); POST `type="text/markdown"` → member shows `type="text/markdown"`. Use the existing `entry_xml(title, content_type, content)` helper and `member` GET pattern.

- [x] **Step 8: Gate + commit** — `cargo xtask validate --no-e2e` green; commit `feat(atompub): per-entry format via content media-type (format_wire seam) (#71)`. (`format_to_wire` takes `&PostFormat` — clippy `needless_pass_by_value`. Also updated two pre-existing integration assertions to the new `text/markdown` wire type.)

---

## Task 2: `j:slug` foreign-markup element

**Files:**
- Modify: `common/src/atompub/mod.rs` (`J_NS` const near `ATOM_NS`/`APP_NS`, lines 34–37)
- Modify: `common/src/atompub/entry.rs` (`j_slug`/`set_j_slug` near `is_draft`/`set_draft`; `write_entry` 427–467; `render_feed` root 502–505)
- Modify: `server/src/atompub/mapping.rs` (`post_to_entry` sets slug; `entry_to_post_fields` ignores incoming)
- Test: in-file `entry.rs`; `server/tests/atompub/atompub_posts.rs`

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces:
  ```rust
  pub const J_NS: &str = "https://jaunder.org/ns/atompub";
  #[must_use] pub fn j_slug(entry: &Entry) -> Option<String>;
  pub fn set_j_slug(entry: &mut Entry, slug: &str);
  ```

- [x] **Step 1: Write failing unit tests** in `entry.rs` `mod tests`:
```rust
#[test]
fn set_and_read_j_slug_round_trips() {
    let mut entry = sample_entry();
    set_j_slug(&mut entry, "my-post");
    assert_eq!(j_slug(&entry), Some("my-post".to_string()));
}
#[test]
fn j_slug_is_serialized_with_namespace() {
    let mut entry = sample_entry();
    set_j_slug(&mut entry, "my-post");
    let out = entry_to_xml(&entry);
    assert!(out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#), "out: {out}");
    assert!(out.contains("<j:slug>my-post</j:slug>"), "out: {out}");
}
#[test]
fn no_j_slug_means_no_namespace_declared() {
    let entry = sample_entry();
    let out = entry_to_xml(&entry);
    assert!(!out.contains("xmlns:j"), "out: {out}");
}
```

- [x] **Step 2: Run → FAIL** — `cd <worktree> && cargo nextest run -p common j_slug`.

- [x] **Step 3: Add `J_NS`** to `common/src/atompub/mod.rs` (next to `APP_NS`):
```rust
/// Jaunder foreign-markup namespace (ADR-0023): `j:slug`, `j:extension`.
pub const J_NS: &str = "https://jaunder.org/ns/atompub";
```
Import it in `entry.rs` (`use super::{AtomPubError, APP_NS, ATOM_NS, J_NS};`).

- [x] **Step 4: Implement helpers** in `entry.rs` (mirror `set_draft`/`is_draft`; the extensions map is prefix→localname→Vec<Extension>):
```rust
/// Read the read-only server slug from a `j:slug` extension, if present.
#[must_use]
pub fn j_slug(entry: &Entry) -> Option<String> {
    entry.extensions.values().find_map(|by_local| {
        by_local.get("slug").and_then(|exts| exts.first()).and_then(|e| e.value.clone())
    })
}

/// Set (idempotently replace) the `j:slug` extension. Emitted on every outgoing
/// entry; the server never reads an incoming one.
pub fn set_j_slug(entry: &mut Entry, slug: &str) {
    for by_local in entry.extensions.values_mut() {
        by_local.remove("slug");
    }
    entry.extensions.retain(|_, by_local| !by_local.is_empty());
    let ext = Extension {
        name: "j:slug".to_string(),
        value: Some(slug.to_string()),
        attrs: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    entry
        .extensions
        .entry("j".to_string())
        .or_default()
        .insert("slug".to_string(), vec![ext]);
}
```

- [x] **Step 5: Emit in `write_entry`** — (a) conditional namespace: after the `xmlns:app` block, add
```rust
if j_slug(entry).is_some() {
    root.push_attribute(("xmlns:j", J_NS));
}
```
  (b) emit the element near the draft block:
```rust
if let Some(slug) = j_slug(entry) {
    write_text_element(writer, "j:slug", &slug);
}
```
  In `render_feed`, the feed root already declares `xmlns`/`xmlns:app` unconditionally; add `root.push_attribute(("xmlns:j", J_NS));` there too (every entry carries a slug, so the feed always needs it; embedded entries pass `declare_namespaces=false` and inherit it).

- [x] **Step 6: Wire into the server mapping** (`server/src/atompub/mapping.rs`): in `post_to_entry`, after building the entry, `set_j_slug(&mut entry, post.slug.as_str());` (use the post's slug field — confirm its accessor on the `post` arg). `entry_to_post_fields` needs **no** change to ignore incoming `j:slug` (it never reads slug from the entry) — add a one-line comment noting the deliberate ignore.

- [x] **Step 7: Run unit tests → PASS**; then add an AtomPub integration test in `atompub_posts.rs` (`#[apply(backends)]`): create a post, GET the member, assert the body contains `<j:slug>` with the post's slug and `xmlns:j`; create a **draft** and assert its entry also carries `<j:slug>` (the gap being fixed). Add a test that POSTing an entry containing `<j:slug>client-supplied</j:slug>` does **not** set the stored slug to that value (server derives its own).

- [x] **Step 8: Gate + commit** — `cargo xtask validate --no-e2e` green; commit `feat(atompub): emit read-only j:slug on every entry (#71)`.

---

## Task 3: Capability discovery (`j:extension` in the service document)

**Files:**
- Modify: `common/src/atompub/service.rs` (`render_service_document` 48–68; tests from line 97)
- Test: in-file `service.rs`; `server/tests/atompub/atompub_service.rs`

**Interfaces:**
- Consumes: `J_NS` (Task 2).
- Produces: the service `<app:service>` root declares `xmlns:j` and contains `<j:extension version="1" features="format-media-type slug"/>`.

- [x] **Step 1: Write failing unit test** in `service.rs` `mod tests` (extend `service_document_lists_two_collections` or add a new test):
```rust
#[test]
fn service_document_advertises_jaunder_extension() {
    let out = render_service_document(&sample_doc()); // reuse the existing test's builder
    assert!(out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#), "out: {out}");
    assert!(out.contains(r#"<j:extension version="1" features="format-media-type slug""#), "out: {out}");
}
```
(If the existing test builds the doc inline, factor a small local `sample_doc()` or inline the same construction.)

- [x] **Step 2: Run → FAIL** — `cargo nextest run -p common service_document_advertises`.

- [x] **Step 3: Implement** in `render_service_document`: add `root.push_attribute(("xmlns:j", J_NS));` after the `xmlns:app` push; and between the `app:workspace` open and the collection writes, emit the marker:
```rust
let mut ext = BytesStart::new("j:extension");
ext.push_attribute(("version", "1"));
ext.push_attribute(("features", "format-media-type slug"));
let _ = writer.write_event(Event::Empty(ext));
```
Import `J_NS` in `service.rs`.

- [x] **Step 4: Run unit test → PASS**; add/extend a server integration assertion in `atompub_service.rs` (`service_document_returns_200_with_app_password`) that the returned doc contains `j:extension` and `features="format-media-type slug"`.

- [x] **Step 5: Gate + commit** — `cargo xtask validate --no-e2e` green; commit `feat(atompub): advertise j:extension capability in the service document (#71)`.

---

## Task 4: Server-side Org canonicalization

**Files:**
- Modify: `common/src/render.rs` (add `canonicalize_org_body`; tests from line 217)
- Modify: `storage/src/post_service.rs` (`perform_post_update` ~195, `perform_post_creation` ~307)
- Test: in-file `render.rs`; `storage/src/post_service.rs` in-file tests; `server/tests/atompub/atompub_posts.rs`

**Interfaces:**
- Produces: `pub fn canonicalize_org_body(body: &str) -> String` — removes the body's **title-source line** following Org title precedence (the same rule `extract_org_title` uses to *find* the title), and strips leading empty lines. Applied to the stored body for `PostFormat::Org` only. Precise contract (user decision 2026-06-27, "strip the title source + leading empty lines"):
  - Scan the **leading header region** (the run of blank lines + `#+key:` lines before the first content line):
    - drop every `#+TITLE:` line (case-insensitive, leading-whitespace tolerant) — this is the recognized title header;
    - **preserve every other `#+FOO:` line verbatim** (ADR-0024);
    - drop blank lines in this region.
  - The first non-blank, non-`#+` line: if it is a top-level `* heading` **and no `#+TITLE:` was seen** (so the heading is the title source per precedence), **drop it**; otherwise keep it. (When a `#+TITLE:` was present, a later `* heading` is content → keep.)
  - Everything from the first kept content line onward is preserved **verbatim**.
  - Strip leading empty lines from the result; trim trailing whitespace. **Idempotent and byte-deterministic.**

- [x] **Step 1: Write failing unit tests** in `render.rs` `mod tests` (exhaustive — this is the load-bearing, user-flagged surface):
```rust
#[test]
fn canon_strips_title_header_keeps_unknown_and_later_heading() {
    // #+TITLE: present → strip it; keep #+FOO:; a LATER * heading is content → keep.
    let out = canonicalize_org_body("#+TITLE: My Post\n#+FOO: keepme\n\n* Section\nBody\n");
    assert_eq!(out, "#+FOO: keepme\n\n* Section\nBody");
}
#[test]
fn canon_strips_leading_heading_when_no_title_header() {
    // No #+TITLE: → the leading * heading IS the title source → strip it.
    let out = canonicalize_org_body("* My Title\n\nBody line\n");
    assert_eq!(out, "Body line");
}
#[test]
fn canon_strips_title_amidst_other_headers_and_leading_blanks() {
    let out = canonicalize_org_body("\n\n#+FOO: x\n#+title: T\n#+BAR: y\n\nbody\n");
    assert_eq!(out, "#+FOO: x\n#+BAR: y\n\nbody");
}
#[test]
fn canon_no_title_source_preserves_headers_and_content() {
    let out = canonicalize_org_body("#+FOO: x\n\njust content\n");
    assert_eq!(out, "#+FOO: x\n\njust content");
}
#[test]
fn canon_non_top_level_heading_is_not_a_title_source() {
    // "** Sub" is not a top-level heading → not the title → keep.
    let out = canonicalize_org_body("** Sub\n\nBody\n");
    assert_eq!(out, "** Sub\n\nBody");
}
#[test]
fn canon_heading_after_body_text_is_content_not_title() {
    let out = canonicalize_org_body("intro\n* Later\nmore\n");
    assert_eq!(out, "intro\n* Later\nmore");
}
#[test]
fn canon_is_idempotent() {
    for body in [
        "#+TITLE: T\n#+FOO: x\n\n* H\nText\n",
        "* My Title\n\nBody\n",
        "#+FOO: x\n\ncontent\n",
    ] {
        let once = canonicalize_org_body(body);
        assert_eq!(canonicalize_org_body(&once), once, "idempotent for {body:?}");
    }
}
```

(Notes for the implementer: `* heading` precedence matches `extract_org_title` — top-level `* ` only, before any body text, and only when no `#+TITLE:` preceded it. Mirror that scanner's structure so the title FOUND and the title STRIPPED can never disagree.)

- [x] **Step 2: Run → FAIL** — `cargo nextest run -p common canonicalize_org`.

- [x] **Step 3: Implement** `canonicalize_org_body` in `render.rs` — a single line-scanner that mirrors `extract_org_title`'s precedence so the title found and the title stripped never disagree:
```rust
/// Canonicalize an ingested Org body (ADR-0024): remove the body's title-source
/// line (a `#+TITLE:` header, or a leading top-level `* heading` when there is no
/// `#+TITLE:`) and strip leading blank lines, while preserving every other line —
/// including unrecognized `#+FOO:` headers and content headings — verbatim. Output
/// is byte-deterministic and idempotent so reconcile (Unit D) never sees false
/// divergence.
#[must_use]
pub fn canonicalize_org_body(body: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut in_header = true; // still scanning the leading blank/#+/title region
    let mut saw_title = false;

    for line in body.lines() {
        if !in_header {
            kept.push(line);
            continue;
        }
        let t = line.trim_start();
        if t.is_empty() {
            continue; // drop leading blank lines in the header region
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("#+title:") {
            saw_title = true;
            continue; // recognized title header → drop
        }
        if t.starts_with("#+") {
            kept.push(line); // unrecognized header → preserve verbatim
            continue;
        }
        // First content line: a top-level "* heading" with no prior #+TITLE: is the
        // title source → drop it; anything else ends the header region and is kept.
        if !saw_title && t.starts_with("* ") {
            in_header = false;
            continue;
        }
        in_header = false;
        kept.push(line);
    }

    kept.join("\n").trim_end().to_string()
}
```
*(The unit tests in Step 1 pin this contract exactly; if a test and this sketch disagree, the tests win — re-read `extract_org_title` (render.rs:149) for the exact `* `/`#+` precedence.)*

- [x] **Step 4: Apply at the storage seam** in `storage/src/post_service.rs`. In **both** `perform_post_creation` (~307) and `perform_post_update` (~195), after `derive_post_metadata(...)` (which still reads the *original* body for the title) compute the canonical body for Org and use it for render + storage:
```rust
let body = if matches!(format, PostFormat::Org) {
    common::render::canonicalize_org_body(&body)
} else {
    body
};
```
Place this **before** `render(&body, &format)` / `create_rendered_post(...)` and before building `UpdatePostInput`. (Order matters: derive title first from the original body, then canonicalize.)

- [x] **Step 5: Storage test** in `post_service.rs` `mod tests` (or `server/tests/storage/storage.rs` if it needs a backend): create an Org post whose body is `"#+TITLE: Hi\n#+FOO: x\n\nHello"`; assert the stored `record.body` has no `#+TITLE:` line, still contains `#+FOO: x` and `Hello`, and `record.title == Some("Hi")`. Add a Markdown control: a Markdown body with a leading `# H1` is stored unchanged (canonicalization is Org-only).

- [x] **Step 6: Double-title regression** — add an AtomPub or web test asserting an Org post with `#+TITLE:` renders the title once: the stored body (hence `rendered_html`) no longer contains the title text from the `#+TITLE:` line, while `record.title` carries it. (Web render double-title at `web/src/pages/ui.rs:379–396` is resolved by the stripped body.)

- [x] **Step 7: Gate + commit** — `cargo xtask validate --no-e2e` green; commit `feat(storage): canonicalize ingested Org bodies (strip the title source, keep the rest) (#71)`. (Two test-forced refinements to the canonicalize sketch: preserve the blank after a kept header; gate the `* heading` drop on `kept.is_empty()` for idempotency. Added a `perform_post_update` Org-canonicalize test to cover the update-path branch.)

---

## Self-review (spec coverage)

- Per-entry format via `atom:content` media type + `format_wire` seam + lenient parser → Task 1.
- `j:slug` read-only on every entry; incoming ignored → Task 2.
- `j:extension` capability discovery → Task 3.
- Server-side org canonicalization (strip `#+TITLE:`, preserve unknown, byte-deterministic) + web double-title fix → Task 4.
- Supersede ADR-0015 token scheme → already done (status edited; ADR-0023/0024 exist) — no task.
- No legacy backfill (no legacy data) → intentionally omitted.
- Edge cases (text/org, text/markdown[;param], html, bare text→default, every entry carries content type + j:slug, incoming j:slug ignored, service doc validates with j:extension, canonicalization strips recognized + preserves unknown + round-trip determinism) → Tasks 1/2/3/4 tests.
