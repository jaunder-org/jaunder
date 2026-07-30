# #720 — Encoded Filename Canonical: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the percent-encoded filename the canonical value in
`media.filename`, so the DB value, the on-disk name, and the URL segment are
byte-identical — unblocking #711's byte-equality comparison.

**Architecture:** `Filename` keeps holding one string, but that string becomes
the _encoded_ form; `Filename::sanitized` encodes as its last step and `FromStr`
gains a canonicity check plus a relocated safe-leaf guard. The three routes that
receive an axum-decoded filename get a new inbound twin, `ProfferedFilename`,
whose `FromStr` re-encodes; every path builder then just interpolates. Display
is the only side that transforms, via `Filename::decoded()`.

**Spec:**
`docs/superpowers/specs/2026-07-30-issue-720-encoded-filename-canonical.md` —
the "what/why." This plan is the "how"; decisions are cited as D1–D9 and
criteria as AC1–AC19 rather than restated.

**Tech Stack:** Rust; `percent-encoding` (already an unconditional `common`
dep); `axum` 0.8; `sqlx`; Leptos (`web`); Playwright (`end2end`); `xtask` static
checks.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **One clean commit per task.** Run `devtool run -- cargo xtask check` before
  each commit so the pre-commit gate passes clean (`jaunder-commit`). `check`
  auto-fixes formatting, so re-check `git status --porcelain` after it goes
  green.
- **Every commit leaves the gate green.** Task 3 exists solely so Task 4's flip
  has somewhere to land without a red intermediate; do not merge them.
