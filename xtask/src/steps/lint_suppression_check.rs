//! Fail-closed inventory of approved Rust lint expectations (#294).
//!
//! This gate structurally enumerates Rust `#[allow(...)]` and `#[expect(...)]`
//! attributes — including lint attributes nested in `cfg_attr` — under the
//! first-party source, test, and build-script paths listed in [`POLICED_ROOTS`] and
//! [`POLICED_FILES`]. Any `#[allow]` is rejected outright. A `#[expect]` is
//! accepted only when the immediately preceding source line carries a non-empty
//! `// lint-suppression:allow <reason>` marker, so approval lives at the site being
//! exempted and the expectation self-removes when clippy no longer sees the lint.
//!
//! The gate fails closed on unreadable roots, unreadable files, Rust parse errors,
//! bare markers, orphan markers, and marker lines that point at multiple lint
//! attributes. It does not inspect generated code outside the policed paths,
//! non-Rust files, third-party dependency source, command-line lint flags, or Rust
//! lint configuration that is not expressed as a source attribute.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::spanned::Spanned;

use crate::markers::{line_comments, marker_in_comment};
use crate::result::{CommandResult, StepResult};

const POLICED_ROOTS: &[&str] = &[
    "client/src",
    "common/src",
    "csr/src",
    "host/src",
    "macros/src",
    "macros/tests",
    "server/src",
    "server/tests",
    "storage/src",
    "test-support/src",
    "test-support/tests",
    "web/src",
    "tools/coverage/src",
    "tools/devtool/src",
    "tools/devtool/tests",
    "tools/doctests/src",
    "xtask/src",
];
const POLICED_FILES: &[&str] = &["server/build.rs"];
const STEP: &str = "lint-suppression";
const RULE: &str =
    "new lint expectations require explicit user approval in a source-site lint-suppression marker";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Site {
    path: String,
    line: u32,
    kind: Kind,
    tokens: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Kind {
    Allow,
    Expect,
}

struct Collected {
    sites: Vec<Site>,
    line_counts: BTreeMap<u32, usize>,
    errors: Vec<String>,
}

struct Visitor<'a> {
    path: &'a str,
    sites: &'a mut Vec<Site>,
    errors: &'a mut Vec<String>,
}
impl Visitor<'_> {
    fn record_meta(&mut self, line: u32, meta: &syn::Meta) {
        let kind = if meta.path().is_ident("allow") {
            Some(Kind::Allow)
        } else if meta.path().is_ident("expect") {
            Some(Kind::Expect)
        } else {
            None
        };
        let syn::Meta::List(list) = meta else {
            return;
        };
        if let Some(kind) = kind {
            self.sites.push(Site {
                path: self.path.into(),
                line,
                kind,
                tokens: list.tokens.to_string(),
            });
        } else if list.path.is_ident("cfg_attr") {
            match list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) {
                Ok(args) => {
                    for meta in args {
                        self.record_meta(line, &meta);
                    }
                }
                Err(e) => self.errors.push(format!(
                    "{}:{line}: cannot parse cfg_attr lint arguments — {e}",
                    self.path
                )),
            }
        }
    }
}
impl<'ast> syn::visit::Visit<'ast> for Visitor<'_> {
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        let line = attr.span().start().line as u32;
        self.record_meta(line, &attr.meta);

        syn::visit::visit_attribute(self, attr);
    }
}

