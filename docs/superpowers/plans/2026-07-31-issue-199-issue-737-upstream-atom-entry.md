# Upstream Atom Document I/O Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire the `atom_syndication`/`rss` fork bridge and delegate every
AtomPub Atom document to `atom_syndication` 0.12.10's public API.

**Architecture:** Part A removes the `[patch.crates-io]` apparatus so the
registry crate is reachable; Part B then replaces
`common/src/atompub/entry.rs`'s hand-rolled `quick-xml` reader and four writers
with `Entry::read_from` / `Entry::write_to` / `Feed::write_to`, leaving only
jaunder's own extension helpers and the two wire structs.

**Tech Stack:** Rust, `atom_syndication` 0.12.10, `rss` 2.1, `quick-xml` 0.41,
`chrono`, Nix/crane, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-199-issue-737-upstream-atom-entry.md`](../specs/2026-07-31-issue-199-issue-737-upstream-atom-entry.md)
— the "what" and "why". This plan is the "how" and does not restate it.

## Review header

**Scope in:** root `Cargo.toml`, `flake.nix`, `flake.lock`, `deny.toml`,
`common/Cargo.toml`, `common/src/atompub/{mod,entry,xml}.rs`,
`server/src/atompub/{mod,posts,media}.rs`, `docs/adr/0043-*`.

**Scope out:** `service.rs`/`categories.rs`/`rsd.rs` internals, the elisp
client, e2e assertions, storage-record timestamp types (Task 1 files it).

| Task | One-line deliverable                                                                  |
| ---- | ------------------------------------------------------------------------------------- |
| 1    | File the follow-up issue for `PostRecord`/`MediaRecord` → `UtcInstant` (no commit).   |
| 2    | Drop the fork bridge; move to registry `atom_syndication` 0.12.10 / `rss` 2.1 (#199). |
| 3    | `FeedMeta`/`MediaLinkEntry` carry `UtcInstant` instead of RFC-3339 `String` (D8).     |
| 4    | `AtomPubError::Serialize` variant mapping to 500, not 400 (D9).                       |
| 5    | `entry_from_xml` → `Entry::read_from`, with the strictness deltas pinned (D6).        |
| 6    | The three serializers → upstream writers; delete the hand-rolled machinery (D1/D3).   |
| 7    | Retire the dead `From` impls, fix the stale module doc, flip ADR-0043 to superseded.  |
| 8    | Full local verification: `validate --no-e2e` + `e2e-local atompub.spec.ts` (D7).      |

**Key risks / decisions:**

- **Task 2 is the risky one.** It touches the Nix vendor path. `nix` evaluation
  in a shallow-clone worktree needs a dirty tree, and flakes ignore untracked
  files — `git add` before any `nix build`.
- **`default-features = false` must survive the version bump** (upstream's
  default is `["builders"]`). Consequence: `FeedBuilder`/`EntryBuilder` are
  **not** available — construct `Feed`/`Entry` with struct literals plus
  `..Default::default()`.
- **The `From<atom_syndication::Error>` impl is reader-only.** Writers must map
  explicitly to `AtomPubError::Serialize`, or a serialization failure would ride
  the `Malformed` → 400 path that Task 4 exists to prevent.
- Task 5 and Task 6 are separable: after Task 5 the reader is upstream's while
  the writers are still hand-rolled, and the round-trip tests must stay green
  across that split.
- **Every task's commit is gated by `cargo xtask check`**, which includes clippy
  at `-D warnings` and the Nix coverage check. Two consequences the steps call
  out explicitly: Task 6 must fix the three serializers' doc comments
  (`missing_errors_doc`, `double_must_use`) in the same edit that changes their
  signatures, and Task 5 must delete `From<quick_xml::Error>` in the same commit
  that orphans it, or its body drops to zero coverage.

## Global Constraints

- `atom_syndication = { version = "0.12.10", default-features = false }` — the
  `default-features = false` is load-bearing (spec A5).
- `rss = "2.1"`; `quick-xml = "0.41"` stays a direct `common` dependency.
- Exactly one `quick-xml` in `Cargo.lock`, at `>= 0.41`; no
  `[advisories].ignore` for RUSTSEC-2026-0194/0195.
- Every public item keeps its current name and export path in `common::atompub`.
- `unwrap_used` / `expect_used` are **denied** in production code (workspace
  lints).
- Commit gate: run `cargo xtask check` before every commit (`jaunder-commit`).
  **No `Co-Authored-By` trailer.**
- Run `cargo xtask` via `devtool run --` from inside the worktree — bare
  `ctx_execute` targets the main repo and yields a false pass.

---

### Task 1: File the follow-up issue for record-level `UtcInstant`

**Files:** none in-tree.

**Interfaces:**

- Consumes: nothing.
- Produces: an issue number, referenced from the ADR draft's follow-up line.

- [x] **Step 1: File it** via `jaunder-issues` (type `Task`, label
      `type-safety`, milestone _Code quality ratchet_, project #1, priority P3).

Title: `types: PostRecord/MediaRecord timestamps as UtcInstant`

Body must state: `PostRecord.{created_at,updated_at,published_at,deleted_at}`
and the media record's `created_at` are raw `chrono::DateTime<Utc>` while every
other field on those records is a domain newtype; migrating them to
`common::time::UtcInstant` (ADR-0072/0063) requires `sqlx`
`Type`/`Encode`/`Decode` impls covering **both** sqlite and postgres, and
touches storage, server, and web call sites. Note that #737 converted the
AtomPub wire structs (`FeedMeta`, `MediaLinkEntry`) but deliberately stopped at
that seam.

- [x] **Step 2: Record the number** in
      `docs/adr/drafts/upstream-atom-document-io.md` — **add** a new bullet to
      the "Consequences" list (the only follow-up bullet there today is about
      archiving the two GitHub forks; do not overwrite it):

```markdown
- **Follow-up.** Migrating the storage records' own timestamps to `UtcInstant` —
  the last non-newtype fields on `PostRecord` — is tracked as #<N>.
