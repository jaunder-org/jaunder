//! Reads what the doctest runner actually evaluated, out of its output.
//!
//! The reconciliation key is `(file, line)`, and it is exact: libtest prints the
//! fence's **opening** line. Verified against this tree —
//! `common/src/token.rs - token::RawToken (line 56)` for the fence opening at
//! `token.rs:56`, and likewise `:59/:64/:69`, `etag.rs:35/:38`,
//! `post_body.rs:15/:19`.
//!
//! Paths are printed relative to the invoked manifest's directory, so a
//! `--manifest-path xtask/Cargo.toml` run prints `src/steps/nix.rs`, not
//! `xtask/src/steps/nix.rs`. Callers hold both spellings; see `check::ScannedFile`.
//!
//! # What this cannot read, stated rather than papered over
//!
//! The line format is `<file> - <item path> (line N)`, and the file is taken as
//! everything before the first ` - `. A source path containing a literal ` - `
//! (space, hyphen, space) would therefore be truncated, and the entry keyed
//! against a path no scan produces — surfacing as a spurious `Orphan` plus a
//! spurious `NotRun`, not as a silent pass. No such path exists in this tree, and
//! `test-support` is **not** an instance: a bare hyphen never collides with the
//! spaced separator.
//!
//! Recorded because the first version of this parser used `rfind` to "defend"
//! against exactly that case, with a test that appeared to prove it. The test was
//! vacuous — `find` and `rfind` agree on every real line — which a mutation check
//! caught. The boundary is inherited here rather than rediscovered.

/// One doctest as the runner reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEntry {
    /// Path as printed — relative to the invoked manifest's directory.
    pub file: String,
    /// The `(line N)` the runner printed: the fence's opening line.
    pub line: usize,
    pub ignored: bool,
    pub failed: bool,
}

/// The `(line N)` marker's byte range and parsed value, taken from the LAST such
/// marker in `head` — a path may itself contain parentheses.
fn last_line_marker(head: &str) -> Option<(usize, usize)> {
    let open = head.rfind("(line ")?;
    let rest = &head[open + "(line ".len()..];
    let close = rest.find(')')?;
    let n = rest[..close].trim().parse().ok()?;
    Some((open, n))
}

/// Every doctest result line in `output`; everything else is skipped.
///
/// The file is everything before the ` - ` that separates it from the item path.
pub fn run_entries(output: &str) -> Vec<RunEntry> {
    let mut out = Vec::new();
    for raw in output.lines() {
        let Some(rest) = raw.trim_start().strip_prefix("test ") else {
            continue;
        };
        let Some(sep) = rest.rfind(" ... ") else {
            continue;
        };
        let (head, outcome) = (&rest[..sep], rest[sep + " ... ".len()..].trim());
        let Some((marker_at, line)) = last_line_marker(head) else {
            continue;
        };
        let before = head[..marker_at].trim_end();
        // ` - ` separates the file from the item path; a module-doc entry has an
        // empty item path, leaving `file -` and a trailing dash to strip.
        let file = match before.find(" - ") {
            // `find`, not `rfind`: the separator is the FIRST ` - `. An item path
            // cannot contain one (Rust identifiers have no spaces), so the first
            // is the only one. See the module doc for the class this cannot read.
            Some(i) => &before[..i],
            None => before.trim_end_matches('-').trim_end(),
        };
        out.push(RunEntry {
            file: file.trim().to_string(),
            line,
            ignored: outcome.starts_with("ignored"),
            failed: outcome.starts_with("FAILED"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `cargo test --workspace --doc` on this tree, 2026-08-01.
    const REAL: &str = "\
running 3 tests
test common/src/token.rs - token::RawToken (line 56) - compile fail ... ok
test common/src/etag.rs - etag::ETag (line 35) - compile fail ... ok
test common/src/post_body.rs - post_body::PostBody (line 15) - compile fail ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.31s
";

    #[test]
    fn parses_file_and_line_from_real_output() {
        let e = run_entries(REAL);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].file, "common/src/token.rs");
        assert_eq!(e[0].line, 56);
        assert!(!e[0].ignored && !e[0].failed);
    }

    #[test]
    fn parses_a_module_doc_entry_with_no_item_path() {
        let out = "test src/lib.rs - (line 4) - compile fail ... ok\n";
        let e = run_entries(out);
        assert_eq!(e[0].file, "src/lib.rs");
        assert_eq!(e[0].line, 4);
    }

    #[test]
    fn records_ignored_and_failed_separately() {
        let out = "\
test a.rs - a::A (line 3) ... ignored
test b.rs - b::B (line 7) ... FAILED
";
        let e = run_entries(out);
        assert!(e[0].ignored && !e[0].failed);
        assert!(e[1].failed && !e[1].ignored);
    }

    #[test]
    fn ignores_summary_and_noise_lines() {
        assert!(run_entries("running 3 tests\ntest result: ok. 3 passed;\n\n").is_empty());
    }

    #[test]
    fn a_hyphenated_crate_directory_survives() {
        // `test-support` is a real scan root. A bare hyphen is NOT the spaced
        // ` - ` separator, so it must not influence the split at all.
        let out = "test test-support/src/x.rs - x::Y (line 9) ... ok\n";
        assert_eq!(run_entries(out)[0].file, "test-support/src/x.rs");
    }

    #[test]
    fn a_module_doc_entry_in_a_hyphenated_crate_keeps_its_directory() {
        // Both shapes at once: no item path AND a hyphen in the path.
        let out = "test test-support/src/lib.rs - (line 12) ... ok\n";
        let e = run_entries(out);
        assert_eq!(e[0].file, "test-support/src/lib.rs");
        assert_eq!(e[0].line, 12);
    }

    #[test]
    fn a_trailing_marker_after_the_line_number_is_not_part_of_the_file() {
        // Real `compile_fail` lines carry a ` - compile fail` suffix AFTER the
        // `(line N)`, so the file must be read from before the marker, not from
        // the last separator on the line.
        let out = "test common/src/token.rs - token::RawToken (line 56) - compile fail ... ok\n";
        let e = run_entries(out);
        assert_eq!(e[0].file, "common/src/token.rs");
        assert_eq!(e[0].line, 56);
    }
}
