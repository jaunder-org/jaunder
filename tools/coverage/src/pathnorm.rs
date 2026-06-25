//! Path normalization for the `cargo llvm-cov report --text` output: rewrite the
//! absolute Nix-sandbox `.rs:` file-header lines to repo-relative ones. Ports
//! the bash `normalize_report_paths` from the retired `scripts/check-coverage`.

/// Strip a leading `"{abs_root}/"` prefix from every file-header line (one
/// ending in `.rs:`). Non-header lines are passed through verbatim. Idempotent.
pub fn normalize_report_text(report: &str, abs_root: &str) -> String {
    let prefix = format!("{abs_root}/");
    let mut out = String::with_capacity(report.len());
    for line in report.split_inclusive('\n') {
        let (content, nl) = match line.strip_suffix('\n') {
            Some(c) => (c, "\n"),
            None => (line, ""),
        };
        if content.ends_with(".rs:") {
            out.push_str(content.strip_prefix(&prefix).unwrap_or(content));
        } else {
            out.push_str(content);
        }
        out.push_str(nl);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_abs_prefix_from_header_lines_only() {
        let root = "/build/source";
        let input = "\
/build/source/server/src/x.rs:
    1|    5|fn f() {}
/build/source/web/src/y.rs:
    2|    0|let z = 1;
";
        let got = normalize_report_text(input, root);
        assert_eq!(
            got,
            "\
server/src/x.rs:
    1|    5|fn f() {}
web/src/y.rs:
    2|    0|let z = 1;
"
        );
    }

    #[test]
    fn is_idempotent_on_relative_paths() {
        let root = "/build/source";
        let input = "server/src/x.rs:\n    1|    5|fn f() {}\n";
        assert_eq!(normalize_report_text(input, root), input);
    }
}