fn collect(path: &str, source: &str, marker: &str) -> Result<Collected, String> {
    let file = syn::parse_file(source).map_err(|e| format!("{path}: cannot parse — {e}"))?;
    let mut sites = Vec::new();
    let mut errors = Vec::new();
    syn::visit::visit_file(
        &mut Visitor {
            path,
            sites: &mut sites,
            errors: &mut errors,
        },
        &file,
    );

    let mut by_line = BTreeMap::new();
    for site in &sites {
        *by_line.entry(site.line).or_insert(0usize) += 1;
    }

    let comments = line_comments(source);

    for (idx, comment) in comments.iter().enumerate() {
        let Some(reason) = comment.and_then(|comment| marker_in_comment(comment, marker)) else {
            continue;
        };
        let marker_line = idx as u32 + 1;
        match by_line.get(&(marker_line + 1)).copied().unwrap_or_default() {
            0 => errors.push(format!(
                "{path}:{marker_line}: {marker} marker is orphaned; {RULE}"
            )),
            1 if reason.is_empty() => errors.push(format!(
                "{path}:{marker_line}: {marker} marker needs a reason; {RULE}"
            )),
            1 => {}
            _ => errors.push(format!(
                "{path}:{marker_line}: {marker} marker points at multiple lint attributes; split the line; {RULE}"
            )),
        }
    }

    Ok(Collected {
        sites,
        line_counts: by_line,
        errors,
    })
}

fn approved_marker<'a>(
    site: &Site,
    source: &'a str,
    line_counts: &BTreeMap<u32, usize>,
    marker: &str,
) -> Option<&'a str> {
    let marker_line = site.line.checked_sub(2)? as usize;
    if line_counts.get(&site.line).copied() != Some(1) {
        return None;
    }
    let comments = line_comments(source);
    let reason = comments
        .get(marker_line)
        .and_then(|comment| comment.and_then(|comment| marker_in_comment(comment, marker)))?;
    (!reason.is_empty()).then_some(reason)
}