- **Storage tests follow the dual-backend template** (`CONTRIBUTING.md` "backend
  parity"). A bare `#[tokio::test]` that should be dual-backend fails the
  `test-backend-pattern` guard.
- **`-p jaunder` needs PostgreSQL.** The whole integration suite is
  `#[apply(backends)]`, so a bare `cargo nextest run -p jaunder` dies with a
  misleading `ConnectionRefused` on the postgres half. Always
  `devtool pg run -- cargo nextest run -p jaunder …`. (`-p common`, `-p storage`
  unit tests, and the `xtask` commands need no wrapper.)
- **Worktree paths.** All work happens in
  `.claude/worktrees/issue-720-encoded-filename-canonical`. Run `cargo xtask`
  via `devtool run --`, never bare `ctx_execute` (which targets the main repo →
  false pass).
- **ADR-0063 newtype conventions** apply, including the deliberate D4a
  deviation.
- `MAX_FILENAME_ENCODED_BYTES` stays `255`. `MEDIA_SEGMENT_ENCODE_SET` stays
  `NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~')`.

## Task list

1. File the restore-validation follow-up issue (separable concern, out of scope
   per spec).
2. Add `Filename::decoded()` — additive, no behavior change.
3. Add `ProfferedFilename` as an _identity_ wrapper and rewire the three inbound
   doors — no behavior change, so the gate stays green.
4. **The flip.** `sanitized` encodes; `FromStr` gains canonicity +
   decoded-safe-leaf + plain length; `ProfferedFilename::from_str` encodes;
   `media_path`/`media_url`/AtomPub member interpolate;
   `encode_filename_segment` deleted; the assertions that invert are updated.
5. Display sites decode: the media-row binding split, `content_disposition`, the
   Atom `<title>`, `detect_content_type`.
6. The `proffered-filename-position` xtask gate.
7. Docs: narrow ADR-0080's coupling note, update `common/src/media.rs`'s module
   doc and the serve route's re-encode comment.

## Key risks / decisions

- **Losing the safe-leaf guard (spec D3).** Canonicity does not imply a safe
  leaf — `a%2Fb.jpg` decodes to `a/b.jpg`. The guard must run on `decode(s)`,
  never on `s`. Task 4 Step 1 writes the test that catches its omission (AC4)
  before the implementation.
- **Task ordering.** Task 3 is a deliberate no-op refactor. Without it, Task 4
  would have to change the type invariant, every path builder, and all three
  route signatures in one commit; with it, the routes are already in place.
- **`ProfferedFilename`'s trailer (D4a).** A default `#[derive(StrNewtype)]`
  emits the sqlx bridge, which the Task 6 gate _cannot_ catch (it scans type
  positions, not query binds). `FromStr` + `Deserialize` only.
- **`get_content_type` keeps its `&str` signature** — it is `pub` and
  unit-tested with string literals at `storage/src/media_manager.rs:474-486`.
  The decode happens at its callers.

---

### Task 1: File the restore-validation follow-up

The spec's "Out of scope" surfaces one separable concern: backup **restore** is
untyped (`storage/src/backup.rs:314` reads rows as
`serde_json::Map<String, Value>`, `:345` binds each cell as text), so a pre-#720
backup restores "successfully" and only fails later as a sqlx `Decode` error on
read. Filed now so it can be picked up concurrently, per `jaunder-issues`.

**Files:** none (tracker only).

**Interfaces:** Produces: an issue number to cite in Task 7's ADR-0080 edit.

- [x] **Step 1: File the issue** via `jaunder-issues` conventions — type `Task`,
      milestone `Correctness & data integrity`, label `needs-triage`.

Filed as **[#725](https://github.com/jaunder-org/jaunder/issues/725)** (type
`Task`, labels `data-integrity` + `needs-triage`, added to Jaunder Backlog #1).
Note: there is no `storage` label in this repo — `data-integrity` is the topic
label that fits.

Title:
`backup: restore does not validate typed columns, so a malformed value fails only at read time`

Body must state: restore is per-table generic and binds every cell as text,
never constructing a domain newtype; a row that violates a column's newtype
invariant is written unchallenged and surfaces as a sqlx `Decode` error on the
next read (the mechanism pinned by `storage/src/media.rs:425`) — a 500 from the
media library, or a silently skipped row via `list_media`'s skip path. Note that
#720 makes this concrete for `media.filename` but the gap is generic. Reference
`docs/adr/drafts/media-filename-encoded-canonical.md`.

- [x] **Step 2: Record the number** in this plan — Task 7 Step 1 now cites #725.

---

### Task 2: `Filename::decoded()`

Additive accessor, no behavior change — under the current raw-column regime
`decoded()` is the identity for every value, which is exactly what the tests
pin. Task 4 changes what it observes without changing its signature.

**Files:**

- Modify: `common/src/media.rs` (impl block at `:218-244`; tests at `:1120+`)

**Interfaces:**

- Produces: `pub fn Filename::decoded(&self) -> std::borrow::Cow<'_, str>`

- [x] **Step 1: Write the failing tests**

In `common/src/media.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn decoded_is_identity_for_a_name_with_nothing_encoded() {
    let f = Filename::sanitized("photo.jpg").expect("valid leaf");
    assert_eq!(f.decoded(), "photo.jpg");
}

#[test]
fn decoded_borrows_when_there_is_nothing_to_decode() {
    let f = Filename::sanitized("photo.jpg").expect("valid leaf");
    assert!(matches!(f.decoded(), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn decoded_undoes_percent_escapes() {
    // Constructed through `FromStr` so this test states the intended post-#720
    // relationship directly, independent of what `sanitized` currently stores.
    let f: Filename = "my%20photo.jpg".parse().expect("a safe leaf today");
    assert_eq!(f.decoded(), "my photo.jpg");
}

#[test]
fn decoded_recovers_a_literal_percent() {
    let f: Filename = "50%25.jpg".parse().expect("a safe leaf today");
    assert_eq!(f.decoded(), "50%.jpg");
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common decoded` Expected: FAIL — no
method named `decoded` found for struct `Filename`

Observed:
`error[E0599]: no method named 'decoded' found for struct 'media::Filename'`,
×4.

- [x] **Step 3: Implement against the tests**

Add to the `impl Filename` block, to signature
`pub fn decoded(&self) -> std::borrow::Cow<'_, str>`. Body is
`percent_encoding::percent_decode_str(&self.0).decode_utf8_lossy()`. The tests
pin both branches (`Borrowed` when nothing is escaped, `Owned` when escapes are
present) and both escape shapes.

Doc comment must state **why lossy is safe**: once D3 lands, a canonical value's
escapes were produced by encoding valid UTF-8, and a lone invalid byte such as
`%FF` fails the canonicity check (`decode` yields U+FFFD, which re-encodes to
`%EF%BF%BD` ≠ `%FF`) — so the lossy substitution is unreachable on a `Filename`.

- [x] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common decoded` Expected: PASS (4
tests)

Observed: `4 tests run: 4 passed, 428 skipped`. Full `cargo xtask check` green.

- [x] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/media.rs
git commit -m "feat(media): add Filename::decoded for display sites (#720)"
```

---

### Task 3: `ProfferedFilename` as an identity wrapper; rewire the three inbound doors

Deliberately behavior-preserving. `ProfferedFilename::from_str` applies the same
safe-leaf oracle `Filename::from_str` applies today and stores the value
unchanged, so every route behaves exactly as before — including the 400 at
`server/tests/atompub/atompub_media.rs:293`, which must keep passing
**unmodified** (AC9).

**Files:**

- Modify: `common/src/media.rs` (new type + tests)
- Modify: `server/src/media.rs:62-68` (`ServeParams.filename`), `:215-234`
  (`validate_serve_params`)
- Modify: `server/src/atompub/media.rs:151`, `:176` (member `GET`/`DELETE`
  extractors)

**Interfaces:**

- Consumes: `Filename` (unchanged).
- Produces:
  - `#[derive(Clone, Debug)] pub struct ProfferedFilename(String);` in
    `common::media`
  - `impl FromStr for ProfferedFilename { type Err = InvalidFilename; }`
  - `impl<'de> Deserialize<'de> for ProfferedFilename` (routes through
    `FromStr`)
  - `impl From<ProfferedFilename> for Filename` — **total**, not `TryFrom`

  `Clone` is load-bearing, not ergonomic: `SoftPath::value()` returns
  `Option<&T>` (`server/src/soft_path.rs`), so the serve route needs an owned
  value to feed `From<ProfferedFilename>`. `Debug` is required by the extractor
  error paths. Everything else in the standard trailer stays off (spec D4a).

- [ ] **Step 1: Write the failing tests**

In `common/src/media.rs`'s test module:

```rust
#[test]
fn proffered_accepts_a_safe_leaf() {
    let p: ProfferedFilename = "photo.jpg".parse().expect("a safe leaf");
    assert_eq!(Filename::from(p), "photo.jpg");
}

#[test]
fn proffered_rejects_a_traversal_or_separator_value() {
    // The decoded segment axum hands us: `a\b.png` is not a leaf, and today's
    // member route answers 400 for it. Pinned here so Task 4's re-encode cannot
    // silently turn that into a 404.
    for bad in ["a\\b.png", "a/b.png", "..", ".", "", "a\0b.png"] {
        assert!(
            bad.parse::<ProfferedFilename>().is_err(),
            "must reject {bad:?}"
        );
    }
}

#[test]
fn proffered_rejects_an_over_long_name_rather_than_truncating() {
    let long = format!("{}.jpg", "a".repeat(MAX_FILENAME_ENCODED_BYTES));
    assert!(long.parse::<ProfferedFilename>().is_err());
}

#[test]
fn proffered_converts_into_filename_infallibly() {
    // The conversion is a `From`, not a `TryFrom` — this compiles only if it is total.
    let p: ProfferedFilename = "photo.jpg".parse().expect("a safe leaf");
    let f: Filename = p.into();
    assert_eq!(f, "photo.jpg");
}

#[test]
fn proffered_deserializes_through_its_validating_door() {
    assert!(serde_json::from_str::<ProfferedFilename>("\"photo.jpg\"").is_ok());
    assert!(serde_json::from_str::<ProfferedFilename>("\"a/b.png\"").is_err());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common proffered` Expected: FAIL —
cannot find type `ProfferedFilename` in this scope

- [ ] **Step 3: Implement against the tests**

Define `ProfferedFilename` in `common/src/media.rs`, next to `Filename` so the
pair reads together. **Do not use `#[derive(StrNewtype)]`** — hand-write
`FromStr` and `Deserialize` only (D4a). No `Display`, `Serialize`, `Deref`,
`AsRef`, or sqlx bridge.

`FromStr` body, to signature
`fn from_str(s: &str) -> Result<Self, InvalidFilename>`: reject
`InvalidFilename::NotASafeLeaf` when
`s.is_empty() || sanitize_filename(s) != s`; reject
`InvalidFilename::TooLong { encoded }` when
`encoded_len(s) > MAX_FILENAME_ENCODED_BYTES`; otherwise
`Ok(ProfferedFilename(s.to_owned()))`. (This mirrors `Filename::from_str`'s
current body exactly — that is what makes this task a no-op. Task 4 inserts the
encode.)

`Deserialize` routes through `FromStr` via `String::deserialize` then
`map_err(de::Error::custom)`, mirroring `SoftPath`'s approach in
`server/src/soft_path.rs`.

`From<ProfferedFilename> for Filename` is `Filename(value.0)`.

**AC7 — pin the absent trailer with compile-fail doctests** on the type, in the
same style as `Filename`'s private-field pair at `common/src/media.rs:163-169`.
These guard against a _future_ `Display`/`Serialize` impl being added by reflex,
which is the whole point of D4a and is not covered by "the impl does not exist
today":

````rust
/// ```compile_fail
/// let p: common::media::ProfferedFilename = "a.jpg".parse().unwrap();
/// let _ = p.to_string(); // no Display: the inbound twin is never rendered
/// ```
/// ```compile_fail
/// let p: common::media::ProfferedFilename = "a.jpg".parse().unwrap();
/// let _ = serde_json::to_string(&p); // no Serialize: inbound only
/// ```
````

Doc comment must state: this is the inbound twin for the three axum routes that
receive a **decoded** path segment; it exists because encoding is not
idempotent, so one `FromStr` cannot serve both a decoded and an already-encoded
input; and it carries a deliberately minimal trailer per D4a, with the reasons.

- [ ] **Step 4: Rewire the three doors**

`server/src/media.rs:67` — `pub filename: SoftPath<ProfferedFilename>,`. In
`validate_serve_params` (`:229-233`), convert on the way out:

```rust
let Some(filename) = params.filename.value() else {
    return Err(StatusCode::NOT_FOUND);
};
// `value()` borrows, so clone before the rewrap — this is what `Clone` on
// `ProfferedFilename` is for.
let filename = Filename::from(filename.clone());
```

Return type is unchanged (`(MediaSource, ContentHash, Filename)`), so
`resolve_media_path` and `serve_response` are untouched.

`server/src/atompub/media.rs:151` and `:176` — extractor becomes
`Path((username, sha, filename)): Path<(Username, ContentHash, ProfferedFilename)>`,
with `let filename = Filename::from(filename);` as the first statement of each
body, so the rest of both handlers is unchanged.

- [ ] **Step 5: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common proffered` Expected: PASS — 12
tests. The filter also matches seven pre-existing `ProfferedInviteCode` /
`ProfferedPassword` tests (`common/src/invite.rs:59,66,76`;
`common/src/password.rs:164,171,182,191`); 5 of the 12 are the new ones.

Run: `devtool pg run -- cargo nextest run -p jaunder media` Expected: PASS —
including `member_rejects_malformed_segment_returns_400`, **unmodified**

- [ ] **Step 6: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/media.rs server/src/media.rs server/src/atompub/media.rs
git commit -m "refactor(media): route inbound URL filenames through ProfferedFilename (#720)"
```

---

### Task 4: The flip — the encoded form becomes canonical

The core change (D1, D3, D4, D5). Irreducible: `sanitized` encoding, `FromStr`
tightening, `ProfferedFilename` encoding, and the path builders ceasing to
encode must land in one commit, or the tree double-encodes and the gate goes
red.

**Files:**

- Modify: `common/src/media.rs` — `InvalidFilename` (`:173-194`, **new
  `NotCanonical` variant**), `FromStr` (`:196-216`), `sanitized` (`:234-243`),
  `encode_filename_segment` (`:387-398`, **deleted**), `media_path`
  (`:440-446`), `MAX_FILENAME_ENCODED_BYTES` doc (`:400-408`), and the **eleven
  existing tests inventoried in Step 4**
- Modify: `server/src/media.rs` — its in-file test `:391-410` (Step 4). _The
  handler code is untouched; only the test's premise inverts._
- Modify: `server/src/atompub/media.rs:42-47` (member URL interpolates; drop the
  `encode_filename_segment` import at `:15`)
- Modify: `server/tests/web/web_media.rs:324-325`
- Modify: `end2end/tests/media.spec.ts:58-59` (assertion **and** its "display
  name stays raw" comment)

**Interfaces:**

- Consumes: `ProfferedFilename` (Task 3), `Filename::decoded()` (Task 2).
- Produces: `Filename` now means "the canonical, percent-encoded safe leaf." No
  signature changes — `media_path`, `media_url`, `Filename::sanitized`, and
  `FromStr` keep their current shapes.

- [ ] **Step 1: Write the failing tests**

In `common/src/media.rs`'s test module:

```rust
#[test]
fn sanitized_stores_the_encoded_form() {
    let f = Filename::sanitized("my photo.jpg").expect("valid leaf");
    assert_eq!(f, "my%20photo.jpg");
    assert_eq!(f.decoded(), "my photo.jpg");
}

#[test]
fn from_str_rejects_a_non_canonical_value() {
    // Raw (unencoded) and a lowercase escape are both non-canonical (AC3).
    assert!("my photo.jpg".parse::<Filename>().is_err());
    assert!("my%2fphoto.jpg".parse::<Filename>().is_err());
    assert!("my%20photo.jpg".parse::<Filename>().is_ok());
}

#[test]
fn from_str_rejects_canonical_but_unsafe_values() {
    // AC4 — the test that fails if D3(2)'s decoded-form safe-leaf guard is dropped.
    // Each of these is canonical, non-empty, neither `.` nor `..`, and under the
    // length bound, so only the decoded-form guard rejects it.
    for bad in ["a%2Fb.jpg", "a%00b.jpg", "a%0D%0Ab.jpg", "a%5Cb.jpg"] {
        assert!(
            bad.parse::<Filename>().is_err(),
            "canonical-but-unsafe value must be rejected: {bad:?}"
        );
    }
}

#[test]
fn from_str_length_bound_is_a_plain_byte_count() {
    let at_limit = "a".repeat(MAX_FILENAME_ENCODED_BYTES);
    let over = "a".repeat(MAX_FILENAME_ENCODED_BYTES + 1);
    assert!(at_limit.parse::<Filename>().is_ok());
    assert!(over.parse::<Filename>().is_err());
}

#[test]
fn media_path_interpolates_without_encoding() {
    let f = Filename::sanitized("my photo.jpg").expect("valid leaf");
    let hash = ContentHash::from_digest(sha2::Sha256::digest(b"x").into());
    let path = media_path(&MediaSource::Upload, &hash, &f);
    assert!(path.ends_with("/my%20photo.jpg"), "{path}");
    // The stored value IS the path segment — byte identity, not a derivation (AC1).
    assert!(path.ends_with(&format!("/{f}")), "{path}");
}

#[test]
fn from_str_distinguishes_non_canonical_from_a_bad_leaf() {
    // Check order matters (spec D3): `a/b.txt` is BOTH an unsafe leaf and
    // non-canonical, and must still report the leaf failure — `from_str_still_
    // reports_a_bad_leaf_as_such` (:1197) pins that. A merely-unencoded name
    // reports the new variant.
    assert!(matches!(
        "a/b.txt".parse::<Filename>().expect_err("not a leaf"),
        InvalidFilename::NotASafeLeaf
    ));
    assert!(matches!(
        "my photo.jpg".parse::<Filename>().expect_err("not canonical"),
        InvalidFilename::NotCanonical
    ));
}

#[test]
fn a_literal_percent_round_trips() {
    // AC14 — the case that exposes a double-encode or double-decode.
    let f = Filename::sanitized("50%.jpg").expect("valid leaf");
    assert_eq!(f, "50%25.jpg");
    assert_eq!(f.decoded(), "50%.jpg");
    assert!(f.to_string().parse::<Filename>().is_ok(), "canonical re-parses");
}

#[test]
fn a_user_typed_escape_does_not_materialize_a_separator() {
    // AC14 — `a%2Fb.jpg` typed literally must store double-encoded, so no `/`
    // appears in any derived path segment.
    let f = Filename::sanitized("a%2Fb.jpg").expect("valid leaf");
    assert_eq!(f, "a%252Fb.jpg");
    assert_eq!(f.decoded(), "a%2Fb.jpg");
    let hash = ContentHash::from_digest(sha2::Sha256::digest(b"x").into());
    let path = media_path(&MediaSource::Upload, &hash, &f);
    let segment = path.rsplit('/').next().expect("a trailing segment");
    assert!(!segment.contains('/'));
    assert_eq!(segment, "a%252Fb.jpg");
}

#[test]
fn proffered_re_encodes_the_decoded_segment() {
    // The serve door: axum hands us the decoded name; the stored form must come back.
    let p: ProfferedFilename = "my photo.jpg".parse().expect("a safe leaf");
    assert_eq!(Filename::from(p), "my%20photo.jpg");
}

#[test]
fn proffered_output_always_satisfies_filename() {
    // What makes `From<ProfferedFilename> for Filename` total (AC6).
    for raw in ["photo.jpg", "my photo.jpg", "50%.jpg", "résumé.pdf", ".hiddenfile"] {
        let p: ProfferedFilename = raw.parse().expect("a safe leaf");
        let f = Filename::from(p);
        assert!(
            f.to_string().parse::<Filename>().is_ok(),
            "must re-parse: {raw:?}"
        );
    }
}
```

The existing contract test `sanitized_output_always_reparses_as_filename`
(`:1319+`) stays and is now the load-bearing pin that `sanitized`'s encoded
output satisfies the tightened `FromStr`.

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common media` Expected: FAIL —
`sanitized_stores_the_encoded_form` (gets `my photo.jpg`),
`from_str_rejects_a_non_canonical_value` (raw currently parses),
`from_str_rejects_canonical_but_unsafe_values` (`a%2Fb.jpg` currently parses),
`media_path_interpolates_without_encoding` (double-encodes), and both round-trip
tests.

- [ ] **Step 3: Implement against the tests**

`InvalidFilename` — add a third variant `NotCanonical` (spec D3), message along
the lines of "filename must be percent-encoded in canonical form (this is the
stored spelling; encode it once at the boundary)". Reusing `NotASafeLeaf` would
tell a caller who sent a raw name that their perfectly-good leaf is not a leaf.

`Filename::from_str` — replace the body's checks with, **in this order** (the
order is load-bearing, not incidental):

