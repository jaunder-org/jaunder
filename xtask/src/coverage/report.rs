use crate::coverage::{FileCoverage, LineCov};

/// Parse `cargo llvm-cov report --text` output. A line is executable iff its
/// second pipe-delimited column is non-blank; covered iff that column is a
/// non-zero count (counts may carry a k/M suffix). File headers end in `.rs:`.
/// A line whose source text contains `// cov:ignore` is treated as
/// non-executable (our explicit exclusion escape-hatch) and omitted.
pub fn parse_text_report(report: &str, repo_root: &str) -> Vec<FileCoverage> {
    let prefix = format!("{}/", repo_root.trim_end_matches('/'));
    let mut files: Vec<FileCoverage> = Vec::new();
    for line in report.lines() {
        if line.ends_with(".rs:") {
            let path = line.strip_suffix(':').unwrap_or(line);
            let rel = path.strip_prefix(&prefix).unwrap_or(path).to_string();
            files.push(FileCoverage {
                path: rel,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        // Format: "<lineno>|<count>|<source...>". Split into at most 3.
        let mut parts = line.splitn(3, '|');
        let (Some(num_col), Some(count_col)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(lineno) = num_col.trim().parse::<u32>() else {
            continue;
        };
        let count = count_col.trim();
        if count.is_empty() {
            continue; // non-executable
        }
        let covered = !is_zero_count(count);
        let text = parts.next().unwrap_or("").to_string();
        if text.contains("// cov:ignore") {
            continue; // explicit exclusion marker — drop from the executable set
        }
        file.lines.push(LineCov {
            line: lineno,
            covered,
            text,
        });
    }
    files
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
    5|     0|    impossible() // cov:ignore
";

    #[test]
    fn parses_executable_lines_with_covered_flag_and_text() {
        let files = parse_text_report(SAMPLE, "/repo");
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "server/src/x.rs");
        // line 1 non-executable (blank count) → omitted; line 5 has `// cov:ignore` → omitted.
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
}
