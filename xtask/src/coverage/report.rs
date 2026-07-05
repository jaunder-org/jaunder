use crate::coverage::{FileCoverage, LineCov};
use anyhow::{bail, Result};

/// Parse `cargo llvm-cov report --text` output. A line is executable iff its
/// second pipe-delimited column is non-blank; covered iff that column is a
/// non-zero count (counts may carry a k/M suffix). File headers end in `.rs:`.
///
/// Explicit exclusion markers, recognized ONLY inside a real trailing `//`
/// comment (never inside a string/char literal), drop lines from the executable
/// set:
/// - line form `// cov:ignore` on an executable line drops that line;
/// - block form `// cov:ignore-start` … `// cov:ignore-stop` drops every line
///   between the markers (and the marker lines themselves).
///
/// Unbalanced block markers are a hard error: a nested `-start`, an unmatched
/// `-start` at EOF, or a `-stop` with no open `-start` all fail loudly.
pub fn parse_text_report(report: &str, repo_root: &str) -> Result<Vec<FileCoverage>> {
    let prefix = format!("{}/", repo_root.trim_end_matches('/'));
    let mut files: Vec<FileCoverage> = Vec::new();
    // Block-exclusion state: `Some(lineno)` while inside a `-start`/`-stop` pair,
    // carrying the start line for diagnostics.
    let mut block_start: Option<u32> = None;
    for line in report.lines() {
        if line.ends_with(".rs:") {
            if let Some(start) = block_start {
                bail!(
                    "cov:ignore-start at line {start} was never closed before the \
                     next file header ({line})"
                );
            }
            let path = line.strip_suffix(':').unwrap_or(line);
            let rel = path.strip_prefix(&prefix).unwrap_or(path).to_string();
            files.push(FileCoverage {
                path: rel,
                lines: Vec::new(),
            });
            continue;
        }
        // Format: "<lineno>|<count>|<source...>". Split into at most 3.
        let mut parts = line.splitn(3, '|');
        let (Some(num_col), Some(count_col)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(lineno) = num_col.trim().parse::<u32>() else {
            continue;
        };
        let text = parts.next().unwrap_or("");

        // Marker detection runs on EVERY report line (executable or not) so a
        // marker sitting on a non-executable comment line is still honored, and
        // is matched only against the line's real trailing comment.
        let comment = line_comment(text);
        if let Some(c) = comment {
            if comment_marker_is(c, "cov:ignore-start") {
                if let Some(start) = block_start {
                    bail!(
                        "nested cov:ignore-start at line {lineno} (a block is \
                         already open from line {start})"
                    );
                }
                block_start = Some(lineno);
                continue; // the marker line itself is dropped
            }
            if comment_marker_is(c, "cov:ignore-stop") {
                if block_start.is_none() {
                    bail!("cov:ignore-stop at line {lineno} with no open cov:ignore-start");
                }
                block_start = None;
                continue; // the marker line itself is dropped
            }
        }

        // Inside an open block → drop the line regardless of executability.
        if block_start.is_some() {
            continue;
        }

        let Some(file) = files.last_mut() else {
            continue;
        };
        let count = count_col.trim();
        if count.is_empty() {
            continue; // non-executable
        }
        if let Some(c) = comment {
            if comment_marker_is(c, "cov:ignore") {
                continue; // line-form exclusion marker — drop from the executable set
            }
        }
        let covered = !is_zero_count(count);
        file.lines.push(LineCov {
            line: lineno,
            covered,
            text: text.to_string(),
        });
    }
    if let Some(start) = block_start {
        bail!("cov:ignore-start at line {start} was never closed (unmatched at EOF)");
    }
    Ok(files)
}

/// A count column is "zero" only if it is literally 0 (covered iff non-zero).
fn is_zero_count(count: &str) -> bool {
    count == "0"
}

/// True iff `marker` is the first whitespace-delimited token of `comment` (the text
/// after `//`, as returned by [`line_comment`]). Anchoring marker recognition to the
/// first token keeps an incidental mention in prose (`// unlike the cov:ignore path`)
/// inert, while still honoring `// cov:ignore` and `// cov:ignore <trailing note>`
/// (#246).
fn comment_marker_is(comment: &str, marker: &str) -> bool {
    comment.split_whitespace().next() == Some(marker)
}

/// Return the text of the first real trailing line comment in `src` — the slice
/// after the first `//` that begins OUTSIDE a `"`-string or `'`-char literal.
/// Returns `None` when there is no such comment (so a `//` inside a string or a
/// doc/string-embedded marker never counts). A **doc comment** — outer `///` or
/// inner `//!` — is deliberately NOT treated as a marker-bearing comment: it
/// can never open/close a `cov:ignore` block or suppress a line, so a marker
/// mentioned in prose documentation is inert. Escapes (`\"`, `\'`) are honored;
/// a `'` that does not open a well-formed char literal (a lifetime tick) is
/// treated as ordinary text. Raw strings are not specially handled — rare in
/// report lines, and a best-effort scan is sufficient here.
fn line_comment(src: &str) -> Option<&str> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            match b {
                b'\\' => i += 1, // skip the escaped character
                b'"' => in_str = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // `///` (outer doc) or `//!` (inner doc) is a doc comment, not a
                // marker-bearing comment — the rest of the line is documentation
                // prose and can never suppress coverage.
                if matches!(bytes.get(i + 2), Some(&b'/') | Some(&b'!')) {
                    return None;
                }
                return Some(&src[i + 2..]);
            }
            b'"' => in_str = true,
            b'\'' => {
                if let Some(len) = char_literal_len(&bytes[i..]) {
                    i += len; // jump past the whole char literal
                    continue;
                }
                // otherwise a lifetime tick — fall through, treat as ordinary text
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// If `bytes` (which starts at a `'`) opens a well-formed char literal, return
/// its length in bytes including both quotes; otherwise `None` (e.g. a lifetime
/// tick like `'a`). Best-effort — handles simple and escaped char literals.
fn char_literal_len(bytes: &[u8]) -> Option<usize> {
    debug_assert_eq!(bytes.first(), Some(&b'\''));
    if bytes.len() < 3 {
        return None;
    }
    if bytes[1] == b'\\' {
        // Escaped: '\n', '\'', '\\', '\0', '\u{1F600}', '\x41' … the byte right
        // after the backslash is literal, so start scanning for the closer past it.
        let mut j = 3;
        while j < bytes.len() {
            if bytes[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        None
    } else {
        // Unescaped: one UTF-8 scalar (1..=4 bytes) then a closing quote. A
        // closing quote within the next few bytes marks a real char literal; its
        // absence (e.g. `'a` in `'a, 'b>`) means a lifetime.
        let end = bytes.len().min(6);
        let mut j = 2;
        while j < end {
            if bytes[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
/repo/server/src/x.rs:
    1|      |use std::foo;
    2|    36|pub fn bar() {
    3|     0|    fail()
    4|  1.36k|    ok()
    5|     0|    impossible() // cov:ignore
";

    #[test]
    fn parses_executable_lines_with_covered_flag_and_text() {
        let files = parse_text_report(SAMPLE, "/repo").unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "server/src/x.rs");
        // line 1 non-executable (blank count) → omitted; line 5 has a real
        // trailing `// cov:ignore` comment → omitted.
        assert_eq!(
            f.lines.iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert!(f.lines[0].covered); // 36
        assert!(!f.lines[1].covered); // 0
        assert!(f.lines[2].covered); // 1.36k (non-zero)
        assert_eq!(f.lines[0].text, "pub fn bar() {");
        assert!(!f.lines.iter().any(|l| l.line == 5)); // excluded by marker
    }

    #[test]
    fn line_marker_ignored_only_as_real_comment() {
        let report = "\
/repo/a.rs:
    1|     0|    boom() // cov:ignore
    2|     0|    kept()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        // Only the line with the genuine trailing comment is dropped.
        assert_eq!(lines, vec![2]);
    }

    #[test]
    fn marker_in_string_literal_does_not_suppress() {
        // The marker text lives inside a string literal, not a real comment —
        // it must NOT drop the line (inverts the old bare-`contains` behavior).
        let report = "\
/repo/a.rs:
    1|     0|    let s = \"// cov:ignore\";
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1]);
    }

    #[test]
    fn block_drops_interior_lines() {
        let report = "\
/repo/a.rs:
    1|    10|    before()
    2|      |    // cov:ignore-start
    3|     0|    skipped_one()
    4|     0|    skipped_two()
    5|      |    // cov:ignore-stop
    6|    10|    after()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        // Interior lines (3, 4) and both marker lines are dropped; boundaries
        // outside the block (1, 6) survive.
        assert_eq!(lines, vec![1, 6]);
    }

    #[test]
    fn unmatched_block_start_is_error() {
        let report = "\
/repo/a.rs:
    1|      |    // cov:ignore-start
    2|     0|    never_closed()
";
        let err = parse_text_report(report, "/repo").unwrap_err();
        assert!(err.to_string().contains("never closed"), "{err}");
    }

    #[test]
    fn stray_block_stop_is_error() {
        let report = "\
/repo/a.rs:
    1|    10|    fine()
    2|      |    // cov:ignore-stop
";
        let err = parse_text_report(report, "/repo").unwrap_err();
        assert!(
            err.to_string().contains("no open cov:ignore-start"),
            "{err}"
        );
    }

    #[test]
    fn nested_block_is_error() {
        let report = "\
/repo/a.rs:
    1|      |    // cov:ignore-start
    2|     0|    inner()
    3|      |    // cov:ignore-start
    4|      |    // cov:ignore-stop
";
        let err = parse_text_report(report, "/repo").unwrap_err();
        assert!(err.to_string().contains("nested"), "{err}");
    }

    #[test]
    fn line_comment_ignores_markers_inside_strings_and_finds_real_comments() {
        assert_eq!(
            line_comment("    boom() // cov:ignore"),
            Some(" cov:ignore")
        );
        assert_eq!(line_comment("    let s = \"// cov:ignore\";"), None);
        assert_eq!(line_comment("    let c = '/';"), None);
        // A lifetime tick must not swallow a following real comment.
        assert_eq!(
            line_comment("    fn f<'a>() {} // cov:ignore"),
            Some(" cov:ignore")
        );
        // Doc comments (`///` outer, `//!` inner) are not marker-bearing
        // comments — a real `//` still is.
        assert_eq!(line_comment("/// cov:ignore-start"), None);
        assert_eq!(line_comment("//! cov:ignore"), None);
        assert_eq!(line_comment("    boom() /// cov:ignore"), None);
    }

    #[test]
    fn doc_comment_line_marker_does_not_suppress() {
        // A `cov:ignore` mentioned inside a doc comment must NOT drop the line:
        // doc comments document behavior, they don't suppress coverage.
        let report = "\
/repo/a.rs:
    1|     0|    kept() /// cov:ignore
    2|     0|    also_kept() //! cov:ignore
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1, 2]);
    }

    #[test]
    fn doc_comment_block_start_is_ignored() {
        // A `/// cov:ignore-start` inside a doc comment must NOT open a block:
        // the following executable line is still measured, and there is no
        // spurious unmatched-`-start` error at EOF.
        let report = "\
/repo/a.rs:
    1|      |    /// cov:ignore-start
    2|     0|    still_measured()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![2]);
    }

    #[test]
    fn incidental_mention_in_real_comment_is_kept() {
        // An executable line whose GENUINE trailing comment merely mentions the token
        // must NOT be dropped (the #246 footgun).
        let report = "\
/repo/a.rs:
    1|     0|    do_work() // unlike the cov:ignore path
    2|     0|    boom() // cov:ignore
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1]); // line 2 dropped (anchored marker), line 1 kept
    }

    #[test]
    fn line_marker_with_trailing_note_is_dropped() {
        let report = "\
/repo/a.rs:
    1|     0|    boom() // cov:ignore reason here
";
        let files = parse_text_report(report, "/repo").unwrap();
        assert!(files[0].lines.is_empty()); // first token is the marker → dropped
    }

    #[test]
    fn block_markers_are_anchored_not_incidental() {
        // A comment mentioning cov:ignore-start as non-first-token must NOT open a block.
        let report = "\
/repo/a.rs:
    1|     0|    keep() // see the cov:ignore-start docs
    2|     0|    also_keep()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1, 2]); // no block opened; both lines measured
    }

    #[test]
    fn comment_marker_is_matches_first_token_only() {
        assert!(comment_marker_is(" cov:ignore", "cov:ignore"));
        assert!(comment_marker_is(" cov:ignore trailing", "cov:ignore"));
        assert!(comment_marker_is("cov:ignore", "cov:ignore")); // no leading space
        assert!(!comment_marker_is(
            " unlike the cov:ignore path",
            "cov:ignore"
        ));
        assert!(comment_marker_is(" cov:ignore-start", "cov:ignore-start"));
        assert!(!comment_marker_is(" cov:ignore-start", "cov:ignore")); // distinct token
    }
}