fn problems_against(scanned: &[(String, String)]) -> Option<String> {
    let mut found = BTreeSet::new();
    let mut sources = BTreeMap::new();
    let mut lines = Vec::new();
    let marker = format!("{STEP}:allow");
    for (path, source) in scanned {
        match collect(path, source, &marker) {
            Ok(collected) => {
                found.extend(collected.sites);
                lines.extend(collected.errors);
                sources.insert(path.as_str(), (source.as_str(), collected.line_counts));
            }
            Err(e) => lines.push(e),
        }
    }
    for site in &found {
        if site.kind == Kind::Allow {
            lines.push(format!(
                "{}:{}: #[allow({})] is forbidden; {RULE}",
                site.path, site.line, site.tokens
            ));
        } else if sources
            .get(site.path.as_str())
            .and_then(|(source, line_counts)| approved_marker(site, source, line_counts, &marker))
            .is_none()
        {
            lines.push(format!(
                "{}:{}: unapproved #[expect({})]; {RULE}",
                site.path, site.line, site.tokens
            ));
        }
    }
    if !lines.is_empty() {
        let mut census = Vec::new();
        for site in &found {
            if site.kind == Kind::Expect
                && let Some(reason) =
                    sources
                        .get(site.path.as_str())
                        .and_then(|(source, line_counts)| {
                            approved_marker(site, source, line_counts, &marker)
                        })
            {
                census.push(format!(
                    "{}:{}: #[expect({})] — {reason}",
                    site.path, site.line, site.tokens
                ));
            }
        }
        if !census.is_empty() {
            lines.push(format!(
                "approved lint expectation census:\n{}",
                census.join("\n")
            ));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    problems_against(scanned)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    for root in POLICED_ROOTS {
        if let Err(e) = rust_files(Path::new(root), &mut files) {
            result.push(StepResult::fail(STEP).detail(format!("cannot scan {root}: {e}")));
            return;
        }
    }
    for file in POLICED_FILES {
        let path = Path::new(file);
        if !path.is_file() {
            result.push(
                StepResult::fail(STEP).detail(format!("cannot scan {file}: not a regular file")),
            );
            return;
        }
        files.push(path.into());
    }
    files.sort();
    let mut scanned = Vec::new();
    let mut errors = Vec::new();
    for path in files {
        match std::fs::read_to_string(&path) {
            Ok(source) => scanned.push((path.display().to_string(), source)),
            Err(e) => errors.push(format!("{}: cannot read — {e}", path.display())),
        }
    }
    if let Some(detail) = problems(&scanned) {
        errors.push(detail);
    }
    result.push(if errors.is_empty() {
        StepResult::ok(STEP)
    } else {
        StepResult::fail(STEP).detail(errors.join("\n"))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_expect_marker_is_clean() {
        let scanned = vec![(
            "a.rs".into(),
            r#"// lint-suppression:allow approved test expectation
#![expect(clippy::expect_used)]
"#
            .into(),
        )];
        assert_eq!(problems_against(&scanned), None);
    }

    #[test]
    fn approved_cfg_attr_expect_marker_is_clean() {
        let scanned = vec![(
            "a.rs".into(),
            r#"// lint-suppression:allow approved conditional expectation
#![cfg_attr(test, expect(clippy::ref_option_ref))]
"#
            .into(),
        )];
        assert_eq!(problems_against(&scanned), None);
    }

    #[test]
    fn rejects_allow_parse_orphan_bare_and_multiple_markers() {
        let scanned = vec![
            ("direct.rs".into(), "#![allow(dead_code)]\n".into()),
            (
                "a.rs".into(),
                "#![cfg_attr(test, cfg_attr(feature = \"x\", allow(dead_code)))]\n".into(),
            ),
            ("bad.rs".into(), "fn {".into()),
            (
                "markers.rs".into(),
                r#"// lint-suppression:allow valid census entry
#![expect(clippy::expect_used)]
// lint-suppression:allow
#![expect(dead_code)]
// lint-suppression:allow orphan
fn f() {}
// lint-suppression:allow two sites
#[expect(dead_code)] #[expect(dead_code)] fn g() {}
"#
                .into(),
            ),
        ];
        let detail = problems_against(&scanned).unwrap();
        assert!(detail.contains("#[allow(dead_code)] is forbidden"));
        assert!(detail.contains("cannot parse"));
        assert!(detail.contains("marker needs a reason"));
        assert!(detail.contains("marker is orphaned"));
        assert!(detail.contains("marker points at multiple lint attributes"));
        assert!(detail.contains("approved lint expectation census"));
        assert!(detail.contains("clippy :: expect_used"));
        assert!(!detail.contains("#[expect(dead_code)] — two sites"));
    }

    #[test]
    fn unapproved_expect_names_site_and_approval_protocol() {
        let scanned = vec![(
            "new.rs".into(),
            "#![expect(clippy::too_many_lines)]\n".into(),
        )];
        let detail = problems_against(&scanned).expect("unapproved expect fails");
        assert!(detail.contains("new.rs:1"));
        assert!(detail.contains("explicit user approval"));
        assert!(detail.contains("lint-suppression marker"));
    }

    #[test]
    fn ignores_comments_strings_and_other_attributes() {
        let scanned = vec![(
            "a.rs".into(),
            r##"// #[expect(dead_code)]
const S: &str = "#[allow(dead_code)]";
#[derive(Debug)] struct S;"##
                .into(),
        )];
        assert_eq!(problems_against(&scanned), None);
    }
}

#[cfg(test)]
#[test]
fn root_walk_is_recursive_and_missing_root_fails() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("fixture.rs"), "").unwrap();
    let mut files = Vec::new();
    rust_files(temp.path(), &mut files).unwrap();
    assert_eq!(files, vec![nested.join("fixture.rs")]);
    assert!(rust_files(&temp.path().join("missing"), &mut files).is_err());
}

#[cfg(test)]
#[test]
fn approved_inventory_matches_all_policed_source_files() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for path in POLICED_ROOTS {
        rust_files(&root.join(path), &mut files).unwrap();
    }
    for path in POLICED_FILES {
        files.push(root.join(path));
    }
    let scanned = files
        .into_iter()
        .map(|path| {
            (
                path.strip_prefix(root).unwrap().display().to_string(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(problems(&scanned), None);
}

#[cfg(test)]
#[test]
fn standalone_build_script_is_policed() {
    assert_eq!(POLICED_FILES, ["server/build.rs"]);
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(POLICED_FILES[0])
            .is_file()
    );
}