1. empty / `.` / `..` → `NotASafeLeaf`
2. `sanitize_filename(&decode(s)) != decode(s)` → `NotASafeLeaf`
3. `s != encode(decode(s))` → `NotCanonical`
4. `s.len() > MAX_FILENAME_ENCODED_BYTES` → `TooLong`

(2) precedes (3) because a separator value like `a/b.txt` is _both_ an unsafe
leaf and non-canonical; the existing `from_str_still_reports_a_bad_leaf_as_such`
(`:1197`) pins that it reports the leaf failure, and canonicity-first would
silently reclassify it. The four branches are pinned by
`from_str_rejects_a_non_canonical_value`,
`from_str_rejects_canonical_but_unsafe_values`,
`from_str_length_bound_is_a_plain_byte_count`, and
`from_str_distinguishes_non_canonical_from_a_bad_leaf`.

The decoded-form guard is the subtle one and gets an explicit comment: **run it
on `decode(s)`, never on `s`** — `sanitize_filename("a%2Fb.jpg")` is
`"a%2Fb.jpg"`, so checking the encoded form passes vacuously and silently admits
a separator, a NUL, or CRLF.

`Filename::sanitized` — unchanged through `truncate_to_budget`, then encode as
the final step before the degenerate-leaf guard. Order is
`sanitize → truncate → encode` (D5); the guard still runs on the pre-encode
value, since `.`/`..` survive encoding either way.

