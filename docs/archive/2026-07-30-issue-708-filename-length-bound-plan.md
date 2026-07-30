# Plan — bound `Filename`'s length so a valid name cannot fail the encoded write (#708)

Spec:
[`docs/archive/2026-07-30-issue-708-filename-length-bound-spec.md`](2026-07-30-issue-708-filename-length-bound-spec.md)
(approved). Decisions referenced as **D1**–**D5**, not restated.

## Review header

**Goal.** Make it impossible to construct a `Filename` whose percent-encoded
form exceeds the filesystem's 255-byte per-component limit, so an over-long name
is a rejection at the strict door and a silent truncation at the intake door —
never an opaque 500 from the file write.

**Scope — in:** `MAX_FILENAME_ENCODED_BYTES`; `InvalidFilename` → enum; the
encoded-length bound in `Filename::from_str`; extension-preserving,
grapheme-safe truncation in `Filename::sanitized`; re-pointing the sanitize
oracle test; an upload round-trip test; an ADR-0080 amendment.

**Scope — out:** changing `MEDIA_SEGMENT_ENCODE_SET` or the media layout
(ADR-0080); the `tar`/ustar 100-byte name-field question (pre-existing, see spec
Out of scope); #711, done next.

**Tasks.**

1. The invariant: const, error enum, `from_str` bound, and `sanitized`
   truncation — one commit.
2. Upload round-trip: a name that previously broke the write now lands
   truncated.
3. Amend ADR-0080's consequence bullet, which currently points at this issue as
   open.

**Key risks and decisions.**

- **T1 is deliberately one commit and cannot be split.** Bounding `from_str`
  without truncating `sanitized` leaves the intake door able to construct a
  violating value (AC1 fails) _and_ reds the existing oracle test; truncating
  `sanitized` without bounding `from_str` leaves the invariant partial. The
  three edits are one invariant.
- **Budget is computed, not assumed.** Every check goes through
  `encode_filename_segment(…).to_string().len()`, never a char count — the whole
  point of D1. A char-count bound is the specific mistake this issue exists to
  avoid.
- **Truncation must stay `from_str`-clean.** `sanitized`'s output must satisfy
  `from_str`, which means truncation may not introduce a path separator, a NUL,
  or the degenerate `.`/ `..`/empty. Cutting on grapheme boundaries keeps UTF-8
  valid; the degenerate guard is explicit (D3 step 4).
- Coverage: new error-enum variants and the pathological branches (extension
  alone over budget, degenerate result) need reachable tests or `cov:ignore`;
  check the gate rather than guessing.

**For agentic workers.** Execute with **`jaunder-iterate`**, delegating via
**`jaunder-dispatch`** where useful. Tick checkboxes here as you go.

## Global constraints

- `cargo xtask check` before each commit (the pre-commit hook runs it anyway;
  running it first leaves a clean staged state). See **`jaunder-commit`**. **No
  `Co-Authored-By`.**
- `common` is wasm-reachable: `Filename` compiles for the CSR build, so no
  host-only API may leak in. Use `cargo xtask check`, whose `wasm-clippy` step
  carries the right flags — do not hand-roll them (`--all-features` on the wasm
  target fails inside `mio`; the real form is in
  `xtask/src/steps/static_checks.rs:58-90`).
- Storage/integration tests are dual-backend per ADR-0053; a bare
  `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern`
  guard.

---

## T1 — The invariant: bound, error, and truncation

- [x] Done — `f4ca64bd`. Proved the tests defend D1 rather than merely pass:
      swapping `encoded_len` for `s.chars().count()` — the exact mistake the
      issue exists to prevent — reds precisely the two intended tests and
      nothing else. Coverage needed no `cov:ignore`; the pathological branches
      are all reachable. Also updated `Filename`'s own doc comment, which still
      described both doors without the bound.

      **Two real bugs found in review and fixed here**, both on the path where the stem's
      *first* grapheme cluster busts the budget (a base character carrying dozens of
      combining marks). Truncation emitted a bare `.jpg` — a **dotfile**, whose
      `Path::extension()` is `None`, so `detect_content_type` answered
      `application/octet-stream` and stored it permanently: precisely the loss D3 keeps the
      extension to prevent. The degenerate guard then reported `NotASafeLeaf` for a name
      that *is* a safe leaf and is merely long, reintroducing the lie D4 removed. Fixed by
      reserving budget for a minimal stem and substituting the existing `"upload"`
      placeholder, so truncation always yields a usable leaf and a long name is never an
      error — which also makes ADR-0080's "no longer an error at all" true as written.
      Both bugs were reproduced before fixing.

      The grapheme walk also moved to `common::text::truncate_by_graphemes`, shared with
      `slugify_title`, which was a near-copy differing only in the per-cluster cost.

**Files**

- `common/src/media.rs` — the const, `InvalidFilename`, `Filename::from_str`,
  `Filename::sanitized`, and the in-file tests.

**Interfaces**

