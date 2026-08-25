//! Generate the ADR index table in `docs/README.md` as a projection of
//! `docs/adr/`. Only the mechanical cells — number, link target, status — are
//! generated; the title cell is hand-curated and preserved (seeded from the ADR
//! heading when a row is first created). The table lives between HTML-comment
//! markers so only that block is ever rewritten.
//!
//! This core is shared by `adr sync-readme` (the writer, here), automated ADR
//! promotion, the deprecated `adr renumber` compatibility command, and the
//! read-only parity gate. No behavior lives in more than one place.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;
use crate::result::StepResult;

pub const README: &str = "docs/README.md";
pub const ADR_DIR: &str = "docs/adr";
/// The materialized view of the architecture. Every accepted ADR must be *cited*
/// here — `view_parity_problems` checks exactly that, and no more.
pub const VIEW: &str = "docs/ARCHITECTURE.md";
pub const BEGIN: &str = "<!-- adr-table:begin -->";
pub const END: &str = "<!-- adr-table:end -->";

/// The link target for an ADR as written from `docs/` — the form the README table
/// renders and the view cites. Spelled once so the three call sites cannot drift.
fn adr_link(filename: &str) -> String {
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

/// A parsed committed table row. Cells are trimmed (padding-proof).
#[derive(Debug, PartialEq, Eq)]
pub struct TableRow {
    pub num: u32,
    pub target: String,
    pub title: String,
    pub status: String,
}

/// A directory entry that qualifies as an ADR: a regular `*.md` file whose name
/// carries a leading number. Content is intentionally not read here so each
/// caller keeps its own IO-error policy.
struct AdrFile {
    num: u32,
    filename: String,
    path: PathBuf,
}

/// The qualifying ADR files under `repo/docs/adr`, unsorted — the single home of
/// the "what counts as an ADR file" rule (`is_file` → `.md` → leading number).
/// The `read_dir` error is returned unwrapped so callers phrase their own context
/// (`parse_adr_dir` wants "reading <dir>", `format_problems` wants
/// "cannot read <dir>").
fn adr_files(repo: &Path) -> Result<Vec<AdrFile>> {
    adr_files_from(
        std::fs::read_dir(repo.join(ADR_DIR))?,
        std::fs::DirEntry::path,
        std::fs::DirEntry::file_name,
        std::fs::DirEntry::file_type,
    )
}

fn adr_files_from<T>(
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

/// Parse one committed table row `| [NNNN](adr/slug.md) | Title | status |`.
/// `None` for the header, the separator, and any non-row line.
fn parse_row(line: &str) -> Option<TableRow> {
    let cells: Vec<&str> = line.split('|').map(str::trim).collect();
    // A table row is bounded by pipes, so the split yields empty first/last cells
    // around exactly three inner cells.
    if cells.len() != 5 || !cells[0].is_empty() || !cells[4].is_empty() {
        return None;
    }
    let (link, title, status) = (cells[1], cells[2], cells[3]);
    let link = link.strip_prefix('[')?;
    let close = link.find(']')?;
    let num: u32 = link[..close].parse().ok()?;
    let paren = link[close..].strip_prefix("](")?;
    let target = paren.strip_suffix(')')?.to_string();
    Some(TableRow {
        num,
        target,
        title: title.to_string(),
        status: status.to_string(),
    })
}

/// Parse the ADR files under `repo/docs/adr`, sorted ascending by number.
pub fn parse_adr_dir(repo: &Path) -> Result<Vec<AdrEntry>> {
    parse_adr_files(repo, adr_files(repo))
}

fn parse_adr_files(repo: &Path, files: Result<Vec<AdrFile>>) -> Result<Vec<AdrEntry>> {
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

/// Parse the committed table rows out of the block text between the markers.
pub fn parse_table_block(block: &str) -> Vec<TableRow> {
    block.lines().filter_map(parse_row).collect()
}

/// The generated table block: header + separator + one row per ADR entry
/// (ascending), reusing an existing row's title when present, else seeding the
/// title from the ADR heading. Single-space padded — prettier owns alignment.
pub fn render_block(entries: &[AdrEntry], existing: &[TableRow]) -> String {
    let mut out = String::from("| #   | Title | Status |\n| --- | ----- | ------ |\n");
    for r in resolved_rows(entries, existing) {
        out.push_str(&format!(
            "| [{:04}]({}) | {} | {} |\n",
            r.num, r.target, r.title, r.status
        ));
    }
    out.trim_end().to_string()
}

/// The byte range strictly between the ADR-table markers: `(start, end)` where
/// `start` is just past `BEGIN` and `end` is at `END`. Errors when either marker
/// is missing or they are out of order.
fn marker_bounds(readme: &str) -> Result<(usize, usize)> {
    let begin = readme
        .find(BEGIN)
        .with_context(|| format!("{README} is missing the `{BEGIN}` marker"))?;
    let end = readme
        .find(END)
        .with_context(|| format!("{README} is missing the `{END}` marker"))?;
    anyhow::ensure!(begin < end, "{README} adr-table markers are out of order");
    Ok((begin + BEGIN.len(), end))
}

/// Replace the text strictly between the markers with `new_block`. Errors when a
/// marker is missing or out of order.
pub fn splice_block(readme: &str, new_block: &str) -> Result<String> {
    let (after_begin, end) = marker_bounds(readme)?;
    Ok(format!(
        "{}\n\n{}\n\n{}",
        &readme[..after_begin],
        new_block,
        &readme[end..]
    ))
}

/// The block text between the markers (for reading existing titles).
fn extract_block(readme: &str) -> Result<String> {
    let (start, end) = marker_bounds(readme)?;
    Ok(readme[start..end].to_string())
}

/// The desired table rows, ascending by number, applying the title-preservation
/// rule once: reuse an existing row's title when a row with that number exists,
/// else seed it from the ADR heading. The single source of that rule — both the
/// renderer ([`render_block`]) and the idempotence check ([`sync_readme_at`])
/// consume it, so they can never disagree.
fn resolved_rows(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<TableRow> {
    let title_by_num: BTreeMap<u32, &str> =
        existing.iter().map(|r| (r.num, r.title.as_str())).collect();
    let mut sorted: Vec<&AdrEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.num);
    sorted
        .into_iter()
        .map(|e| {
            let title = title_by_num
                .get(&e.num)
                .copied()
                .unwrap_or(e.title.as_str())
                .to_string();
            TableRow {
                num: e.num,
                target: adr_link(&e.filename),
                title,
                status: e.status.clone(),
            }
        })
        .collect()
}

/// Regenerate the ADR table in `repo/docs/README.md` from `repo/docs/adr`.
/// A no-op (no write) when the table already matches semantically, so it is
/// idempotent regardless of prettier's column padding. Returns a human summary.
pub fn sync_readme_at(repo: &Path) -> Result<String> {
    let readme_path = repo.join(README);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let entries = parse_adr_dir(repo)?;
    let existing = parse_table_block(&extract_block(&readme)?);

    let desired = resolved_rows(&entries, &existing);
    if desired == existing {
        return Ok(format!("{} rows, already in sync", entries.len()));
    }

    let updated = splice_block(&readme, &render_block(&entries, &existing))?;
    std::fs::write(&readme_path, &updated)
        .with_context(|| format!("writing {}", readme_path.display()))?;

    let existing_nums: BTreeSet<u32> = existing.iter().map(|r| r.num).collect();
    let entry_nums: BTreeSet<u32> = entries.iter().map(|e| e.num).collect();
    let added = entry_nums.difference(&existing_nums).count();
    let removed = existing_nums.difference(&entry_nums).count();
    Ok(format!(
        "{} rows ({added} added, {removed} removed)",
        entries.len()
    ))
}

/// Whether `repo`'s README carries the ADR-table markers. `Ok(false)` when the
/// README is absent (a scratch/test repo may omit it entirely) so the caller can
/// skip table sync; a genuine read error still propagates rather than being
/// mistaken for "no markers".
pub fn readme_has_markers(repo: &Path) -> Result<bool> {
    let readme_path = repo.join(README);
    match std::fs::read_to_string(&readme_path) {
        Ok(s) => Ok(s.contains(BEGIN) && s.contains(END)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("reading {}", readme_path.display())),
    }
}

/// Entry point for `cargo xtask adr sync-readme`.
pub fn sync_readme() -> StepResult {
    match sync_readme_at(Path::new(".")) {
        Ok(summary) => StepResult::ok("adr-sync-readme").detail(summary),
        Err(e) => StepResult::fail("adr-sync-readme").detail(format!("{e:#}")),
    }
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

/// The `adr-readme-parity` problems: the committed table's mechanical cells
/// (number, link target, status), row presence, and ordering must match the ADR
/// directory. Titles are not compared (they are hand-owned). Does not panic on a
/// transient duplicate number — that is `identifier-collisions`' concern.
pub fn parity_problems(entries: &[AdrEntry], existing: &[TableRow]) -> Vec<String> {
    let mut problems = Vec::new();
    let row_by_num: BTreeMap<u32, &TableRow> = existing.iter().map(|r| (r.num, r)).collect();
    let entry_nums: BTreeSet<u32> = entries.iter().map(|e| e.num).collect();

    for e in entries {
        match row_by_num.get(&e.num) {
            None => problems.push(format!("ADR {:04} has no README table row", e.num)),
            Some(r) => {
                let want = adr_link(&e.filename);
                if r.target != want {
                    problems.push(format!(
                        "ADR {:04} row link is `{}`, expected `{want}`",
                        e.num, r.target
                    ));
                }
                if r.status != e.status {
                    problems.push(format!(
                        "ADR {:04} row status is `{}`, expected `{}`",
                        e.num, r.status, e.status
                    ));
                }
            }
        }
    }
    for r in existing {
        if !entry_nums.contains(&r.num) {
            problems.push(format!(
                "README row {:04} has no matching ADR file (orphan)",
                r.num
            ));
        }
    }
    let nums: Vec<u32> = existing.iter().map(|r| r.num).collect();
    let mut ascending = nums.clone();
    ascending.sort_unstable();
    if nums != ascending {
        problems.push("README ADR rows are not in ascending number order".to_string());
    }

    problems.sort();
    problems
}

/// Read `repo`'s README + ADR directory and compute the parity problems. Errors
/// when the README is unreadable or the table markers are absent.
pub fn parity_report(repo: &Path) -> Result<Vec<String>> {
    parity_report_with(repo, || adr_files(repo))
}

fn parity_report_with(
    repo: &Path,
    files: impl FnOnce() -> Result<Vec<AdrFile>>,
) -> Result<Vec<String>> {
    let readme_path = repo.join(README);
    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let entries = parse_adr_files(repo, files())?;
    let existing = parse_table_block(&extract_block(&readme)?);
    Ok(parity_problems(&entries, &existing))
}

/// The `adr-view-parity` problems: every **accepted** ADR must be cited at least
/// once in `docs/ARCHITECTURE.md`. `superseded`, `deprecated` and `rejected` are
/// excluded — the view describes the architecture as it stands, not its history.
///
/// A citation is either a markdown link whose target is `adr/NNNN-<slug>.md` (how
/// the view normally cites) or a bare `ADR-NNNN` token (how prose sometimes does).
/// Both count.
///
/// There is deliberately no allowlist, exemption file, or baseline — this is a
/// pure function of the tree, matching the coverage gate's stateless stance
/// (ADR-0050). The absence of an escape hatch is the design: when an ADR is
/// named here the fix is to describe it in the view, never to exempt it.
///
/// A number cited by the view with no corresponding ADR file is a dangling link,
/// not a parity problem — only accepted ADRs are walked.
///
/// Errors when the view is unreadable: a gate that silently passes because its
/// input vanished would be retired by a `rm`.
///
/// **What this cannot see** (ADR-0085's honesty obligation). It is a substring
/// test over the whole file, so a citation counts wherever it appears —  inside a
/// fenced code block, an HTML comment, or a "superseded by ADR-NNNN" aside. It
/// therefore proves that an ADR is *mentioned*, not that the surrounding prose is
/// true, not that the mention is anywhere sensible, and not that a `superseded`
/// ADR has stopped being cited as current. Those are the replay audit's job, and
/// the reason it is not replaced by this step. The failure direction is
/// deliberate: a deliberately odd view can produce a false pass, but nothing
/// produces a false alarm, which is the right way round for a step that blocks
/// every commit.
pub fn view_parity_problems(repo: &Path) -> Result<Vec<String>> {
    let view_path = repo.join(VIEW);
    let view = std::fs::read_to_string(&view_path)
        .with_context(|| format!("reading {}", view_path.display()))?;
    let mut problems = Vec::new();
    for e in parse_adr_dir(repo)? {
        if e.status != "accepted" {
            continue;
        }
        let link = adr_link(&e.filename);
        let token = format!("ADR-{:04}", e.num);
        if !view.contains(&link) && !view.contains(&token) {
            problems.push(format!(
                "ADR {:04} ({}) is accepted but is not cited in {VIEW}",
                e.num, e.title
            ));
        }
    }
    problems.sort();
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(num: u32, file: &str, title: &str, status: &str) -> AdrEntry {
        AdrEntry {
            num,
            filename: file.into(),
            title: title.into(),
            status: status.into(),
        }
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
    fn parse_row_trims_padded_cells_and_skips_non_rows() {
        let r = parse_row("| [0007](adr/0007-auth-mechanisms.md)   | Dual-Path Auth | accepted |")
            .expect("a row");
        assert_eq!(r.num, 7);
        assert_eq!(r.target, "adr/0007-auth-mechanisms.md");
        assert_eq!(r.title, "Dual-Path Auth");
        assert_eq!(r.status, "accepted");
        assert!(parse_row("| # | Title | Status |").is_none());
        assert!(parse_row("| --- | --- | --- |").is_none());
        assert!(parse_row("plain text").is_none());
    }

    #[test]
    fn render_block_preserves_existing_title_and_seeds_new_from_heading() {
        let entries = vec![
            entry(1, "0001-a.md", "Heading One", "accepted"),
            entry(2, "0002-b.md", "Heading Two", "accepted"),
        ];
        let existing = vec![TableRow {
            num: 1,
            target: "adr/0001-a.md".into(),
            title: "Curated One".into(),
            status: "accepted".into(),
        }];
        let block = render_block(&entries, &existing);
        // Existing row keeps its curated title; the new row seeds from the heading.
        assert!(block.contains("| [0001](adr/0001-a.md) | Curated One | accepted |"));
        assert!(block.contains("| [0002](adr/0002-b.md) | Heading Two | accepted |"));
    }

    #[test]
    fn render_block_drops_orphans_and_sorts_ascending() {
        let entries = vec![
            entry(3, "0003-c.md", "Three", "accepted"),
            entry(1, "0001-a.md", "One", "accepted"),
        ];
        // An existing row for a now-deleted ADR 2 must not survive.
        let existing = vec![TableRow {
            num: 2,
            target: "adr/0002-b.md".into(),
            title: "Two".into(),
            status: "accepted".into(),
        }];
        let block = render_block(&entries, &existing);
        let one = block.find("0001-a.md").unwrap();
        let three = block.find("0003-c.md").unwrap();
        assert!(one < three, "ascending order");
        assert!(!block.contains("0002-b.md"), "orphan dropped");
    }

    #[test]
    fn splice_block_replaces_only_between_markers() {
        let readme = format!("intro\n\n{BEGIN}\n\nOLD TABLE\n\n{END}\n\noutro\n");
        let out = splice_block(&readme, "NEW TABLE").unwrap();
        assert!(out.contains("intro\n"));
        assert!(out.contains("outro\n"));
        assert!(out.contains(&format!("{BEGIN}\n\nNEW TABLE\n\n{END}")));
        assert!(!out.contains("OLD TABLE"));
    }

    #[test]
    fn splice_block_errors_on_missing_marker() {
        let err = splice_block("no markers here", "x").unwrap_err();
        assert!(format!("{err:#}").contains("marker"));
    }

    #[test]
    fn desired_matches_current_when_in_sync() {
        let entries = vec![entry(1, "0001-a.md", "Heading", "accepted")];
        let existing = vec![TableRow {
            num: 1,
            target: "adr/0001-a.md".into(),
            title: "Curated".into(),
            status: "accepted".into(),
        }];
        // The preserved title is the curated one, so desired == current: a no-op.
        assert_eq!(resolved_rows(&entries, &existing), existing);
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
    fn parity_problems_flags_mechanical_drift_but_ignores_titles() {
        let entries = vec![
            entry(1, "0001-a.md", "H1", "accepted"),
            entry(2, "0002-b.md", "H2", "superseded"),
        ];
        // Row 1: title differs (OK — not compared) but everything mechanical agrees.
        // Row 2: status is stale. Plus an orphan row 9.
        let existing = vec![
            TableRow {
                num: 1,
                target: "adr/0001-a.md".into(),
                title: "Totally Different".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 2,
                target: "adr/0002-b.md".into(),
                title: "H2".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 9,
                target: "adr/0009-x.md".into(),
                title: "Ghost".into(),
                status: "accepted".into(),
            },
        ];
        let problems = parity_problems(&entries, &existing);
        assert!(
            problems.iter().any(|p| p.contains("ADR 0002 row status")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("orphan")),
            "{problems:?}"
        );
        assert!(
            !problems.iter().any(|p| p.contains("0001")),
            "title-only diff must not flag: {problems:?}"
        );
    }

    #[test]
    fn parity_problems_flags_missing_row_and_bad_ordering() {
        let entries = vec![
            entry(1, "0001-a.md", "H1", "accepted"),
            entry(2, "0002-b.md", "H2", "accepted"),
        ];
        // Row for ADR 2 is missing; the two present rows are out of order.
        let existing = vec![
            TableRow {
                num: 3,
                target: "adr/0003-c.md".into(),
                title: "T3".into(),
                status: "accepted".into(),
            },
            TableRow {
                num: 1,
                target: "adr/0001-a.md".into(),
                title: "T1".into(),
                status: "accepted".into(),
            },
        ];
        let problems = parity_problems(&entries, &existing);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("ADR 0002 has no README table row")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("ascending")),
            "{problems:?}"
        );
    }

    #[test]
    fn parity_problems_does_not_panic_on_duplicate_number() {
        // The always-0000 sentinel: two entries share num 0. Must not panic.
        let entries = vec![
            entry(0, "0000-doc.md", "Doc", "accepted"),
            entry(0, "0000-new.md", "New", "accepted"),
        ];
        let existing = vec![TableRow {
            num: 0,
            target: "adr/0000-doc.md".into(),
            title: "Doc".into(),
            status: "accepted".into(),
        }];
        let _ = parity_problems(&entries, &existing);
    }

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

    #[test]
    fn fail_closed_population_unreadable_adr_readme_file_type() {
        struct Fake {
            path: PathBuf,
            name: OsString,
        }
        let repo = scratch_repo("adr-file-type-owner");
        std::fs::create_dir_all(repo.join(ADR_DIR)).unwrap();
        std::fs::write(repo.join(README), format!("# ADRs\n{BEGIN}\n{END}\n")).unwrap();
        let path = repo.join("docs/adr/0001-unreadable.md");
        let report = parity_report_with(&repo, || {
            adr_files_from(
                [Ok(Fake {
                    path: path.clone(),
                    name: OsString::from("0001-unreadable.md"),
                })],
                |entry| entry.path.clone(),
                |entry| entry.name.clone(),
                |_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected",
                    ))
                },
            )
        });
        let error = match report {
            Ok(_) => panic!("injected file-type failure must fail the parity population"),
            Err(error) => error,
        };
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
        let step = crate::steps::adr_check::parity_step(Err(error));
        assert!(!step.ok);
        let detail = step.detail.unwrap();
        assert!(detail.contains(&path.display().to_string()), "{detail}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn parity_report_reads_readme_and_errors_without_markers() {
        let repo = scratch_repo("parity-report");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();

        // Markers present, mechanical cells agree (title free to differ): clean.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | Curated | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();
        assert!(parity_report(&repo).unwrap().is_empty());

        // A stale status cell is reported.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | Curated | proposed |\n\n{END}\n"
            ),
        )
        .unwrap();
        let problems = parity_report(&repo).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("ADR 0001 row status")),
            "{problems:?}"
        );

        // No markers at all: an error, not a silent empty report.
        std::fs::write(repo.join("docs/README.md"), "# Docs\n\nno table here\n").unwrap();
        assert!(parity_report(&repo).is_err());

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn readme_has_markers_distinguishes_absent_present_and_missing() {
        let repo = scratch_repo("has-markers");
        // No README file: absent, reported as false (not an error).
        assert!(!readme_has_markers(&repo).unwrap());
        // Present markers.
        std::fs::write(
            repo.join("docs/README.md"),
            format!("# Docs\n\n{BEGIN}\n\n{END}\n"),
        )
        .unwrap();
        assert!(readme_has_markers(&repo).unwrap());
        // README exists but carries no markers.
        std::fs::write(repo.join("docs/README.md"), "# Docs\n").unwrap();
        assert!(!readme_has_markers(&repo).unwrap());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn gates_ignore_docs_adr_template_md() {
        // `docs/adr/template.md` (the copyable ADR skeleton, #207) has no leading
        // number, so both gate entry points must skip it: it is neither an
        // `adr-format` subject nor an `adr-readme-parity` row. Guards against a
        // refactor that starts checking `docs/adr/*.md` regardless of number.
        //
        // Teeth: rename the fixture to `0099-template.md` and this fails — the
        // `# ADR-0000:` heading mismatches `0099` (adr-format) and no README row
        // exists for 0099 (parity). See the plan's inversion check.
        let repo = scratch_repo("template-ignored");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        std::fs::write(
            repo.join("docs/adr/template.md"),
            "# ADR-0000: Title of the decision\n\n- Status: proposed\n",
        )
        .unwrap();
        // README carries only the real ADR's row — none for the template.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | First | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();

        let fmt = format_problems(&repo);
        let parity = parity_report(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            fmt.is_empty(),
            "template.md must not be an adr-format subject: {fmt:?}"
        );
        assert!(
            parity.is_empty(),
            "template.md must not be a parity row: {parity:?}"
        );
    }

    #[test]
    fn gates_ignore_docs_adr_drafts_subdir() {
        // Feature PRs track numberless files under `docs/adr/drafts/`; the
        // serialized promoter numbers them after feature merge. The subdirectory
        // is excluded twice over by the shared enumeration rule (a non-recursive
        // `read_dir` skips the subdir entry, which is not a file; and there is no
        // leading number), so a draft — even one that violates every ADR format
        // rule (no `# ADR-NNNN:` heading, no status line) — must never trip
        // `adr-format` or `adr-readme-parity`.
        //
        // Teeth: move the fixture up to `docs/adr/0099-some-decision.md` and this
        // fails — the `# ADR-DRAFT:` heading mismatches 0099 (adr-format) and no
        // README row exists for 0099 (parity).
        let repo = scratch_repo("drafts-ignored");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        std::fs::create_dir_all(repo.join("docs/adr/drafts")).unwrap();
        std::fs::write(
            repo.join("docs/adr/drafts/some-decision.md"),
            "# ADR-DRAFT: Some Decision\n\nNo status line, no number — still invisible.\n",
        )
        .unwrap();
        // README carries only the real ADR's row — none for the draft.
        std::fs::write(
            repo.join("docs/README.md"),
            format!(
                "# Docs\n\n{BEGIN}\n\n| # | Title | Status |\n| --- | --- | --- |\n\
                 | [0001](adr/0001-a.md) | First | accepted |\n\n{END}\n"
            ),
        )
        .unwrap();

        let fmt = format_problems(&repo);
        let parity = parity_report(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);

        assert!(
            fmt.is_empty(),
            "a drafts/ entry must not be an adr-format subject: {fmt:?}"
        );
        assert!(
            parity.is_empty(),
            "a drafts/ entry must not be a parity row: {parity:?}"
        );
    }

    /// Write `docs/ARCHITECTURE.md` in a scratch repo.
    fn write_view(repo: &Path, body: &str) {
        std::fs::write(repo.join(VIEW), body).unwrap();
    }

    #[test]
    fn view_parity_accepts_a_markdown_link_citation() {
        let repo = scratch_repo("view-link");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(
            &repo,
            "# View\n\nWe do it this way ([0001](adr/0001-a.md)).\n",
        );

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_accepts_a_bare_token_citation() {
        let repo = scratch_repo("view-token");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nThe seam is fixed by ADR-0001.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_reports_an_uncited_accepted_adr() {
        let repo = scratch_repo("view-uncited");
        std::fs::write(
            repo.join("docs/adr/0007-lonely.md"),
            "# ADR-0007: Lonely decision\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nNothing is cited here.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("0007"), "{problems:?}");
        assert!(problems[0].contains("Lonely decision"), "{problems:?}");
    }

    #[test]
    fn view_parity_ignores_a_superseded_adr() {
        let repo = scratch_repo("view-superseded");
        std::fs::write(
            repo.join("docs/adr/0002-old.md"),
            "# ADR-0002: Old decision\n\n- Status: superseded\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nNothing is cited here.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_ignores_a_citation_with_no_adr_file() {
        // The view may name a number that has no file. That is a dangling link,
        // not this step's business — the step only walks accepted ADRs.
        let repo = scratch_repo("view-dangling");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();
        write_view(&repo, "# View\n\nSee ADR-0001 and also ADR-9999.\n");

        let problems = view_parity_problems(&repo).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn view_parity_errors_when_the_view_is_absent() {
        // A step that silently passes when its input is missing is worse than
        // useless: deleting the view would retire the gate.
        let repo = scratch_repo("view-absent");
        std::fs::write(
            repo.join("docs/adr/0001-a.md"),
            "# ADR-0001: First\n\n- Status: accepted\n",
        )
        .unwrap();

        let report = view_parity_problems(&repo);
        let _ = std::fs::remove_dir_all(&repo);
        assert!(report.is_err(), "absent view must be an error");
    }
}