`encode_filename_segment` — **delete**. `MEDIA_SEGMENT_ENCODE_SET` and
`encoded_len` stay private; `encoded_len` is still `truncate_to_budget`'s cost
function (D5).

`media_path` — drop the `encode_filename_segment` call; interpolate `filename`
directly.

`server/src/atompub/media.rs:42-47` — interpolate `record.filename` directly in
the member path; remove `encode_filename_segment` from the `:15` import.

`ProfferedFilename::from_str` — insert the encode after the safe-leaf and
`.`/`..` checks and before the length check, so the length bound is measured on
the encoded value (`s.len()` after encoding == `encoded_len(input)` before, so
the check is equivalent either way; measure after, to match `Filename`'s
plain-byte rule).

`MAX_FILENAME_ENCODED_BYTES`'s doc — the invariant no longer depends on the
encode set; the _intake budget_ does. `InvalidFilename::TooLong`'s message stays
as-is: it is still reporting an encoded length, and the explanation is still the
reason a short-looking name is rejected.

- [ ] **Step 4: Repair the eleven existing tests the flip invalidates**

This is the largest part of the task and the easiest to under-scope. Three of
these need their **meaning** re-decided, not just a literal updated. Work the
list; do not search-and-replace.

**(a) `common/src/media.rs:784` `layout_args` — the shared helper.** It builds a
`Filename` via `test_support::parse_filename`, i.e. `FromStr`, so every raw name
it is called with now panics. Change it to construct through the _intake_ door,
`Filename::sanitized(name)`, so callers keep passing the name a user would type
and the helper yields the canonical value. This fixes `:813` and `:845` at the
source; `:830` (`photo.jpg`) is unaffected either way.

**(b) `:813`
`media_url_is_representable_for_names_the_newtype_would_otherwise_reject`.**
Survives (a) unchanged in intent — the point is still that a name with a space
yields a valid `RootRelativeUrl`. Verify its expected URLs are the encoded
spellings.

**(c) `:845` `media_path_encodes_whitespace_and_url_structural_characters`.**
Its _subject_ is deleted — `media_path` no longer encodes. Rename to
`media_path_interpolates_the_already_encoded_name` and re-point the assertions:
the input is now `Filename::sanitized(raw)` and the expectation is that the path
segment is byte-identical to the stored value. Keep all five hazard cases
(space, `?`, `#`, `%`, non-ASCII) — they are still the cases that matter, just
now pinned at intake rather than at path construction.

**(d) `:1166` `from_str_rejects_a_name_whose_encoded_form_exceeds_the_budget`.**
Broken in a subtle way: `"ä".repeat(100)` is now **non-canonical**, so it fails
check (3) and can never reach `TooLong`. #708's meaning has to move to the door
where it still applies. Split it:

```rust
#[test]
fn from_str_rejects_an_over_long_canonical_name() {
    // At `FromStr` the value is already encoded, so the bound is a plain byte count.
    let over = "a".repeat(MAX_FILENAME_ENCODED_BYTES + 1);
    assert!(matches!(
        over.parse::<Filename>().expect_err("over budget"),
        InvalidFilename::TooLong { .. }
    ));
}

#[test]
fn proffered_rejects_a_name_whose_encoded_form_exceeds_the_budget() {
    // #708's original case, relocated: the *proffered* door receives the decoded
    // name, so this is where "100 chars, 200 raw bytes, 600 encoded" is still the
    // hazard a char-count bound would miss.
    let raw = "ä".repeat(100);
    let err = raw.parse::<ProfferedFilename>().expect_err("over budget");
    assert!(matches!(err, InvalidFilename::TooLong { .. }), "{err}");
    let msg = err.to_string();
    assert!(msg.contains("percent-encoded"), "{msg}");
    assert!(msg.contains("255"), "{msg}");
}
```

