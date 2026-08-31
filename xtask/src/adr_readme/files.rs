use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;

pub const ADR_DIR: &str = "docs/adr";

/// The link target for an ADR as written from `docs/` — the form the README table
/// renders and the view cites. Spelled once so the three call sites cannot drift.
pub(super) fn adr_link(filename: &str) -> String {
    format!("adr/{filename}")
}

/// The status tokens legal on a **numbered** ADR (the canonical status cell is
/// exactly one of these).
///
/// `proposed` is absent by design: numbering is the acceptance event, so a numbered
/// ADR has been accepted. A *draft* may still say `proposed` — drafts are invisible
/// to this gate (numberless, and in a subdirectory `adr_files` never descends into),
/// and `adr promote` rewrites the token as it assigns the number.
///
/// There is deliberately no five-token constant alongside this one. Nothing ever
/// validated a draft in code, so the draft vocabulary lives where it is actually
/// consulted — `docs/adr/template.md` and the `jaunder-adr` skill. Keeping a wider
/// set here would also make the out-of-vocabulary message below advertise
/// `proposed` as legal while the rule above rejects it.
const NUMBERED_STATUS_VOCAB: [&str; 4] = ["accepted", "superseded", "deprecated", "rejected"];

/// An ADR file projected to its table-relevant fields. `title` is the heading
/// text with the `ADR-NNNN:` / `NNNN.` prefix stripped (used only to seed a new
/// row); `status` is the single status token.
pub struct AdrEntry {
    pub num: u32,
    pub filename: String,
    pub title: String,
    pub status: String,
}

/// A directory entry that qualifies as an ADR: a regular `*.md` file whose name
/// carries a leading number. Content is intentionally not read here so each
/// caller keeps its own IO-error policy.
pub(super) struct AdrFile {
    num: u32,
    filename: String,
    path: PathBuf,
}

/// The qualifying ADR files under `repo/docs/adr`, unsorted — the single home of
/// the "what counts as an ADR file" rule (`is_file` → `.md` → leading number).
/// The `read_dir` error is returned unwrapped so callers phrase their own context
/// (`parse_adr_dir` wants "reading <dir>", `format_problems` wants
/// "cannot read <dir>").
pub(super) fn adr_files(repo: &Path) -> Result<Vec<AdrFile>> {
    adr_files_from(
        std::fs::read_dir(repo.join(ADR_DIR))?,
        std::fs::DirEntry::path,
        std::fs::DirEntry::file_name,
        std::fs::DirEntry::file_type,
    )
}

pub(super) fn adr_files_from<T>(
    entries: impl IntoIterator<Item = std::io::Result<T>>,
    path_of: impl Fn(&T) -> PathBuf,
    file_name: impl Fn(&T) -> OsString,
    file_type: impl Fn(&T) -> std::io::Result<std::fs::FileType>,
) -> Result<Vec<AdrFile>> {
    entries
        .into_iter()
        .map(|entry| {
            let entry = entry?;
            qualify_adr_file(path_of(&entry), file_name(&entry), file_type(&entry))
        })
        .collect::<Result<Vec<_>>>()
        .map(|files| files.into_iter().flatten().collect())
}

fn qualify_adr_file(
    path: PathBuf,
    filename: OsString,
    file_type: std::io::Result<std::fs::FileType>,
) -> Result<Option<AdrFile>> {
    if !file_type
        .with_context(|| format!("reading file type {}", path.display()))?
        .is_file()
    {
        return Ok(None);
    }
    let filename = filename.to_string_lossy().into_owned();
    if !filename.ends_with(".md") {
        return Ok(None);
    }
    let Some(num) = ids::leading_number(&filename) else {
        return Ok(None);
    };
    Ok(Some(AdrFile {
        num,
        filename,
        path,
    }))
}

/// The title text of a `# ADR-NNNN: Title` (or legacy `# NNNN. Title`) heading,
/// prefix stripped. Falls back to the whole heading when it matches neither form.
fn heading_title(content: &str) -> String {
    let line = content.lines().find(|l| l.starts_with("# ")).unwrap_or("");
    let after = line.trim_start_matches("# ").trim();
    for (prefix, sep) in [("ADR-", ": "), ("", ". ")] {
        if let Some((lhs, title)) = after.split_once(sep)
            && lhs.starts_with(prefix)
            && !lhs.is_empty()
            && lhs[prefix.len()..].chars().all(|c| c.is_ascii_digit())
        {
            return title.trim().to_string();
        }
    }
    after.to_string()
}

/// The status token a draft carries until promotion numbers it, and the token
/// promotion writes in its place. Spelled once here because the gate, the table
/// projection and `adr promote`'s rewrite all test against them.
pub(crate) const PROPOSED: &str = "proposed";
pub(crate) const ACCEPTED: &str = "accepted";