```rust
/// The filesystem's per-path-component limit, in bytes. ext4/XFS/btrfs, APFS and NTFS all
/// cap a single name at 255, and the media layout puts the whole filename in one component
/// (ADR-0080), so this is the entire budget.
///
/// Measured against the **percent-encoded** form, because that is what lands on disk — so
/// this bound depends on [`MEDIA_SEGMENT_ENCODE_SET`]. Changing that set changes what names
/// are representable; revisit this together with it.
const MAX_FILENAME_ENCODED_BYTES: usize = 255;

/// Error returned when a string is not a usable media filename leaf.
#[derive(Debug, Error)]
pub enum InvalidFilename {
    /// Not a canonical single path component.
    #[error("filename must be a non-empty safe path leaf (no path components, `.`/`..`, or null bytes)")]
    NotASafeLeaf,
    /// Longer than the filesystem can store once percent-encoded.
    #[error(
        "filename is too long: {encoded} bytes once percent-encoded, limit {MAX_FILENAME_ENCODED_BYTES} \
         (encoding expands each unsafe byte to `%XX`, so the limit is on the encoded form, not the \
         characters you typed)"
    )]
    TooLong { encoded: usize },
}
```

The message names the encoded length **and** explains the expansion, because "my
90-character name was rejected" is otherwise baffling — D4.

`from_str` gains the bound after the existing canonical-leaf check, so a bad
leaf still reports `NotASafeLeaf`:

```rust
fn from_str(s: &str) -> Result<Self, Self::Err> {
    if s.is_empty() || sanitize_filename(s) != s {
        return Err(InvalidFilename::NotASafeLeaf);
    }
    let candidate = Filename(s.to_owned());
    let encoded = encoded_len(&candidate);
    if encoded > MAX_FILENAME_ENCODED_BYTES {
        return Err(InvalidFilename::TooLong { encoded });
    }
    Ok(candidate)
}
```

`encoded_len` is a private helper over `encode_filename_segment`, so the budget
is computed in exactly one place and both doors share it.

`sanitized` sanitizes, then truncates to fit (D3):
`Path::file_stem()`/`Path::extension()` for the split (**not** a manual last-dot
split — `.hiddenfile` must survive), reserve the extension's encoded length,
fill the stem by grapheme cluster accumulating encoded bytes
(`use unicode_segmentation::UnicodeSegmentation`, as `slug.rs` does), then guard
empty/`.`/ `..` → `NotASafeLeaf`.

**Callers to update:** `InvalidFilename` becoming an enum is source-compatible
at every current use — `media_manager.rs:142` and `atompub/media.rs:93` both
`map_err(|_| …)`, discarding it. Confirm with a workspace build rather than
assuming; if any site matches on it, that site decides how to report the two
cases.

**Tests** (in `common/src/media.rs`)

```rust
#[test]
fn from_str_rejects_a_name_whose_encoded_form_exceeds_the_budget() {
    // Raw length is under the limit; the *encoded* length is not. A char-count bound would
    // accept this — which is the mistake this issue exists to prevent.
    let raw = "ä".repeat(100); // 200 raw bytes, 600 encoded
    let err = raw.parse::<Filename>().expect_err("must reject");
    assert!(matches!(err, InvalidFilename::TooLong { .. }), "{err}");
    assert!(err.to_string().contains("percent-encoded"), "{err}");
}

#[test]
fn from_str_still_reports_a_bad_leaf_as_such() {
    assert!(matches!(
        "../escape".parse::<Filename>().expect_err("must reject"),
        InvalidFilename::NotASafeLeaf
    ));
}

#[test]
fn sanitized_truncates_instead_of_failing_and_keeps_the_extension() {
    let long = format!("{}.jpg", "a".repeat(400));
    let f = Filename::sanitized(&long).expect("must truncate, not fail");
    assert!(f.ends_with(".jpg"), "extension must survive: {f}");
    assert!(encoded_len(&f) <= MAX_FILENAME_ENCODED_BYTES);
    // The property that actually matters (AC3): the stored content type is unchanged.
    assert_eq!(detect_content_type(&f), detect_content_type(&long));
}

#[test]
fn sanitized_truncation_is_measured_in_encoded_bytes_not_characters() {
    // ~3× expansion: a char-count budget would leave this over the filesystem limit.
    let long = format!("{}.png", "ä".repeat(300));
    let f = Filename::sanitized(&long).expect("must truncate");
    assert!(encoded_len(&f) <= MAX_FILENAME_ENCODED_BYTES);
    assert!(f.ends_with(".png"));
}

#[test]
fn sanitized_never_splits_a_grapheme_cluster() {
    // Devanagari: base + combining vowel sign. Splitting them corrupts the character.
    let long = format!("{}.txt", "ि".repeat(200));
    let f = Filename::sanitized(&long).expect("must truncate");
    assert!(encoded_len(&f) <= MAX_FILENAME_ENCODED_BYTES);
    // Re-parsing through the strict door proves the result is a valid, in-budget leaf.
    assert!(f.as_ref().parse::<Filename>().is_ok(), "{f}");
}

#[test]
fn sanitized_preserves_a_dotfile_rather_than_treating_it_as_all_extension() {
    // `Path::extension()` is `None` here; a manual last-dot split would truncate the stem
    // to nothing and destroy the name.
    let f = Filename::sanitized(".hiddenfile").expect("valid leaf");
    assert_eq!(f, ".hiddenfile");
}

#[test]
fn sanitized_truncates_the_whole_name_when_the_extension_alone_is_over_budget() {
    let f = Filename::sanitized(&format!("x.{}", "ä".repeat(300))).expect("must truncate");
    assert!(encoded_len(&f) <= MAX_FILENAME_ENCODED_BYTES);
}

#[test]
fn sanitized_rejects_input_that_truncates_to_a_degenerate_name() {
    // Guard from D3 step 4: a result of "", ".", or ".." is not a filename.
    assert!(Filename::sanitized("..").is_err());
    assert!(Filename::sanitized("").is_err());
}
```

