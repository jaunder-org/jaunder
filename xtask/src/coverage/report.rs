use crate::coverage::{FileCoverage, LineCov};
use crate::markers::{comment_marker_is, line_comment, marker_in_comment};
use anyhow::{Result, bail};

/// Parse `cargo llvm-cov report --text` output. A line is executable iff its
/// second pipe-delimited column is non-blank; covered iff that column is a
/// non-zero count (counts may carry a k/M suffix). File headers end in `.rs:`.
///
/// Explicit exclusion markers, recognized ONLY inside a real trailing `//`
/// comment (never inside a string/char literal), drop lines from the executable
/// set:
/// - line form `// cov:ignore: <specific reason>` drops that line;
/// - block form `// cov:ignore-start: <specific reason>` …
///   `// cov:ignore-stop` drops every line between the markers (and the marker
///   lines themselves).
///
/// Reasons are mandatory and non-empty. Legacy bare line/start forms, empty
/// reasons, and reason-bearing stops are hard errors. Nested, unmatched, and
/// stray block markers are likewise hard errors.
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
        let mut line_ignored = false;
        if let Some(c) = comment {
            if let Some(reason) = marker_in_comment(c, "cov:ignore-start:") {
                if reason.is_empty() {
                    bail!("cov:ignore-start at line {lineno} requires a non-empty reason");
                }
                if let Some(start) = block_start {
                    bail!(
                        "nested cov:ignore-start at line {lineno} (a block is \
                         already open from line {start})"
                    );
                }
                block_start = Some(lineno);
                continue; // the marker line itself is dropped
            }
            if comment_marker_is(c, "cov:ignore-start") {
                bail!(
                    "legacy cov:ignore-start at line {lineno}; use \
                     cov:ignore-start: <specific reason>"
                );
            }
            if let Some(suffix) = c.trim_start().strip_prefix("cov:ignore-stop")
                && (suffix.is_empty()
                    || suffix.starts_with(char::is_whitespace)
                    || suffix.starts_with(':'))
            {
                if !suffix.is_empty() {
                    bail!("noncanonical cov:ignore-stop at line {lineno}; it takes no reason");
                }
                if block_start.is_none() {
                    bail!("cov:ignore-stop at line {lineno} with no open cov:ignore-start");
                }
                block_start = None;
                continue; // the marker line itself is dropped
            }
            if let Some(reason) = marker_in_comment(c, "cov:ignore:") {
                if reason.is_empty() {
                    bail!("cov:ignore at line {lineno} requires a non-empty reason");
                }
                line_ignored = true;
            } else if comment_marker_is(c, "cov:ignore") {
                bail!("legacy cov:ignore at line {lineno}; use cov:ignore: <specific reason>");
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
        if line_ignored {
            continue; // line-form exclusion marker — drop from the executable set
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
/repo/server/src/x.rs:
    1|      |use std::foo;
    2|    36|pub fn bar() {
    3|     0|    fail()
    4|  1.36k|    ok()
    5|     0|    impossible() // cov:ignore: platform-only signal handler
";

    #[test]
    fn parses_executable_lines_with_covered_flag_and_text() {
        let files = parse_text_report(SAMPLE, "/repo").unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "server/src/x.rs");
        // line 1 non-executable (blank count) → omitted; line 5 has a real
        // trailing reason-bearing `// cov:ignore: …` comment → omitted.
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
    fn reason_bearing_line_marker_is_accepted_only_as_a_real_comment() {
        let report = "\
/repo/a.rs:
    1|     0|    boom() // cov:ignore: unavailable in host coverage
    2|     0|    kept()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        // Only the line with the genuine trailing comment is dropped.
        assert_eq!(lines, vec![2]);
    }

    #[test]
    fn reason_bearing_marker_in_a_string_literal_does_not_suppress() {
        let report = "\
/repo/a.rs:
    1|     0|    let marker = \"// cov:ignore: quoted prose\";
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1]);
    }

    #[test]
    fn reason_bearing_block_drops_interior_lines() {
        let report = "\
/repo/a.rs:
    1|    10|    before()
    2|      |    // cov:ignore-start: generated parser state
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
    fn legacy_and_empty_line_and_start_markers_are_errors() {
        for (marker, expected) in [
            ("cov:ignore", "legacy cov:ignore"),
            ("cov:ignore:   ", "requires a non-empty reason"),
            ("cov:ignore-start", "legacy cov:ignore-start"),
            ("cov:ignore-start:   ", "requires a non-empty reason"),
        ] {
            let report = format!("/repo/a.rs:\n    1|     0|    code() // {marker}\n");
            let err = parse_text_report(&report, "/repo").unwrap_err();
            assert!(err.to_string().contains(expected), "{err}");
        }
    }

    #[test]
    fn noncanonical_block_stop_is_an_error() {
        let report = "\
/repo/a.rs:
    1|      |    // cov:ignore-start: compiler bookkeeping
    2|      |    // cov:ignore-stop: no longer needed
";
        let err = parse_text_report(report, "/repo").unwrap_err();
        assert!(err.to_string().contains("noncanonical"), "{err}");
    }

    #[test]
    fn unmatched_block_start_is_error() {
        let report = "\
/repo/a.rs:
    1|      |    // cov:ignore-start: generated source
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
    1|      |    // cov:ignore-start: generated source
    2|     0|    inner()
    3|      |    // cov:ignore-start: compiler bookkeeping
    4|      |    // cov:ignore-stop
";
        let err = parse_text_report(report, "/repo").unwrap_err();
        assert!(err.to_string().contains("nested"), "{err}");
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
        // A `/// cov:ignore-start: …` inside a doc comment must NOT open a block:
        // the following executable line is still measured, and there is no
        // spurious unmatched-`-start` error at EOF.
        let report = "\
/repo/a.rs:
    1|      |    /// cov:ignore-start: documentation is inert
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
    2|     0|    boom() // cov:ignore: host cannot exercise this span
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![1]); // line 2 dropped (anchored marker), line 1 kept
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
    fn block_stop_is_anchored_not_incidental() {
        // Inside an OPEN block, an incidental mention of cov:ignore-stop must NOT close
        // it — only a real anchored `// cov:ignore-stop` does (the -stop side of AC2).
        // Under the old bare-`contains`, line 2 would spuriously close the block, then
        // line 4's real `-stop` would `bail!` (no open block).
        let report = "\
/repo/a.rs:
    1|      |    // cov:ignore-start: generated source
    2|     0|    dropped() // mentions cov:ignore-stop but not as a marker
    3|     0|    still_dropped()
    4|      |    // cov:ignore-stop
    5|     0|    measured()
";
        let files = parse_text_report(report, "/repo").unwrap();
        let lines: Vec<u32> = files[0].lines.iter().map(|l| l.line).collect();
        assert_eq!(lines, vec![5]); // block stayed open past the incidental mention
    }
}