**(e) `:1185` `from_str_accepts_a_name_exactly_at_the_budget`.** Plain ASCII is
canonical, so this passes unchanged. Verify, do not edit.

**(f) `:1197` `from_str_still_reports_a_bad_leaf_as_such`.** Passes **only** if
Step 3's check order is right (leaf before canonicity). Treat a failure here as
a signal the order was inverted, not as a test to update.

**(g) The six `encoded_len(&f) <= MAX_FILENAME_ENCODED_BYTES` truncation
assertions — `:1212`, `:1224`, `:1239`, `:1256`, `:1262`, `:1286`.** All now
measure the _wrong thing_: `f` already holds escapes, so `encoded_len`
re-encodes `%`→`%25` and the `ä` case measures ~414 against a 255 bound. Each
becomes `assert!(f.len() <= MAX_FILENAME_ENCODED_BYTES, "{f}")`. This is the
re-decision the spec's AC5 anticipates: once the stored value _is_ the encoded
form, "measured in encoded bytes" and "byte length" are the same statement, and
`f.len()` is the honest one. The surrounding test names
(`sanitized_truncation_is_measured_in_encoded_bytes_not_characters` and friends)
stay accurate — the _budget_ is still measured in encoded bytes at intake (D5);
it is only the assertion's spelling that changes.