**Re-point the oracle test (D5).**
`sanitize_filename_output_always_reparses_as_filename`
(`common/src/media.rs:1047`) asserts the **free function's** output always
re-parses. That stops being true — `sanitize_filename` does not truncate.
Re-point it at the intake door, which is the property that matters, and add a
long input:

```rust
#[test]
fn sanitized_output_always_reparses_as_filename() {
    // The invariant that matters: whatever the intake door emits must satisfy the strict
    // door. `sanitize_filename` alone no longer guarantees this — it does not truncate
    // (#708) — so the claim belongs to `Filename::sanitized`.
    for raw in [
        "photo.jpg", "../../etc/passwd", "foo/bar/baz.txt", "C:\\Users\\file.txt",
        "file\0name.txt", "a b.txt", ".hidden", "no-ext",
        &"ä".repeat(300),                    // over budget once encoded
        &format!("{}.jpg", "a".repeat(400)), // over budget, extension must survive
    ] {
        let Ok(f) = Filename::sanitized(raw) else { continue };
        assert!(f.as_ref().parse::<Filename>().is_ok(), "sanitized({raw:?}) = {f:?}");
    }
}
```

**Run**

- `cargo nextest run -p common filename` and `… sanitized` — the new tests FAIL
  before the change, PASS after. **Verify at least the encoded-budget one goes
  red first** rather than assuming; that is the assertion the whole issue rests
  on.
- `cargo xtask check`, then commit.

## T2 — Upload round-trip for a name that previously broke the write

- [x] Done — `d2a92011`. Reproduced the original defect before trusting the
      test: with truncation disabled it fails with
      `500 {"server":{"message":"server operation     failed"}}` — verbatim what
      a user saw for an ordinary filename.

Unit tests prove the type cannot hold an over-long name. This proves the
**upload** that used to 500 now succeeds — AC6, and the regression the issue is
actually about.

**Files**

- `server/tests/web/web_media.rs` — beside the #675 round-trip test.

Upload with a filename whose encoded form exceeds 255 bytes (e.g.
`"ä".repeat(200) + ".jpg"`), assert `200`, assert the returned `filename`/`url`
are the **truncated** name, then GET the returned `url` and compare the served
bytes. Before T1 the write fails and this is a 500.

Dual-backend (`#[apply(backends)]`), per ADR-0053.

**Run**

- `cargo nextest run -p jaunder --test integration upload` (postgres cases need
  the xtask gate — bare `nextest` reports a false `ConnectionRefused`).
- `cargo xtask check`, then commit.

## T3 — Amend ADR-0080

- [x] Done — `4aeda639`. Status unchanged, so no `sync-readme`;
      `adr-readme-parity` green.

ADR-0080's Consequences bullet currently says the length ceiling "is tracked as
#708, which must decide whether to bound the raw or the encoded length, and
whether the intake door truncates or rejects." Once this lands that sentence
points at a closed issue and states the decision as open. Amend it to record the
answer: bounded on the **encoded** form in `Filename`'s doors, `from_str`
rejects, `sanitized` truncates preserving the extension.

This is an edit to an **accepted** ADR, not a new one — no `promote`, no number.
Per **`jaunder-adr`**, do not touch `docs/README.md` (generated); the status
line is unchanged, so no `sync-readme` either.

**Run** — `cargo xtask check` (`adr-format`, `adr-readme-parity`, `doc-links`),
then commit.

## Self-review

- Each task is one commit and leaves the tree green. T1 is large because the
  three edits are one invariant — splitting it would leave AC1 violated or the
  oracle test red mid-branch.
- No task introduces scaffolding a later task removes.
- Spec coverage: D1 → T1 (const + `encoded_len`); D2 → T1 (both doors); D3 → T1
  (truncation algorithm + dotfile/degenerate/extension tests); D4 → T1 (error
  enum); D5 → T1 (re-pointed oracle test). AC1–5, 7 → T1; AC6 → T2; AC8 → the
  per-task gate. The ADR amendment (T3) is not an AC but keeps ADR-0080 true.