/// An ADR's status line, located once for every consumer.
pub(crate) struct StatusLine<'a> {
    /// Byte span of the line within the document, excluding its terminator — so a
    /// rewriter can splice a replacement in without recomputing where the line
    /// starts, and the gate can quote the line verbatim.
    pub span: std::ops::Range<usize>,
    /// The trimmed remainder after the `- Status:` / `Status:` prefix, returned
    /// **whole** rather than pre-split: the gate must keep rejecting
    /// `- Status: accepted (superseded)` for carrying more than one token, and a
    /// helper that yielded only the first token would silently drop that rule. An
    /// empty remainder (`- Status:` with nothing after it) is `Some` with `rest`
    /// empty, so the gate reports "not a single token" rather than "missing".
    pub rest: &'a str,
    /// Whether the line is in canonical form — `- Status:` at column 0. Locating a
    /// non-canonical line is deliberate (so the rewrite and the gate always agree
    /// on *which* line is the status line, even a misindented one), but accepting
    /// it is not: `adr-format` requires the canonical spelling, exactly as it did
    /// before the parses were unified.
    pub canonical: bool,
}

/// Locate `content`'s status line: the first line whose `trim_start` begins
/// `- Status:` or a bare `Status:`.
///
/// The single home of "which line is the status line, and what does it say" — the
/// `adr-format` gate, the table projection, and `adr promote`'s status rewrite all
/// read it here, so they cannot disagree. They previously disagreed: this parse
/// tolerated indentation and the bare form, while the gate matched only a column-0
/// `- Status:`. A rewrite that picked a different line than the gate would emit a
/// promoted ADR that instantly fails `adr-format`.
///
/// Locating leniently and *judging* strictly is the point: agreement on the line is
/// a parsing concern, canonical spelling is a policy one, and [`StatusLine`] hands
/// each consumer the part it owns.
pub(crate) fn status_line(content: &str) -> Option<StatusLine<'_>> {
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        let bare = bare.strip_suffix('\r').unwrap_or(bare);
        let trimmed = bare.trim_start();
        if let Some(rest) = trimmed
            .strip_prefix("- Status:")
            .or_else(|| trimmed.strip_prefix("Status:"))
        {
            return Some(StatusLine {
                span: offset..offset + bare.len(),
                rest: rest.trim(),
                canonical: bare.starts_with("- Status:"),
            });
        }
        offset += line.len();
    }
    None
}

/// The single status token from the status line, or `""` when there is none.
fn status_token(content: &str) -> String {
    status_line(content)
        .and_then(|s| s.rest.split_whitespace().next())
        .unwrap_or("")
        .to_string()
}

/// Parse the ADR files under `repo/docs/adr`, sorted ascending by number.
pub fn parse_adr_dir(repo: &Path) -> Result<Vec<AdrEntry>> {
    parse_adr_files(repo, adr_files(repo))
}

pub(super) fn parse_adr_files(repo: &Path, files: Result<Vec<AdrFile>>) -> Result<Vec<AdrEntry>> {
    let dir = repo.join(ADR_DIR);
    let mut entries = Vec::new();
    for f in files.with_context(|| format!("reading {}", dir.display()))? {
        let content = std::fs::read_to_string(&f.path)
            .with_context(|| format!("reading {}", f.path.display()))?;
        entries.push(AdrEntry {
            num: f.num,
            filename: f.filename,
            title: heading_title(&content),
            status: status_token(&content),
        });
    }
    entries.sort_by_key(|e| e.num);
    Ok(entries)
}