**(h) `server/src/media.rs:391-410`
`resolve_media_path_encodes_the_filename_like_the_writer_does`.** Its
`assert_eq!` on the returned `Filename` now sees the encoded spelling for all
four cases, and its comment ("axum hands us the decoded segment, so resolving
has to re-encode") now describes `ProfferedFilename`'s job, not `media_path`'s.
Rename to
`resolve_media_path_recovers_the_stored_spelling_from_a_decoded_segment`, assert
the returned `Filename` equals the `stored` column of each pair, and rewrite the
comment accordingly. The handler code itself needs no change — this is the test
that proves Task 3's rewiring now carries the encode.

**(i) `server/tests/web/web_media.rs:324-325`** →
`assert_eq!(resp.filename, "my%20photo.jpg");`, and rewrite the neighbouring
comment naming the value as the raw display name.

**(j) `end2end/tests/media.spec.ts:58-59`** →
`expect(json.filename).toBe("my%20holiday%20photo.jpg");` and rewrite the "The
display name stays raw" comment: the wire field is the canonical encoded form
because it is a lookup key (D7); display surfaces decode it.

**Completeness check for this step.** Before moving on, run
`rg -n 'encoded_len\(&|parse_filename\(|\.filename, "' common/src server/src server/tests`
and confirm every hit is either in this list or provably unaffected. A raw-name
assertion that survives to Task 5 is a red gate at the wrong commit.

- [ ] **Step 5: Add the two missing AtomPub round-trip tests (AC8)**

The serve route's encoding round-trip is covered by
`server/tests/web/web_media.rs:301 upload_then_serve_round_trips_a_filename_needing_encoding`,
but `server/tests/atompub/atompub_media.rs` uses `pic.png` everywhere — so the
two member routes, which are exactly where `ProfferedFilename`'s re-encode is
load-bearing, currently have **no test that fails if the re-encode were
dropped**. Add both, dual-backend per `CONTRIBUTING.md`:

```rust
#[apply(backends)]
async fn member_get_resolves_a_filename_needing_encoding(#[case] backend: Backend) {
    // Upload via Slug `my photo.jpg`, then GET the member URL the entry advertised.
    // axum decodes the segment; only ProfferedFilename's re-encode recovers the
    // stored `my%20photo.jpg`, so this 200 is the re-encode's proof.
    // → assert status 200 and that the entry's <title> round-trips.
}

#[apply(backends)]
async fn member_delete_resolves_a_filename_needing_encoding(#[case] backend: Backend) {
    // Same setup; DELETE the member URL, assert 204/200 and that a follow-up GET
    // is 404 — proving the delete matched the stored row rather than missing.
}
```

Build both on the existing fixtures in that file (the upload helper and
`member_uri` extraction used by `member_rejects_malformed_segment_returns_400`'s
neighbours), so they follow the file's established shape rather than inventing a
second harness.

Add AC6's **integration-level** truncation assertion here too — the spec is
specific that the weaker "an over-long segment does not resolve" form does not
discriminate, since a name that never matched anything does not resolve either:

```rust
#[apply(backends)]
async fn an_over_long_segment_does_not_truncate_onto_a_stored_name(#[case] backend: Backend) {
    // Store a name sitting exactly at the budget, then request a LONGER segment
    // whose truncation would land on it. `ProfferedFilename` rejects rather than
    // repairs, so this must miss — if it ever 200s, truncation crept back into the
    // inbound door and one user's URL is reaching another's file.
}
```

- [ ] **Step 6: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common media` Expected: PASS

Run: `devtool pg run -- cargo nextest run -p jaunder media` Expected: PASS —
including `member_rejects_malformed_segment_returns_400` unmodified (AC9) and
both new member round-trips

Run: `devtool run -- cargo xtask e2e-local media.spec.ts` Expected: PASS —
including "a filename needing percent-encoding uploads and serves"

- [ ] **Step 7: Commit**

```bash
devtool run -- cargo xtask check
git add common/src/media.rs server/src/media.rs server/src/atompub/media.rs server/tests/web/web_media.rs server/tests/atompub/atompub_media.rs end2end/tests/media.spec.ts
git commit -m "feat(media)!: make the percent-encoded filename canonical in the database (#720)"
```

---

### Task 5: Display sites decode

After Task 4 the tree is correct but shows encoded names. This task makes every
cosmetic surface decode (D6), and splits the one binding that serves two roles.

**Files:**

- Modify: `web/src/media/component.rs:289-310`
- Modify: `server/src/media.rs:132`, `:134` (pass decoded to both helpers)
- Modify: `storage/src/media_manager.rs:115`, `:363`
- Modify: `common/src/atompub/entry.rs` (the `<title>` render site)
- Test: `server/src/media.rs` in-file tests; `server/tests/web/web_media.rs`;
  `common/src/atompub/entry.rs` in-file tests; `end2end/tests/media.spec.ts`

**Interfaces:**

- Consumes: `Filename::decoded()` (Task 2).
- Produces: no new API.

- [ ] **Step 1: Write the failing tests**

`server/src/media.rs` in-file tests — AC12, both header parameters as exact
strings:

```rust
#[test]
fn content_disposition_carries_decoded_and_rfc5987_forms() {
    // The argument is the *decoded* name; the helper's own `NON_ALPHANUMERIC`
    // encode is the RFC 5987 one and is unrelated to the media segment set.
    let value = content_disposition("image/png", "my photo.jpg");
    assert!(value.contains("filename=\"my photo.jpg\""), "{value}");
    assert!(value.contains("filename*=UTF-8''my%20photo.jpg"), "{value}");
    // A double-encode would show `my%2520photo.jpg` here.
    assert!(!value.contains("%2520"), "{value}");
}
```

`common/src/atompub/entry.rs` in-file tests — AC13:

```rust
#[test]
fn media_link_entry_title_is_the_decoded_filename() {
    let entry = media_link_entry_fixture("my%20photo.jpg");
    let xml = render_media_link_entry(&entry);
    assert!(xml.contains("<title>my photo.jpg</title>"), "{xml}");
}
```

**AC10 and AC11 both go to Playwright, not `web_media.rs`.** `render_media_row`
(`web/src/media/component.rs:282`) is a CSR Leptos view — the media library is a
static shell with no SSR, so no server test ever renders that row, and
`web_media.rs` has no HTML-body assertions to build on.
`end2end/tests/media.spec.ts` is the only surface that observes both bindings:

```ts
test("the media row decodes its label but not its delete key", async ({
  page,
}) => {
  // AC10 — one binding used to serve both roles (component.rs:289). The label is
  // cosmetic and decodes; the hidden field is the key that round-trips to
  // `delete_media(filename: Filename)` and must stay canonical.
  const row = page.getByRole("row", { name: /my holiday photo\.jpg/ });
  await expect(row.getByRole("link")).toHaveText("my holiday photo.jpg");
  await expect(row.locator('input[name="filename"]')).toHaveValue(
    "my%20holiday%20photo.jpg",
  );
});

test("a name needing percent-encoding deletes from the media library", async ({
  page,
}) => {
  // AC11 — the end-to-end check that the hidden field was NOT decoded. A decoded
  // key is rejected by `Filename`'s wire door, so the delete would fail outright.
  page.on("dialog", (d) => d.accept()); // the row's onclick confirm()
  await page.getByRole("button", { name: "Delete" }).click();
  await expect(
    page.getByRole("link", { name: "my holiday photo.jpg" }),
  ).toHaveCount(0);
});
```

Both build on the existing upload flow in that file (the `my holiday photo.jpg`
fixture at `:36-65`), so they need no new harness.

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool pg run -- cargo nextest run -p jaunder media` Expected: FAIL —
`content_disposition` receives the encoded name, so `filename=` shows
`my%20photo.jpg` and `filename*=` shows the double-encoded `my%2520photo.jpg`;
the rendered row shows the encoded name as its link text.

Run: `devtool run -- cargo nextest run -p common atompub` Expected: FAIL —
`<title>my%20photo.jpg</title>`

- [ ] **Step 3: Implement against the tests**

`web/src/media/component.rs:289` — split into two bindings:

```rust
// Two roles, two spellings (#720, spec D6). The link text is cosmetic and decodes;
// the hidden field is the *key* that round-trips to `delete_media(filename: Filename)`,
// so it must stay canonical — decoding it would make every delete fail at the wire door.
let display_name = item.filename.decoded().into_owned();
let filename_key = item.filename.to_string();
```

Line 300 uses `display_name`; line 310's `value=` uses `filename_key`.

`server/src/media.rs:132`/`:134` — pass `&filename.decoded()` to
`detect_content_type` and to `content_disposition`. `content_disposition`'s doc
comment gains a sentence: it takes the **decoded** name, and its internal
`NON_ALPHANUMERIC` encode is the RFC 5987 one — a different set from the media
segment's, deliberately (ADR-0080).

`storage/src/media_manager.rs:115` and `:363` —
`Self::get_content_type(content_type, &filename.decoded())`.
`get_content_type`'s `&str` signature and its literal-driven tests at `:474-486`
are unchanged. (`:363` always passes `Some(content_type)`, so it never reaches
the detect branch; changed anyway so the two callers cannot drift.)

`common/src/atompub/entry.rs` — the `<title>` render site emits
`&entry.title.decoded()` (note the `&`: `write_text_element` at `:622` takes
`&str`, and `decoded()` returns a `Cow`). The field stays typed `Filename` (D6);
the `MediaLinkEntry.title` doc comment says the value is canonical and the
renderer decodes.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool pg run -- cargo nextest run -p jaunder media` Expected: PASS

Run: `devtool run -- cargo nextest run -p common atompub` Expected: PASS

Run: `devtool run -- cargo xtask e2e-local media.spec.ts` Expected: PASS

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add web/src/media/component.rs server/src/media.rs storage/src/media_manager.rs common/src/atompub/entry.rs server/tests/web/web_media.rs end2end/tests/media.spec.ts
git commit -m "feat(media): decode filenames at every display surface (#720)"
```

---

### Task 6: The `proffered-filename-position` gate

`ProfferedFilename` must be `pub` for the route signatures, so privacy cannot
confine it (D8). The discriminator is **bare versus wrapped** — the serve
route's legitimate position _is_ a struct field, so a "no struct fields" rule
would be undecidable.

**Files:**

- Create: `xtask/src/steps/proffered_filename_check.rs`
- Modify: `xtask/src/lib.rs:31` (module decl), `:343` and `:380` (registration
  alongside `proffered_secret_check::run`)

**Interfaces:**

- Produces: `pub fn problems(scanned: &[(String, String)]) -> Option<String>`
  and `pub fn run(result: &mut CommandResult)`, matching
  `proffered_secret_check`'s shape.

- [ ] **Step 1: Write the failing tests**

In `xtask/src/steps/proffered_filename_check.rs`'s `#[cfg(test)] mod tests`,
mirroring `proffered_secret_check`'s fixture style:

```rust
const SOFT_PATH_FIELD: &str = r#"
pub struct ServeParams {
    pub filename: SoftPath<ProfferedFilename>,
}
"#;

const PATH_TUPLE: &str = r#"
pub async fn member_get(
    Path((username, sha, filename)): Path<(Username, ContentHash, ProfferedFilename)>,
) -> Result<Response, HandlerError> { todo!() }
"#;

const BARE_STRUCT_FIELD: &str = r#"
pub struct MediaItem {
    pub filename: ProfferedFilename,
}
"#;

const SERVER_PARAM: &str = r#"
#[server]
pub async fn delete_media(filename: ProfferedFilename) -> WebResult<()> { todo!() }
"#;

const BARE_RETURN: &str = r#"
pub fn parse_it() -> ProfferedFilename { todo!() }
"#;

const PLAIN_FN_PARAM: &str = r#"
fn helper(filename: ProfferedFilename) {}
"#;

const IMPORT_AND_COMMENT: &str = r#"
use common::media::ProfferedFilename;
// `ProfferedFilename` is the inbound twin for URL path segments.
"#;

#[test]
fn wrapped_extractor_positions_are_allowed() {
    assert!(violations(SOFT_PATH_FIELD).is_empty());
    assert!(violations(PATH_TUPLE).is_empty());
}

#[test]
fn a_bare_struct_field_is_a_violation() {
    assert_eq!(violations(BARE_STRUCT_FIELD), vec![3]);
}

#[test]
fn a_server_parameter_is_a_violation() {
    assert_eq!(violations(SERVER_PARAM), vec![3]);
}

#[test]
fn a_return_position_is_a_violation() {
    assert_eq!(violations(BARE_RETURN), vec![2]);
}

#[test]
fn a_plain_fn_parameter_is_a_violation() {
    assert_eq!(violations(PLAIN_FN_PARAM), vec![2]);
}

#[test]
fn imports_and_comments_are_allowed() {
    assert!(violations(IMPORT_AND_COMMENT).is_empty());
}

#[test]
fn problems_reports_the_offending_path_and_line() {
    let scanned = vec![("web/src/media/api.rs".to_owned(), BARE_STRUCT_FIELD.to_owned())];
    let detail = problems(&scanned).expect("a violation");
    assert!(detail.contains("web/src/media/api.rs"), "{detail}");
    assert!(detail.contains("`ProfferedFilename`"), "{detail}");
}

#[test]
fn the_owner_file_is_exempt() {
    let scanned = vec![("common/src/media.rs".to_owned(), BARE_RETURN.to_owned())];
    assert!(problems(&scanned).is_none());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml proffered_filename`
Expected: FAIL — no such module

- [ ] **Step 3: Implement against the tests**

**Duplicate `proffered_secret_check`'s `type_index` whole-word helper into this
file with a pointer comment naming the original — do not extract it.**
Extracting would refactor an existing gate this issue did not authorize
touching, and the function is nine lines. Note the duplication in the module doc
so a later consolidation is a deliberate choice rather than a discovery.

`fn violations(source: &str) -> Vec<usize>` — for each line, find the whole-word
`ProfferedFilename`; skip lines whose trimmed form starts with `//`, `use `, or
`pub use `; otherwise it is a violation **unless wrapped**, where wrapped means
either:

- `line[..idx]` ends with `SoftPath<`, or
- `line[..idx]` contains `Path<(` **and** `line[idx..]` contains `)>`.

The seven fixtures pin every branch: both wrapped shapes, all four bare
positions, and the import/comment exemption.

`POLICED_ROOTS` is `proffered_secret_check`'s list verbatim (every crate that
can name the type). The owner file is `common/src/media.rs`.