```

**No commit for this task.** `.gitignore` excludes `docs/adr/drafts/*` except
its README, so `git add` on that path exits non-zero ("The following paths are
ignored"). The draft rides to `main` via `cargo xtask adr promote` at ship,
which stages the promoted file itself. Carry the edit forward uncommitted.

---

### Task 2: Retire the fork bridge (#199)

**Files:**

- Modify: `Cargo.toml:102-111` (delete the `[patch.crates-io]` block and its
  comment)
- Modify: `flake.nix:23-31` (delete the `atom-fork` / `rss-fork` inputs),
  `flake.nix:42-43` (their function arguments), `flake.nix:300-340` (the
  `cargoVendorDir` / `overrideVendorGitCheckout` binding), **and
  `flake.nix:1167`**
  (`deny = craneLib.cargoDeny { inherit src cargoVendorDir; … }` — a second
  consumer outside that range; `rg -n 'cargoVendorDir' flake.nix` finds them
  all)
- Modify: `flake.lock` (regenerated)
- Modify: `deny.toml:245-250` (drop `jaunder-org` from
  `[sources.allow-org].github`)
- Modify: `common/Cargo.toml:25` (`atom_syndication`), and the `rss` line
- Modify: `Cargo.lock` (regenerated)

**Interfaces:**

- Consumes: nothing.
- Produces: registry `atom_syndication` 0.12.10 with `Entry::read_from` /
  `Entry::write_to` / `Feed::write_to` reachable, and `WriteConfig` exported
  from the crate root. All later tasks depend on this.

- [x] **Step 1: Delete the patch block and bump the requirements**

In `Cargo.toml`, remove the entire `# TEMPORARY (jaunder #193 / ADR-0043)`
comment and the `[patch.crates-io]` section.

In `common/Cargo.toml`:

```toml
rss = { version = "2.1", default-features = false, features = ["builders", "atom"] }
atom_syndication = { version = "0.12.10", default-features = false }
```

Copy this verbatim — **only the version numbers change**. `rss`'s `builders` and
`atom` features are load-bearing (`common/src/feed/rss.rs` uses
`ChannelBuilder`, `ItemBuilder`, `GuidBuilder`, and
`rss::extension::atom::AtomExtension`), and `default-features = false` on
`atom_syndication` is spec A5.

- [x] **Step 2: Delete the Nix vendoring**

Remove the `atom-fork` and `rss-fork` flake inputs, their entries in the outputs
function's argument list, and the whole
`cargoVendorDir = craneLib.vendorCargoDeps { ... };` binding with its
`overrideVendorGitCheckout`. Any `cargoVendorDir` reference in the crane arg
sets goes too — crane falls back to its own default vendoring.

- [x] **Step 3: Drop the sources allowance**

In `deny.toml`, remove `jaunder-org` from `[sources.allow-org].github` (and the
`# TEMPORARY` comment above it). If the list becomes empty, remove the
`[sources.allow-org]` table rather than leaving `github = []`.

- [x] **Step 4: Regenerate the lockfiles**

```bash
devtool run -- cargo tree -i quick-xml
devtool run -- nix flake lock
```

**Do NOT run `cargo update`.** A bare `cargo update` bumps the _whole_ graph —
it drags `wasm-bindgen` 0.2.121 → 0.2.126, which then mismatches the
`wasm-bindgen-cli` pinned in the Nix toolchain and fails the `nix-coverage` gate
with "rust Wasm file schema version 0.2.126 / this binary schema version
0.2.121". (`cargo update -p atom_syndication` is also wrong: with the `[patch]`
gone but the lock still naming the git sources, it fails with "package ID
specification did not match any packages".)

Instead let cargo do the **minimal** re-resolve — any command that reads the
lock rewrites only the entries whose requirements changed, and `cargo tree`
doubles as Step 5's verification. Confirm the blast radius before moving on:

```bash
git diff --stat Cargo.lock
```

Expected: ~10 changed lines, touching **only** the `atom_syndication` and `rss`
entries (version, `source` git→registry, and a new `checksum`). Anything larger
means a wholesale update slipped in — `git checkout -- Cargo.lock` and redo.

Then stage everything, including `flake.lock` — Nix flakes ignore untracked
files, so an unstaged new/changed file is invisible to the build:

```bash
git add -A
```

- [x] **Step 5: Verify the dependency graph**

```bash
cargo tree -i quick-xml
```

Expected: exactly one `quick-xml v0.41.x`, with `atom_syndication v0.12.10` and
`rss v2.1.x` among its dependents and **no**
`(git+https://github.com/jaunder-org/...)` source annotations anywhere.

- [x] **Step 6: Verify the build and the advisories**

```bash
devtool run -- cargo xtask check --no-test
```

Expected: PASS. If `rss` 2.1 changed an API used by `common/src/feed/rss.rs`,
fix the call sites here — that is in scope for this task.

- [x] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock common/Cargo.toml flake.nix flake.lock deny.toml
git commit -m "deps: drop the atom_syndication/rss fork patch (#199)"
```

---

### Task 3: `UtcInstant` at the AtomPub wire seam

**Files:**

- Modify: `common/src/atompub/entry.rs` (`FeedMeta`, `MediaLinkEntry`, and the
  two writers that read their timestamp fields)
- Modify: `server/src/atompub/posts.rs:189-202`,
  `server/src/atompub/media.rs:55-65`
- Test: in-file `#[cfg(test)]` in `common/src/atompub/entry.rs`

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:

```rust
pub struct FeedMeta {
    pub id: AbsoluteUrl,
    pub title: String,
    pub updated: UtcInstant,
    pub self_url: AbsoluteUrl,
    pub first: Option<AbsoluteUrl>,
    pub next: Option<AbsoluteUrl>,
    pub previous: Option<AbsoluteUrl>,
}

pub struct MediaLinkEntry {
    pub id: AbsoluteUrl,
    pub title: Filename,
    pub edit_uri: AbsoluteUrl,
    pub edit_media_uri: AbsoluteUrl,
    pub content_src: AbsoluteUrl,
    pub content_type: ContentType,
    pub published: UtcInstant,
    pub updated: UtcInstant,
}
```

Task 6 consumes both. `UtcInstant` comes from `crate::time::UtcInstant`; convert
to atom's `FixedDateTime` with `instant.value().fixed_offset()`.

- [x] **Step 1: Write the failing tests**

Add to `common/src/atompub/entry.rs`'s test module:

Use the existing `crate::test_support::parse_utc_instant` (test_support.rs:448)
— the test module already imports its siblings `parse_absolute_url` /
`parse_filename` / `parse_content_type` from there. Do not hand-roll a local
helper.

```rust
#[test]
fn feed_meta_updated_is_serialized_as_rfc3339_utc() {
    let meta = FeedMeta {
        id: parse_absolute_url("https://example.com/atompub/alice/posts"),
        title: "Alice's Posts".to_string(),
        updated: parse_utc_instant("2026-05-31T12:00:00Z"),
        self_url: parse_absolute_url("https://example.com/atompub/alice/posts"),
        first: None,
        next: None,
        previous: None,
    };
    let out = render_feed(&meta, &[]);
    assert!(out.contains("2026-05-31T12:00:00"), "out: {out}");
}

#[test]
fn media_link_entry_timestamps_are_serialized_as_rfc3339_utc() {
    let out = render_media_link_entry(&MediaLinkEntry {
        id: parse_absolute_url("https://h/atompub/alice/media/abc/pic.png"),
        title: parse_filename("pic.png"),
        edit_uri: parse_absolute_url("https://h/atompub/alice/media/abc/pic.png"),
        edit_media_uri: parse_absolute_url("https://h/media/upload/ab/c0/abc/pic.png"),
        content_src: parse_absolute_url("https://h/media/upload/ab/c0/abc/pic.png"),
        content_type: parse_content_type("image/png"),
        published: parse_utc_instant("2026-06-01T00:00:00Z"),
        updated: parse_utc_instant("2026-06-02T00:00:00Z"),
    });
    assert!(out.contains("<published>2026-06-01T00:00:00"), "out: {out}");
    assert!(out.contains("<updated>2026-06-02T00:00:00"), "out: {out}");
}
```

Also update the two existing tests that construct `FeedMeta`
(`render_feed_wraps_entries_with_paging`,
`render_feed_without_paging_omits_optional_links`) and the two that construct
`MediaLinkEntry` to the new field names.

- [x] **Step 2: Run the tests, verify they fail**

```bash
devtool run -- cargo nextest run -p common atompub::entry
```

Expected: FAIL — no field `updated` on `FeedMeta`; no field `published` on
`MediaLinkEntry`.

- [x] **Step 3: Implement against the tests**

Change the struct fields as in **Interfaces**. In `write_entry`'s feed and
media-link paths, replace the verbatim `&meta.updated_rfc3339` /
`&entry.published_rfc3339` writes with `&meta.updated.value().to_rfc3339()` and
the equivalents — the writers are still the hand-rolled ones at this point; Task
6 replaces them.

Update the producers:

```rust
// server/src/atompub/posts.rs — replaces the map_or_else over to_rfc3339()
let updated = records
    .first()
    .map_or_else(|| UtcInstant::from(chrono::Utc::now()), |p| p.updated_at.into());
```

and in `server/src/atompub/media.rs`, replace
`let timestamp = record.created_at.to_rfc3339();` with
`let timestamp = UtcInstant::from(record.created_at);`, filling `published` and
`updated` from it (`UtcInstant` is `Copy`, so no `.clone()`).

Both files need `use common::time::UtcInstant;` added.

Note when executing: a bare `cargo nextest run -p jaunder` fails the six
`case_2_postgres` media tests with `ConnectionRefused` — that is the harness, not
the code. Re-run under `devtool pg run -- cargo nextest run -p jaunder`.

- [x] **Step 4: Run the tests, verify they pass**

```bash
devtool run -- cargo nextest run -p common atompub::entry
devtool run -- cargo nextest run -p jaunder
```

Expected: PASS.

- [x] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/atompub/entry.rs server/src/atompub/posts.rs server/src/atompub/media.rs
git commit -m "refactor(atompub): carry wire timestamps as UtcInstant (#737)"
```

---

### Task 4: A serialization error that is not a 400

**Files:**

- Modify: `common/src/atompub/mod.rs:41-59`
- Modify: `server/src/atompub/mod.rs:208-213`
- Test: in-file `#[cfg(test)]` in both files

**Interfaces:**

- Consumes: nothing.
- Produces:

```rust
pub enum AtomPubError {
    /// The supplied XML could not be parsed as the expected document type.
    Malformed(String),
    /// The document could not be written. Server-side, never the client's fault.
    Serialize(String),
}

impl From<atom_syndication::Error> for AtomPubError {
    fn from(e: atom_syndication::Error) -> Self;  // -> Malformed
}
```

Task 6 uses `AtomPubError::Serialize` explicitly via `.map_err(...)`; it must
**not** rely on the `From` impl for writes, because that impl yields `Malformed`
(→ 400).

- [ ] **Step 1: Write the failing tests**

In `common/src/atompub/mod.rs`:

```rust
#[test]
fn atom_error_converts_to_malformed() {
    let err: AtomPubError = atom_syndication::Error::InvalidStartTag.into();
    assert!(matches!(err, AtomPubError::Malformed(_)));
}

#[test]
fn serialize_error_displays_its_cause() {
    let err = AtomPubError::Serialize("boom".to_string());
    assert!(err.to_string().contains("boom"));
}
```

In `server/src/atompub/mod.rs`:

```rust
#[test]
fn malformed_document_is_a_bad_request() {
    let err = HandlerError::from(common::atompub::AtomPubError::Malformed("x".to_string()));
    assert!(matches!(err, HandlerError::BadRequest));
}

#[test]
fn serialization_failure_is_internal_not_bad_request() {
    let err = HandlerError::from(common::atompub::AtomPubError::Serialize("x".to_string()));
    assert!(matches!(err, HandlerError::Internal));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

```bash
devtool run -- cargo nextest run -p common atompub
devtool run -- cargo nextest run -p jaunder atompub
```

Expected: FAIL — no variant `Serialize`; no `From<atom_syndication::Error>`.

- [ ] **Step 3: Implement against the tests**

Add the `Serialize` variant with
`#[error("failed to serialize AtomPub document: {0}")]`, add the
`From<atom_syndication::Error>` impl mapping to `Malformed(e.to_string())`, and
make the `HandlerError` conversion match on the variant — `Malformed` →
`BadRequest`, `Serialize` → `log_internal(&err)` then `HandlerError::Internal`.
Take the error by value (`err`) rather than discarding it with `_`, since the
internal arm logs it.

- [ ] **Step 4: Run the tests, verify they pass**

Same two commands. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/atompub/mod.rs server/src/atompub/mod.rs
git commit -m "feat(atompub): distinguish serialization failure from malformed input (#737)"
```

---

### Task 5: `entry_from_xml` reads via `atom_syndication`

**Files:**

- Modify: `common/src/atompub/entry.rs` (replace the reader; delete `Parser`,
  `Acc`, `Field`, `build_entry`, `read_xhtml_content`, `resolve_ref`,
  `decode_text`, `local_name`, `local_name_end`, `attr_value`, `capture_link`,
  `append`, `trimmed`, `parse_dt`)
- Test: in-file `#[cfg(test)]` in `common/src/atompub/entry.rs`

**Interfaces:**

- Consumes: `AtomPubError` + its `From<atom_syndication::Error>` (Task 4).
- Produces: `pub fn entry_from_xml(xml: &str) -> Result<Entry, AtomPubError>` —
  signature unchanged.

- [ ] **Step 1: Write the failing tests**

**Keep `non_scalar_char_ref_is_an_error` as-is** — its assertion still holds
(`&#xD800;` reaches `gref.resolve_char_ref()`, which errors on a surrogate →
`Error::Xml` → `Malformed`); only its comment, which explains the deleted
`resolve_ref`'s unreachable arm, needs rewriting.

**Rewrite `unsupported_entity_is_an_error` into its inverse** — `atom_text`
resolves predefined entities, then char refs, then falls through to re-emitting
the reference verbatim (`util.rs:92-102`), so this is the spec's one _loosening_
delta and must be pinned, not deleted:

```rust
#[test]
fn an_unsupported_entity_is_passed_through_literally() {
    // Loosening delta (spec B6a): upstream re-emits an unresolvable reference rather
    // than rejecting the entry — lenient ingest, consistent with R5 in mapping.rs.
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>x&bogus;y</title>
</entry>"#;
    let entry = entry_from_xml(xml).expect("parse");
    assert_eq!(entry.title().as_str(), "x&bogus;y");
}
```

Then add the strictness/delta tests:

```rust
#[test]
fn rejects_an_unparseable_updated() {
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <updated>not-a-date</updated>
</entry>"#;
    assert!(matches!(
        entry_from_xml(xml),
        Err(AtomPubError::Malformed(_))
    ));
}

#[test]
fn rejects_a_title_type_outside_the_text_constructs() {
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title type="text/markdown">T</title>
</entry>"#;
    assert!(matches!(
        entry_from_xml(xml),
        Err(AtomPubError::Malformed(_))
    ));
}

#[test]
fn a_media_type_on_content_is_still_accepted() {
    // The ADR-0023 format carrier: `type` on <content> is a media type and is NOT
    // constrained to the text|html|xhtml construct that <title>/<summary> are.
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text/org">* heading</content>
</entry>"#;
    let entry = entry_from_xml(xml).expect("parse");
    assert_eq!(content_parts(&entry), (Some("text/org"), Some("* heading")));
}

#[test]
fn a_prefixed_atom_title_does_not_populate_the_title() {
    // Accepted narrowing (spec D6.3): upstream matches qualified names, so a
    // prefixed child lands in the extension map instead of the title.
    let xml = r#"<entry xmlns:atom="http://www.w3.org/2005/Atom">
  <atom:title>T</atom:title>
</entry>"#;
    let entry = entry_from_xml(xml).expect("parse");
    assert_ne!(entry.title().as_str(), "T");
}

#[test]
fn xhtml_content_escapes_literal_apostrophes() {
    // Accepted delta: upstream's atom_xhtml applies escape() to text events.
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div>it's</div></content>
</entry>"#;
    let entry = entry_from_xml(xml).expect("parse");
    let (_, value) = content_parts(&entry);
    let value = value.expect("xhtml value");
    assert!(value.contains("&apos;"), "value: {value}");
}

#[test]
fn xhtml_entity_references_survive_the_round_trip() {
    // NOT a delta — today's reader also stores `&amp;` (BytesText::new re-escapes).
    let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div>b &amp; c</div></content>
</entry>"#;
    let entry = entry_from_xml(xml).expect("parse");
    let (_, value) = content_parts(&entry);
    assert!(value.expect("xhtml value").contains("&amp;"));
}
```

Finally, **extend** `draft_and_html_round_trip_through_serialize_then_parse` to
actually cover spec B3. It asserts title, summary, content, categories, and the
draft flag today — but not links, timestamps, or `j:slug`. The slug matters
most: today's bespoke `Parser` has no `slug` arm at all, so reading one back is
new behavior arriving with this task and pinned by nothing. Add to that test,
before serializing:

```rust
    entry.links = vec![Link {
        rel: "edit".to_string(),
        href: "https://h/atompub/alice/posts/1".to_string(),
        ..Default::default()
    }];
    entry.published =
        Some(chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap());
    set_j_slug(&mut entry, "my-post");
```

and after re-parsing:

```rust
    assert_eq!(parsed.links().len(), 1);
    assert_eq!(parsed.links()[0].rel(), "edit");
    assert_eq!(parsed.published().map(chrono::DateTime::to_rfc3339).as_deref(),
               Some("2026-01-01T00:00:00+00:00"));
    assert_eq!(parsed.updated().to_rfc3339(), entry.updated().to_rfc3339());
    assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
```

Keep every other existing reader test unchanged — they are the regression net
for D2.

- [ ] **Step 2: Run the tests, verify they fail**

```bash
devtool run -- cargo nextest run -p common atompub::entry
```

Expected: FAIL — the new strictness tests fail because the bespoke reader
tolerates all three inputs.

- [ ] **Step 3: Implement against the tests**

Replace the body with a delegation to upstream, then delete every helper listed
in **Files**:

```rust
pub fn entry_from_xml(xml: &str) -> Result<Entry, AtomPubError> {
    Ok(xml.parse::<Entry>()?)
}
```

`impl FromStr for Entry` delegates to `Entry::read_from(s.as_bytes())`, and the
`?` uses Task 4's `From<atom_syndication::Error>`.

**Delete `impl From<quick_xml::Error> for AtomPubError`
(`common/src/atompub/mod.rs:49-53`) in this same task.** The reader was its only
`?`-driven caller, and an unused trait impl is not `dead_code` — it would
compile but drop to zero coverage at this task's commit, which the gate's Nix
coverage check catches. (Its sibling `From<std::io::Error>` is insulated by its
own test and goes in Task 7.)

Two existing tests need their expectations re-read rather than rewritten —
confirm `malformed_xml_is_an_error` and `document_without_entry_is_an_error`
still pass (upstream yields `Eof` and `InvalidStartTag` respectively, both
mapping to `Malformed`). If `parses_id_and_timestamps` or the xhtml tests fail
on whitespace, adjust the assertion to `contains` rather than reintroducing a
trim.

- [ ] **Step 4: Run the tests, verify they pass**

```bash
devtool run -- cargo nextest run -p common atompub::entry
devtool run -- cargo nextest run -p jaunder atompub
```

Expected: PASS. The `server` run matters here — `mapping.rs` has ~15 tests
driving `entry_from_xml`.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/atompub/entry.rs common/src/atompub/mod.rs
git commit -m "refactor(atompub): read entries via atom_syndication (#737)"
```

---

### Task 6: The three serializers write via `atom_syndication`

**Files:**

- Modify: `common/src/atompub/entry.rs` (`entry_to_xml`, `write_entry`,
  `render_feed`, `render_media_link_entry`)
- Modify: `common/src/atompub/xml.rs` (delete `write_link`)
- Modify: `common/src/atompub/entry.rs` (`set_draft` / `set_j_slug` namespace
  bookkeeping)
- Modify: `server/src/atompub/posts.rs:204,271,438-457,446,541`,
  `server/src/atompub/media.rs:123,167`
- Test: in-file `#[cfg(test)]` in `common/src/atompub/entry.rs`

**Interfaces:**

- Consumes: `FeedMeta`/`MediaLinkEntry` with `UtcInstant` (Task 3);
  `AtomPubError::Serialize` (Task 4).
- Produces:

```rust
pub fn entry_to_xml(entry: &Entry) -> Result<String, AtomPubError>;
pub fn render_feed(meta: &FeedMeta, entries: &[Entry]) -> Result<String, AtomPubError>;
pub fn render_media_link_entry(entry: &MediaLinkEntry) -> Result<String, AtomPubError>;
```

and in `server/src/atompub/posts.rs`:

```rust
fn post_entry_response(
    status: StatusCode,
    post: &PostRecord,
    base: &AbsoluteUrl,
    username: &Username,
) -> Result<Response, HandlerError>;
```

- [ ] **Step 1: Write the failing tests**

Update every existing serializer test to unwrap the new `Result`
(`entry_to_xml(&entry).expect("serialize")`), then add:

```rust
#[test]
fn draft_entry_declares_the_app_namespace_and_clearing_removes_it() {
    let mut entry = sample_entry();
    set_draft(&mut entry, true);
    let out = entry_to_xml(&entry).expect("serialize");
    assert!(out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#), "out: {out}");

    set_draft(&mut entry, false);
    let out = entry_to_xml(&entry).expect("serialize");
    assert!(!out.contains("xmlns:app"), "out: {out}");
    assert!(!out.contains("app:draft"), "out: {out}");
}

#[test]
fn media_link_content_carries_type_and_src() {
    let out = render_media_link_entry(&sample_media_link_entry()).expect("serialize");
    // Upstream writes a paired element, not a self-closing one (spec delta).
    assert!(out.contains(r#"<content type="image/png""#), "out: {out}");
    assert!(
        out.contains(r#"src="https://h/media/upload/ab/c0/abc/pic.png""#),
        "out: {out}"
    );
    assert!(out.contains("</content>"), "out: {out}");
}
```

Extract the `MediaLinkEntry` literal shared by the media-link tests into a
`sample_media_link_entry()` helper rather than repeating it a third time.

Also strengthen `render_feed_wraps_entries_with_paging` for spec B7 — asserting
`>First<` and `>Second<` appear is a weak proxy for "one `<entry>` per input
entry", since a duplicated or dropped entry still passes. Add:

```rust
    assert_eq!(out.matches("<entry").count(), 2, "out: {out}");
```

- [ ] **Step 2: Run the tests, verify they fail**

```bash
devtool run -- cargo nextest run -p common atompub::entry
```

Expected: FAIL — `entry_to_xml` returns `String`, so `.expect` does not compile.

- [ ] **Step 3: Implement against the tests**

Signatures per **Interfaces**. Each writer builds the upstream model and calls
`write_to`, mapping the error explicitly:

```rust
fn to_xml_string<W>(write: W) -> Result<String, AtomPubError>
where
    W: FnOnce(Vec<u8>) -> Result<Vec<u8>, atom_syndication::Error>,
{
    let bytes = write(Vec::new()).map_err(|e| AtomPubError::Serialize(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| AtomPubError::Serialize(e.to_string()))
}
```

`entry_to_xml` is then `to_xml_string(|w| entry.write_to(w))`. `render_feed`
builds a `Feed` with a struct literal (**not** `FeedBuilder` — the `builders`
feature is off): `id` from `meta.id`, `title: Text::plain(&meta.title)`,
`updated: meta.updated.value().fixed_offset()`, `links` assembled from
`self_url`/`first`/`previous`/`next` with the matching `rel`,
`entries: entries.to_vec()`, and `namespaces` carrying `app` → `APP_NS` and `j`
→ `J_NS`. `render_media_link_entry` builds an `Entry` whose `content` is
`Content { content_type: Some(...), src: Some(...), value: None, ..Default::default() }`,
with `links` for `edit` and `edit-media` and
`title: Text::plain(entry.title.decoded())`.

For D4, `set_draft` inserts `("app", APP_NS)` into `entry.namespaces` when
setting and removes the `"app"` key when clearing; `set_j_slug` inserts
`("j", J_NS)`. Delete `write_entry` and `xml.rs`'s `write_link` once nothing
calls them.

**Fix the doc comments in the same edit, or this task fails its own gate.**
Clippy pedantic is `warn` and the gate runs `-D warnings`:

- Each of the three now needs a `## Errors` section
  (`clippy::missing_errors_doc`). `entry_from_xml` already has one — match its
  style.
- Each must **lose** its `#[must_use]` (`clippy::double_must_use` fires on a
  `#[must_use]` fn returning `Result`).
- Each one's existing doc asserts the opposite of the new signature —
  "Serialization writes into an in-memory buffer, which cannot fail, so this is
  infallible and returns a `String` directly" and "Writes into an in-memory
  buffer, so it is infallible." Delete those sentences; the residual error is
  the `Vec<u8>` write, which cannot fail in practice but is no longer claimed
  away.

At the call sites, add `?` and change `post_entry_response` to return
`Result<Response, HandlerError>`, propagating at its two callers (`posts.rs:418`
and `:428`).

- [ ] **Step 4: Run the tests, verify they pass**

```bash
devtool run -- cargo nextest run -p common atompub
devtool run -- cargo nextest run -p jaunder
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/atompub/entry.rs common/src/atompub/xml.rs server/src/atompub/posts.rs server/src/atompub/media.rs
git commit -m "refactor(atompub): write entries and feeds via atom_syndication (#737)"
```

---

### Task 7: Retire the dead conversions and the stale rationale

**Files:**

- Modify: `common/src/atompub/mod.rs:1-9` (module doc), `:49-59` (the two `From`
  impls and the `io_error_converts_to_malformed` test)
- Modify: `common/src/atompub/entry.rs:1-13` (module doc)
- Modify: `docs/adr/0043-quick-xml-fork-patch.md:3` (status)
- Modify: `docs/README.md` (generated — via `sync-readme`, not by hand)

**Interfaces:**

- Consumes: everything above.
- Produces: nothing consumed downstream.

- [ ] **Step 1: Verify the remaining `From` impl is genuinely unused**

```bash
rg -n 'std::io::Error' common/src/atompub/
```

Expected: no `?`-driven use remains — `service.rs`/`categories.rs` discard
writer errors with `let _ =`. If a use survives, keep the impl and note why.
(`From<quick_xml::Error>` was already deleted in Task 5, where it was orphaned.)

- [ ] **Step 2: Delete it and fix the docs**

Remove `impl From<std::io::Error> for AtomPubError` and the now-orphaned
`io_error_converts_to_malformed` test.

Rewrite `entry.rs`'s module doc: the data model is still
`atom_syndication::Entry`, but the XML I/O is now upstream's; what remains local
is the `app:control/app:draft` and `j:slug` extension handling plus the two wire
structs. Delete the "we do not reuse `atom_syndication`'s XML I/O" paragraph
outright (spec C4).

Adjust `mod.rs`'s module doc where it claims the module could be contributed
upstream — narrow it to the `app:`/RSD documents that remain jaunder's.

- [ ] **Step 3: Flip ADR-0043 and re-sync the table**

Set `- Status: superseded` in `docs/adr/0043-quick-xml-fork-patch.md` and add a
line under the heading pointing at
`docs/adr/drafts/upstream-atom-document-io.md`. Then:

```bash
devtool run -- cargo xtask adr sync-readme
```

Expected: `docs/README.md`'s status cell for 0043 updates. The
`adr-readme-parity` gate fails the commit if it drifts.

- [ ] **Step 4: Run the tests**

```bash
devtool run -- cargo nextest run -p common atompub
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/atompub/mod.rs common/src/atompub/entry.rs docs/adr/0043-quick-xml-fork-patch.md docs/README.md
git commit -m "docs(atompub): retire the crate-private rationale and supersede ADR-0043 (#737)"
```

Note: prettier runs pre-commit on prose and restages it — run `prettier -w` on
the touched markdown first, or expect a restage.

---

### Task 8: Full local verification

**Files:** none.

**Interfaces:** consumes the whole branch.

- [ ] **Step 1: The static/coverage gate**

```bash
devtool run -- cargo xtask validate --no-e2e
```

Expected: PASS. Run it in the **foreground** with a generous timeout —
backgrounded gate runs get killed. `xtask check` auto-fixes formatting but does
not commit, so check `git status --porcelain` afterwards and fold any reformat
into the relevant commit.

- [ ] **Step 2: The AtomPub e2e**

```bash
devtool run -- cargo xtask e2e-local atompub.spec.ts
```

Expected: PASS. Before running, confirm no stale server holds port 3000
(`ss -ltn 'sport = :3000'`) — a stale server yields a false negative.

- [ ] **Step 3: Confirm the acceptance criteria mechanically**

```bash
rg -n 'patch.crates-io' Cargo.toml
rg -n 'jaunder-org' Cargo.toml flake.nix flake.lock deny.toml
rg -n 'quick_xml' common/src/atompub/entry.rs
rg -n '_rfc3339' common/src/atompub/ server/src/atompub/
```

Expected: all four return nothing (spec A1, A2, B1, B9).

- [ ] **Step 4: Commit any residue**

Only if Step 1 left formatting changes:

```bash
git add -A
git commit -m "style: apply gate formatting"
```

Prefer folding such a change into the commit that introduced it
(`git commit --fixup` + autosquash) over a trailing style commit.

## Self-review

**Spec coverage:** A1-A2 → Task 2 + Task 8 Step 3. A3 → Task 2 Step 2. A4 → Task
2 Steps 4-5. A5 → Task 2 Step 1 (Global Constraints). A6 → Task 2 Step 6 + Task
8 Step 1. **B1 → Task 6** (not Task 5 — after Task 5 the _writers_ still use
`quick_xml`, so the `rg` comes back clean only once Task 6 lands) + Task 8
Step 3. B2 → Task 6. **B3 → Task 5 Step 1**, which extends
`draft_and_html_round_trip_through_serialize_then_parse` with the links,
timestamp, and `j:slug` assertions it was missing. B4 → Task 6 Step 1. B5 → Task
5 Step 1. B6 → Task 5 Step 1. B6a → Task 5 Step 1 (the rewritten
`an_unsupported_entity_is_passed_through_literally`, plus the retained surrogate
test). **B7 → Task 6 Step 1**, which adds the `<entry`-count assertion. B8 →
Task 6 Step 1. B9 → Task 3 + Task 8 Step 3. B10 → Task 4 Step 1. **B11 → Task
5** (`From<quick_xml::Error>`) **+ Task 7** (`From<std::io::Error>`). C1-C2 →
Task 8. C3 → Task 7 (ADR draft already written during the design step). C4 →
Task 7 Step 2. C5 → Task 1.

**Type consistency:** `FeedMeta.updated` / `MediaLinkEntry.{published,updated}`
are named identically in Tasks 3 and 6. `AtomPubError::Serialize` is introduced
in Task 4 and consumed in Task 6. `post_entry_response`'s new return type
appears in Task 6's Interfaces and nowhere earlier.

**Known ordering constraint:** Task 3 edits the hand-rolled writers that Task 6
deletes. That is deliberate — it keeps the `UtcInstant` change independently
testable rather than entangling it with the upstream swap.