/// The `adr-format` problems for one ADR file: the line-1 heading must be
/// `# ADR-NNNN: <nonempty>` with `NNNN` matching the filename number, and a
/// `- Status: <token>` line must exist with a single token from
/// [`NUMBERED_STATUS_VOCAB`] and nothing trailing. `filename`/`num` come from the
/// directory entry.
fn file_format_problems(filename: &str, num: u32, content: &str) -> Vec<String> {
    let mut problems = Vec::new();

    let line1 = content.lines().next().unwrap_or("");
    let prefix = format!("# ADR-{:04}: ", num);
    match line1.strip_prefix(&prefix) {
        Some(title) if !title.trim().is_empty() => {}
        Some(_) => problems.push(format!("{filename}: heading has an empty title")),
        None => problems.push(format!(
            "{filename}: heading must be `# ADR-{:04}: <title>` (found `{line1}`)",
            num
        )),
    }

    match status_line(content) {
        None => problems.push(format!("{filename}: missing a `- Status: <token>` line")),
        // A non-canonical line is located but not tolerated. Unifying the parse was
        // about the gate and the rewrite agreeing on *which* line; it must not
        // quietly widen what the gate accepts, which before the unification was a
        // column-0 `- Status:` and nothing else.
        Some(status) if !status.canonical => problems.push(format!(
            "{filename}: status line must be `- Status: <token>` at column 0 (found `{}`)",
            &content[status.span]
        )),
        Some(status) => {
            let rest = status.rest;
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() != 1 {
                problems.push(format!(
                    "{filename}: `- Status:` must be a single token with nothing trailing (found `{rest}`)"
                ));
            } else if tokens[0] == PROPOSED {
                // Special-cased ahead of the membership check so the diagnosis names
                // the fix rather than a list the reader has to reason about.
                problems.push(format!(
                    "{filename}: status is `proposed`, but numbering is the acceptance \
                     event — a decision still under consideration belongs in docs/adr/drafts/"
                ));
            } else if !NUMBERED_STATUS_VOCAB.contains(&tokens[0]) {
                problems.push(format!(
                    "{filename}: status `{}` is not one of {NUMBERED_STATUS_VOCAB:?}",
                    tokens[0]
                ));
            }
        }
    }
    problems
}