Module doc must state the bare-versus-wrapped rationale and carry the same
accepted per-line-matching limitation note `proffered_secret_check:86-89`
carries.

- [ ] **Step 4: Register the step**

`xtask/src/lib.rs:31` — `pub mod proffered_filename_check;` `:343` and `:380` —
`steps::proffered_filename_check::run(&mut result);` immediately after each
`proffered_secret_check::run` call, so it runs in both the `check` and
`validate` ladders.

- [ ] **Step 5: Run the tests, verify they pass**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml proffered_filename`
Expected: PASS (8 tests)

- [ ] **Step 6: Prove it bites on the real tree**

Temporarily add **both** lines to `web/src/media/api.rs`:

```rust
use common::media::ProfferedFilename;
pub fn leak() -> ProfferedFilename { todo!() }
```

The `use` is essential — without it `check --no-test` fails on an
unresolved-type compile error _before_ the gate runs, and the step would appear
to pass for entirely the wrong reason. Run
`devtool run -- cargo xtask check --no-test` and confirm the failure names the
`proffered-filename-position` step, `web/src/media/api.rs`, and the `leak` line
(the `use` line must **not** be reported). Revert both lines and confirm `check`
goes green again. Do not commit the temporary edit.

- [ ] **Step 7: Commit**

```bash
devtool run -- cargo xtask check
git add xtask/src/steps/proffered_filename_check.rs xtask/src/lib.rs
git commit -m "feat(xtask): confine ProfferedFilename to axum extractor positions (#720)"
```

---

### Task 7: Docs — narrow ADR-0080, refresh the stale prose

The comments that assert the now-reversed arrangement are the next reader's map,
so a stale one is a defect (AC18, AC19). The ADR draft itself is already written
at `docs/adr/drafts/media-filename-encoded-canonical.md` and is numbered by
`cargo xtask adr promote` at ship, not here.

**Files:**

- Modify: `docs/adr/0080-media-path-naming-correspondence.md` — the DB bullet
  (`:59-60`), the `encode_filename_segment` bullet (`:52-58`), the
  three-spellings table (`:71-77`), the serve-route re-encode bullet
  (`:100-104`), the coupling note (`:110-114`)
- Modify: `common/src/media.rs` — module doc `:15`, `:21-22`, `:28-29`,
  `:31-43`; the `Filename` type doc `:129-169`; `media_path`'s doc `:430-432`
- Modify: `server/src/media.rs:243-248` (the serve route's re-encode comment)
- Modify: `server/tests/atompub/atompub_media.rs:273-276`, `:293-294` (comments
  naming `Filename` as the extractor type)

**Interfaces:** none.

- [ ] **Step 1: Narrow ADR-0080**

`:59-60` — replace "The database `filename` column keeps the raw name" with a
pointer: this was reversed by
`docs/adr/drafts/media-filename-encoded-canonical.md` (#720); the column now
holds the encoded form and display decodes.

**Write that path as a bare code span, never a markdown link.**
`docs/adr/drafts/` is gitignored (`.gitignore:48`), but
`xtask/src/doc_links.rs:224-229` resolves link targets against the **on-disk**
tree while gating tracked files — so a link resolves on your box (where the
draft exists) and fails on a clean CI checkout (where it does not). A code span
is also what `adr promote` Pass C needs: `xtask/src/adr.rs:298-305` rewrites
bare `drafts/<slug>` path-form references repo-wide to the numbered path.

`:52-58` — the bullet says the AtomPub member URL "shares only
`encode_filename_segment`, which is public for exactly that reason." That
function is deleted in Task 4. Rewrite: the member URL now interpolates the
already-canonical value, and the encode set is private again.

`:100-104` — the bullet asserting "the serve route's re-encode is **not**
redundant … `media_path` re-encodes it" is the ADR twin of the code comment Step
3 rewrites. Same fix: the re-encode now lives in `ProfferedFilename`'s door, and
`media_path` interpolates. Keep the warning that it looks removable and is not.

`:71-77` — the three-spellings table's Database row becomes `encoded`, and the
closing line "**URL → disk is identity. DB → disk requires encoding**" becomes:
DB, disk and URL are one spelling; the derivations are display-decode and the
inbound-door re-encode.

`:110-114` — the coupling note keeps its first half (the bound is on the encoded
form, a char count cannot express it) and its second half is narrowed: the
dependency is a property of `Filename::sanitized`'s intake **budget**, not of
`Filename`'s invariant, which is now a plain byte length. Widening the encode
set still shrinks the set of names that survive intake intact, so the two are
still revisited together.

Add a line noting that restore does not validate typed columns, citing `#725`
from Task 1.