/// Every ADR file's `adr-format` problems, sorted for stable output. A directory
/// read error is surfaced as a single problem rather than a panic.
pub fn format_problems(repo: &Path) -> Vec<String> {
    let files = match adr_files(repo) {
        Ok(f) => f,
        Err(e) => return vec![format!("cannot read {}: {e}", repo.join(ADR_DIR).display())],
    };
    let mut problems = Vec::new();
    for f in files {
        match std::fs::read_to_string(&f.path) {
            Ok(content) => problems.extend(file_format_problems(&f.filename, f.num, &content)),
            Err(e) => problems.push(format!("{}: cannot read ({e})", f.filename)),
        }
    }
    problems.sort();
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway repo dir with `docs/adr/`, unique per (pid, tag) so parallel
    /// tests don't collide. Cleaned best-effort by the caller.
    fn scratch_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "jaunder-adr-readme-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs/adr")).unwrap();
        dir
    }

    #[test]
    fn heading_title_strips_canonical_and_legacy_prefixes() {
        assert_eq!(
            heading_title("# ADR-0021: SQLite discipline: avoid deferred txns\n"),
            "SQLite discipline: avoid deferred txns"
        );
        assert_eq!(
            heading_title("# 0030. Coverage re-anchor by text identity\n"),
            "Coverage re-anchor by text identity"
        );
    }

    #[test]
    fn status_line_spans_the_line_and_judges_canonicity() {
        // `(span-as-text, rest, canonical)` — asserting on the sliced span rather
        // than raw offsets keeps expectations readable and still pins the span.
        fn parsed(content: &str) -> Option<(&str, &str, bool)> {
            status_line(content).map(|s| (&content[s.span], s.rest, s.canonical))
        }

        // Indentation and the bare `Status:` form are THE discriminating cases:
        // the only two spellings where a column-0-only parse and a token parse
        // can disagree. Trailing whitespace is *not* discriminating — the gate
        // already trims — so an implementation that only handled it would pass a
        // weaker test than this.
        assert_eq!(
            parsed("# T\n\n- Status: accepted\n"),
            Some(("- Status: accepted", "accepted", true))
        );
        // Located, but NOT canonical: the rewrite still finds these, the gate still
        // rejects them. Separating the two judgements is the whole point.
        assert_eq!(
            parsed("# T\n\n  - Status: accepted\n"),
            Some(("  - Status: accepted", "accepted", false))
        );
        assert_eq!(
            parsed("# T\n\nStatus: superseded\n"),
            Some(("Status: superseded", "superseded", false))
        );
        // Remainder returned whole, so the gate can still count tokens.
        assert_eq!(
            parsed("# T\n\n- Status: a (b)\n"),
            Some(("- Status: a (b)", "a (b)", true))
        );
        // Empty remainder is Some(""), not None — "not a single token", not "missing".
        assert_eq!(parsed("# T\n\n- Status:\n"), Some(("- Status:", "", true)));
        // The span excludes the terminator, including a CRLF's `\r`.
        assert_eq!(
            parsed("# T\r\n\r\n- Status: accepted\r\n"),
            Some(("- Status: accepted", "accepted", true))
        );
        // Unterminated final line.
        assert_eq!(
            parsed("# T\n\n- Status: accepted"),
            Some(("- Status: accepted", "accepted", true))
        );
        assert_eq!(parsed("# T\n\nno status\n"), None);
    }

    #[test]
    fn file_format_problems_rejects_a_non_canonical_status_line() {
        // Unifying the parse must not widen what the gate accepts. Before the
        // unification an indented or bare-`Status:` line was reported (as a missing
        // status line); it must still be reported, or the refactor smuggled in a
        // loosening. Teeth: delete the `canonical` arm and both of these go green.
        for body in [
            "# ADR-0007: Auth\n\n  - Status: accepted\n",
            "# ADR-0007: Auth\n\nStatus: accepted\n",
        ] {
            let problems = file_format_problems("0007-a.md", 7, body);
            assert!(
                problems.iter().any(|p| p.contains("at column 0")),
                "{body:?} -> {problems:?}"
            );
        }
    }

    #[test]
    fn status_token_reads_list_and_bare_forms() {
        assert_eq!(
            status_token("# T\n\n- Status: accepted\n- Note: x\n"),
            "accepted"
        );
        assert_eq!(status_token("# T\n\nStatus: superseded\n"), "superseded");
        assert_eq!(status_token("# T\n\nno status here\n"), "");
    }

    #[test]
    fn file_format_problems_rejects_proposed_on_a_numbered_adr() {
        // The message must name the remedy (the drafts pen), not just the rule —
        // "proposed is illegal" without "put it in drafts/" tells the reader what
        // they may not do and nothing about what they should.
        let problems =
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: proposed\n");
        assert!(
            problems.iter().any(|p| p.contains("docs/adr/drafts/")),
            "{problems:?}"
        );
    }

    #[test]
    fn file_format_problems_accepts_every_numbered_token() {
        for token in NUMBERED_STATUS_VOCAB {
            let body = format!("# ADR-0007: Auth\n\n- Status: {token}\n");
            let problems = file_format_problems("0007-a.md", 7, &body);
            assert!(problems.is_empty(), "{token}: {problems:?}");
        }
    }

    #[test]
    fn out_of_vocab_message_no_longer_advertises_proposed() {
        // Teeth: the message formats the vocab with `{:?}`, so a vocab constant
        // carrying `proposed` would tell a numbered ADR it is legal while the
        // rule above rejects it. Restore STATUS_VOCAB here and this fails.
        let problems =
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: accpeted\n");
        assert!(
            problems.iter().any(|p| p.contains("not one of")),
            "{problems:?}"
        );
        assert!(
            !problems.iter().any(|p| p.contains("\"proposed\"")),
            "{problems:?}"
        );
    }

    #[test]
    fn file_format_problems_flags_each_violation() {
        // Clean file: no problems.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: accepted\n")
                .is_empty()
        );
        // Legacy heading form.
        assert!(
            file_format_problems("0007-a.md", 7, "# 0007. Auth\n\n- Status: accepted\n")
                .iter()
                .any(|p| p.contains("heading must be"))
        );
        // Filename/heading number mismatch.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0008: Auth\n\n- Status: accepted\n")
                .iter()
                .any(|p| p.contains("heading must be"))
        );
        // Missing status.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\nbody\n")
                .iter()
                .any(|p| p.contains("missing a `- Status:"))
        );
        // Trailing prose after the token.
        assert!(
            file_format_problems(
                "0007-a.md",
                7,
                "# ADR-0007: Auth\n\n- Status: accepted (superseded)\n"
            )
            .iter()
            .any(|p| p.contains("single token"))
        );
        // Out-of-vocabulary token.
        assert!(
            file_format_problems("0007-a.md", 7, "# ADR-0007: Auth\n\n- Status: accpeted\n")
                .iter()
                .any(|p| p.contains("not one of"))
        );
    }

    #[test]
    fn parse_adr_dir_reads_sorts_and_skips_non_adrs() {
        let repo = scratch_repo("parse-dir");
        let adr = repo.join("docs/adr");
        std::fs::write(
            adr.join("0002-b.md"),
            "# ADR-0002: Second\n\n- Status: superseded\n",
        )
        .unwrap();
        std::fs::write(
            adr.join("0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        // Skipped: not markdown, and markdown without a leading number.
        std::fs::write(adr.join("0003-c.txt"), "ignore me").unwrap();
        std::fs::write(adr.join("template.md"), "# ADR-template\n").unwrap();

        let entries = parse_adr_dir(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        let projected: Vec<_> = entries
            .iter()
            .map(|e| {
                (
                    e.num,
                    e.filename.as_str(),
                    e.title.as_str(),
                    e.status.as_str(),
                )
            })
            .collect();
        assert_eq!(
            projected,
            vec![
                (1, "0001-a.md", "First", "accepted"),
                (2, "0002-b.md", "Second", "superseded"),
            ]
        );
    }
}