- [ ] **Step 2: Refresh `common/src/media.rs`'s module doc**

`:21-22` — the column holds the **encoded** name, identical to the on-disk name
and the URL segment; the media list decodes for display.

`:28-29` — "the three spellings … (raw in the database, encoded on disk and in
URLs)" becomes one canonical spelling plus a decoded display view, still
pointing at ADR-0080 **and** the new draft.

`:31-43` ("Untrusted input") — `Filename`'s `FromStr` no longer "admits only an
already-safe leaf"; it admits only a **canonical encoded** value whose decoded
form is a safe leaf, and `ProfferedFilename` is named as the inbound door for
URL segments.

`:15` — "The `<filename>` segment is percent-encoded
([`MEDIA_SEGMENT_ENCODE_SET`])" now describes where the value _came from_, not
something `media_path` does. Same for `media_path`'s own doc at `:430-432`.

`:129-169` — **the `Filename` type doc, the most-read prose on this type and the
one that goes flatly wrong.** It currently says the value is "the canonical form
produced by `sanitize_filename`" and that `FromStr` "accepts a string iff it is
already a canonical leaf (`sanitize_filename(s) == s`)". Rewrite the whole
two-door explanation: the value is the canonical percent-encoded leaf; `FromStr`
validates canonicity _and_ that the decoded form is a safe leaf, in that order
and for the reason in spec D3; `sanitized` normalizes, truncates, then encodes;
and `ProfferedFilename` is the third door, for URL segments axum has decoded.
Keep the two `compile_fail` doctests at `:163-169` — they still hold.

- [ ] **Step 3: Rewrite the serve route's re-encode comment**

`server/src/media.rs:243-248` currently explains that `media_path` re-encodes
what axum decoded. After Task 4 that is `ProfferedFilename`'s job and
`media_path` does not encode at all. Rewrite to: axum decodes the segment,
`ProfferedFilename`'s door re-encodes it to recover the stored spelling, and
`media_path` then interpolates. Keep the load-bearing warning — it still reads
like something to simplify away, and removing it still breaks serving for any
name needing encoding.

Also update `server/tests/atompub/atompub_media.rs:273-276` and `:293-294`,
whose comments name `Filename` as the member routes' extractor type. The
_assertions_ are correct and stay untouched (AC9); only the prose naming the
type is stale after Task 3.

- [ ] **Step 4: Verify the docs gates**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS —
`adr-readme-parity` and `doc_links` both green. Do **not** hand-edit
`docs/README.md`; it is a generated projection (#196).

Note: the pre-commit hook runs `prettier -w` on prose, which restages. Run
`prettier -w docs/adr/0080-media-path-naming-correspondence.md` before staging
so the formatting lands in this commit.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git add docs/adr/0080-media-path-naming-correspondence.md common/src/media.rs server/src/media.rs server/tests/atompub/atompub_media.rs
git commit -m "docs(media): narrow ADR-0080's coupling note for the encoded column (#720)"
```

---

## Self-review

**Spec coverage.** D1 → Task 4. D2 → Tasks 2, 4. D3 → Task 4 Steps 1/3 (AC3,
AC4, AC5). D4 → Tasks 3, 4. D4a → Task 3 Step 3 (no `StrNewtype` derive). D5 →
Task 4 Step 3 (intake order preserved; `encoded_len` retained). D6 → Task 5, all
four rows of its table. D7 → Task 4 Step 4. D8 → Task 6. D9 → Task 7 plus the
already-written draft.

AC1 → T4 S1 (`media_path_interpolates_without_encoding`). AC2 → T4 S3
(deletion) + T6's privacy-by-deletion. AC3, AC4, AC5 → T4 S1. AC6 → T3 S1 (unit)
**and T4 S5** (the integration-level truncation assertion the spec requires).
AC7 → **T3 S3's two compile-fail doctests**. AC8 → **T4 S5's two new AtomPub
member round-trips**, plus the existing serve round-trip at `web_media.rs:301`.
AC9 → T3 S5 and T4 S6, unmodified test. AC10, AC11 → **T5 S1, in Playwright**
(the media row is a CSR view; no server test renders it). AC12, AC13 → T5 S1.
AC14 → T4 S1. AC15 → T4 S4(i)(j). AC16 → T6 S1 and S6. AC17 → T5 S3. AC18, AC19
→ T7.

**Corrections folded in after the cold plan review** — recorded so a later
reader knows these were found, not overlooked:

- **Task 4 originally listed two inverted assertions; the real count is
  eleven**, across `common/src/media.rs` and `server/src/media.rs` (which was
  missing from its Files list entirely). Three needed their _meaning_
  re-decided, not a literal updated — see S4(d) and S4(g). The "every commit
  leaves the gate green" constraint was false as first written.
- **`InvalidFilename::NotCanonical` and the check order** are new to the spec
  (D3), forced by `from_str_still_reports_a_bad_leaf_as_such`: a separator value
  is both an unsafe leaf and non-canonical, and canonicity-first would silently
  reclassify it.
- **`ProfferedFilename` needs `Clone` + `Debug`** — `SoftPath::value()` borrows,
  so the serve route cannot rewrap without an owned value. Spec D4a amended.
- **`-p jaunder` needs `devtool pg run --`**; the bare form dies on the postgres
  half.
- **AC10 could not be delivered where first placed** — `web_media.rs` has no
  HTML assertions and `render_media_row` is CSR-only.
- **Task 7's ADR-0080 pointer must be a code span, not a link**, or `doc-links`
  passes locally and fails on a clean CI checkout where the gitignored draft is
  absent.
- **Task 6 takes a duplicated `type_index` rather than extracting it** —
  extraction would refactor an existing gate this issue did not authorize.

**Placeholder scan:** one intentional token, `#725` in Task 7 Step 1, which Task
1 Step 2 replaces with the filed issue number. Task 4 S5 and T5 S1's test bodies
are specified by comment rather than written out where the surrounding harness
determines the mechanics (fixture names, status assertions); every one names its
exact subject and the failure it must catch, so none is a "write tests for the
above."

**Type consistency:** `ProfferedFilename` (Task 3) is used under that exact name
in Tasks 4, 6, and 7, with `Clone`/`Debug` derived in Task 3 and relied on in
Task 3 S4. `Filename::decoded()` returns `Cow<'_, str>` in Task 2 and is
consumed as `.decoded()`, `&…decoded()`, and `.decoded().into_owned()` in Tasks
4 and 5, each with the borrow form its call site needs.
`From<ProfferedFilename> for Filename` is a total `From` in Tasks 3 and 4.
`InvalidFilename::NotCanonical` is introduced in Task 4 S3 and used in Task 4
S1. `problems`/`violations` in Task 6 match `proffered_secret_check`'s existing
signatures.
