//! Markdown link scanning — the single home of "what is a link, and where does it
//! point" for the whole of `xtask` (#682).
//!
//! Two callers share it: `adr::strip_one_level` (rewriting a promoted draft's
//! targets) and the `doc-links` gate step (checking the tracked corpus). Keeping
//! one scanner is what makes those two agree about code spans, URL schemes, and
//! fragments — a second implementation would drift.

use std::path::Path;

use anyhow::{Context, Result};

/// Trees excluded from the gate. `docs/archive/` is a frozen record — its links are
/// dead because the docs moved on, and rewriting them would falsify the record.
/// `docs/superpowers/` holds transient specs and plans, which routinely link files
/// they only propose to create.
const EXCLUDED: &[&str] = &["docs/archive/", "docs/superpowers/"];

/// An inline Markdown link found outside code spans and fenced blocks. Carries a
/// byte range rather than a line number so the scanner computes only what its
/// callers read.
pub struct Link {
    /// Byte range of the *target* within the source — the text between `](` and the
    /// closing `)` (or the whitespace that starts a link title).
    pub span: std::ops::Range<usize>,
    /// The target text.
    pub target: String,
}

/// Blank out fenced blocks and inline code spans, replacing every non-newline byte
/// with a space. Length- and line-preserving, so byte offsets computed on the mask
/// are valid in the original.
///
/// UTF-8 safety: the only bytes inspected are backtick, `~`, space, tab and `\n`,
/// none of which can occur inside a multi-byte character (lead and continuation
/// bytes are all >= 0x80). So byte-wise scanning never lands mid-character, and the
/// `from_utf8` below cannot fail.
///
/// Fences and inline spans are matched by backtick-RUN length, not by a fixed
/// three-character marker: a closer must use the opener's character and be at least
/// as long, and an inline run of N pairs with the next run of exactly N. Both rules
/// exist to keep the block that wraps other fenced blocks (a 4-backtick fence) from
/// being closed by the 3-backtick fences inside it.
///
/// An unclosed fence masks to end of file. That yields false negatives (links after
/// it go unchecked), never false positives — the safe direction for a gate. Only
/// inline `](target)` links are scanned at all, so reference-style definitions
/// (`[x]: ./a.md`) go unchecked for the same benign reason.
fn mask_code(body: &str) -> String {
    /// Blank a whole line, keeping its newline.
    fn blank(out: &mut [u8], at: usize, line: &str) {
        for (k, b) in line.bytes().enumerate() {
            if b != b'\n' {
                out[at + k] = b' ';
            }
        }
    }
    /// The fence character and run length this line opens or closes with, if any.
    /// Leading whitespace is trimmed first — a fence may be indented inside a list
    /// item. CommonMark requires a run of at least three `` ` `` or `~`.
    fn fence_run(line: &str) -> Option<(u8, usize)> {
        let bytes = line.trim_start().as_bytes();
        let ch = *bytes.first()?;
        if ch != b'`' && ch != b'~' {
            return None;
        }
        let run = bytes.iter().take_while(|&&b| b == ch).count();
        (run >= 3).then_some((ch, run))
    }

    /// Blank paired backtick spans within a single line. A run of N backticks pairs
    /// with the next run of EXACTLY N — which is how a literal backtick is written
    /// inline (``` ``a ` b`` ```). Matching a run of 1 against any later backtick
    /// would end that span early and leave its tail live.
    fn blank_spans(out: &mut [u8], at: usize, line: &str) {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'`' {
                i += 1;
                continue;
            }
            let open = b[i..].iter().take_while(|&&x| x == b'`').count();
            let mut j = i + open;
            let closed = loop {
                while j < b.len() && b[j] != b'`' {
                    j += 1;
                }
                if j >= b.len() {
                    break None;
                }
                let run = b[j..].iter().take_while(|&&x| x == b'`').count();
                if run == open {
                    break Some(j + run);
                }
                j += run;
            };
            match closed {
                Some(end) => {
                    for slot in out.iter_mut().take(at + end).skip(at + i) {
                        *slot = b' ';
                    }
                    i = end;
                }
                // Unterminated on this line: leave it live rather than masking the
                // rest of the line on the strength of one stray backtick.
                None => i += open,
            }
        }
    }

    let mut out: Vec<u8> = body.bytes().collect();
    let mut fence: Option<(u8, usize)> = None;
    let mut at = 0usize;
    for line in body.split_inclusive('\n') {
        match (fence, fence_run(line)) {
            (None, Some(open)) => {
                fence = Some(open);
                blank(&mut out, at, line);
            }
            // A closer must use the same character and be AT LEAST as long as the
            // opener. Equality would let the 3-backtick fences inside a 4-backtick
            // block close it, leaving the block's own prose live.
            (Some((open_ch, open_run)), Some((ch, run))) if ch == open_ch && run >= open_run => {
                fence = None;
                blank(&mut out, at, line);
            }
            (Some(_), _) => blank(&mut out, at, line),
            (None, None) => blank_spans(&mut out, at, line),
        }
        at += line.len();
    }
    String::from_utf8(out).expect("masking preserves UTF-8 boundaries")
}

/// Every inline `](target)` link in `body`, skipping fenced code blocks and inline
/// code spans.
pub fn links_in(body: &str) -> Vec<Link> {
    let masked = mask_code(body);
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b'(' {
            let start = i + 2;
            // The target ends at the closing paren, or at the whitespace that
            // introduces an optional link title.
            let mut end = start;
            while end < bytes.len() && !matches!(bytes[end], b')' | b'\n' | b' ' | b'\t') {
                end += 1;
            }
            // The link is only well-formed if a closing paren follows on this line.
            let mut close = end;
            while close < bytes.len() && bytes[close] != b')' && bytes[close] != b'\n' {
                close += 1;
            }
            if end > start && close < bytes.len() && bytes[close] == b')' {
                out.push(Link {
                    span: start..end,
                    target: body[start..end].to_string(),
                });
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// True when `target` is a relative path worth resolving — i.e. not a
/// `http:`/`https:`/`mailto:` URL and not a bare `#anchor`.
pub fn is_relative_target(target: &str) -> bool {
    !target.starts_with('#')
        && !target.starts_with("http://")
        && !target.starts_with("https://")
        && !target.starts_with("mailto:")
}

/// 1-based line containing byte `offset`.
fn line_at(body: &str, offset: usize) -> usize {
    body[..offset].matches('\n').count() + 1
}

/// A dead relative link in one file.
pub struct DeadLink {
    pub line: usize,
    pub target: String,
}

/// Relative links in `repo`/`rel` whose target does not exist on disk.
///
/// The shared per-file unit: `promote` checks the one file it just wrote, the gate
/// maps this over the whole corpus. Neither owns a second resolver, so "what counts
/// as dead" cannot diverge between a warning and a hard failure.
///
/// Targets resolve against the file's own directory, and a `#fragment` is dropped
/// first — anchors within a document are not validated (a link to a real file with a
/// stale anchor still passes).
pub fn dead_links_in(repo: &Path, rel: &str) -> Result<Vec<DeadLink>> {
    let path = repo.join(rel);
    let body = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
    let dir = path.parent().unwrap_or(repo).to_path_buf();
    Ok(links_in(&body)
        .into_iter()
        .filter(|link| is_relative_target(&link.target))
        .filter(|link| {
            let bare = link.target.split('#').next().unwrap_or_default();
            !bare.is_empty() && !dir.join(bare).exists()
        })
        .map(|link| DeadLink {
            line: line_at(&body, link.span.start),
            target: link.target,
        })
        .collect())
}

/// Tracked `*.md` under `repo`, minus [`EXCLUDED`] and minus tracked-but-absent
/// paths.
///
/// Tracked, not on-disk: an untracked scratch file is nobody's contract, and a
/// gitignored draft must stay invisible to the gate. Absent paths are dropped so a
/// staged deletion fails at its `git rm`, not here.
fn gated_files(repo: &Path) -> Result<Vec<String>> {
    Ok(crate::git::ls_files_md(repo)?
        .into_iter()
        .filter(|rel| !EXCLUDED.iter().any(|tree| rel.starts_with(tree)))
        .filter(|rel| repo.join(rel).exists())
        .collect())
}

/// Every dead link across [`gated_files`], formatted `<file>:<line> -> <target>`.
pub fn problems(repo: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for rel in gated_files(repo)? {
        for dead in dead_links_in(repo, &rel)? {
            out.push(format!("{rel}:{} -> {}", dead.line, dead.target));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A fresh git repo under a pid-scoped temp dir, identity configured — the
    /// `git.rs::tests::temp_repo` idiom. `tag` must be unique per test: the tests in
    /// this module share a process, so the pid alone does not separate them.
    fn repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jaunder-links-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["config", "user.email", "t@t"],
            &["config", "user.name", "t"],
        ] {
            assert!(git(&dir, args).success());
        }
        dir
    }

    /// Run git against `dir` via [`crate::git::at`] — which scrubs `GIT_DIR` and
    /// friends. Not optional: these tests run under the pre-commit hook, which
    /// exports them, and a bare `Command::new("git")` would silently retarget every
    /// call at the real repository.
    fn git(dir: &Path, args: &[&str]) -> std::process::ExitStatus {
        crate::git::at(dir).args(args).status().unwrap()
    }

    /// Write `rel` under `dir`, creating its parent.
    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// [`write`], then track it — `git ls-files` is what the gate enumerates.
    fn commit(dir: &Path, rel: &str, body: &str) {
        write(dir, rel, body);
        assert!(git(dir, &["add", rel]).success());
        assert!(git(dir, &["commit", "-qm", "c"]).success());
    }

    // --- links_in: what counts as a link ---

    #[test]
    fn finds_a_plain_inline_link() {
        let ls = links_in("see [x](a.md) here");
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].target, "a.md");
    }

    #[test]
    fn finds_multiple_links_on_one_line() {
        let ls = links_in("[a](x.md) and [b](y.md)");
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[1].target, "y.md");
    }

    #[test]
    fn span_covers_exactly_the_target() {
        let body = "see [x](a.md)";
        let l = &links_in(body)[0];
        assert_eq!(&body[l.span.clone()], "a.md");
    }

    #[test]
    fn skips_links_inside_a_fenced_block() {
        assert!(links_in("before\n```\n[x](a.md)\n```\nafter").is_empty());
    }

    #[test]
    fn skips_links_inside_a_tilde_fenced_block() {
        assert!(links_in("~~~\n[x](a.md)\n~~~\n").is_empty());
    }

    #[test]
    fn skips_links_inside_an_indented_fenced_block() {
        // CONTRIBUTING.md fences code inside list items; a column-0-only check
        // would leave those blocks live.
        assert!(links_in("- item:\n\n  ```\n  [x](a.md)\n  ```\n").is_empty());
    }

    #[test]
    fn skips_links_inside_an_inline_code_span() {
        assert!(links_in("write `[x](a.md)` like so").is_empty());
    }

    #[test]
    fn finds_a_link_after_a_fenced_block_closes() {
        let ls = links_in("```\n[a](x.md)\n```\n[b](y.md)");
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].target, "y.md");
    }

    #[test]
    fn a_longer_fence_is_not_closed_by_a_shorter_one() {
        // The idiom for showing a fenced block inside a fenced block: a 4-backtick
        // outer fence wrapping 3-backtick inner ones. Treating the opener as exactly
        // "```" ends the block at the first inner fence, leaving the text between the
        // inner fences live — a false POSITIVE, the direction this module's own doc
        // comment promises never to produce.
        //
        // The link sits between the inner fences on purpose: put it after them and a
        // buggy scanner re-opens on the second inner fence and masks it anyway, so
        // the test would pass while the bug stood.
        let body = "````markdown\n```\nSee [x](gone.md).\n```\n````\n";
        assert!(links_in(body).is_empty(), "found {}", links_in(body).len());
    }

    #[test]
    fn a_closing_fence_may_be_longer_than_its_opener() {
        // CommonMark: the closer must be at LEAST as long as the opener. Guards the
        // above fix against over-tightening into an equality check.
        let ls = links_in("```\ncode\n`````\n[y](b.md)\n");
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].target, "b.md");
    }

    #[test]
    fn a_double_backtick_span_containing_a_backtick_is_masked() {
        // The way you write a literal backtick inline. Pairing single backticks ends
        // the span at the inner one and leaves the link live.
        assert!(links_in("write ``[x](a.md) ` here`` ok").is_empty());
    }

    // --- is_relative_target ---

    #[test]
    fn urls_and_anchors_are_not_relative_targets() {
        for t in ["https://e.com", "http://e.com", "mailto:a@b.c", "#section"] {
            assert!(!is_relative_target(t), "{t}");
        }
    }

    #[test]
    fn paths_are_relative_targets() {
        for t in ["a.md", "../a.md", "adr/", "a.md#frag"] {
            assert!(is_relative_target(t), "{t}");
        }
    }

    // --- line_at ---

    #[test]
    fn line_at_is_one_based() {
        assert_eq!(line_at("a\nb\nc", 0), 1);
        assert_eq!(line_at("a\nb\nc", 2), 2);
        assert_eq!(line_at("a\nb\nc", 4), 3);
    }

    // --- dead_links_in ---

    #[test]
    fn existing_target_is_not_dead() {
        let d = repo("alive");
        write(&d, "docs/a.md", "[x](b.md)\n");
        write(&d, "docs/b.md", "hi\n");
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    #[test]
    fn missing_target_is_dead_with_line_and_target() {
        let d = repo("dead");
        write(&d, "docs/a.md", "one\n[x](gone.md)\n");
        let found = dead_links_in(&d, "docs/a.md").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].target, "gone.md");
    }

    #[test]
    fn directory_target_resolves() {
        let d = repo("dir");
        std::fs::create_dir_all(d.join("docs/adr")).unwrap();
        write(&d, "docs/a.md", "[x](adr/)\n");
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    #[test]
    fn fragment_is_stripped_before_resolving() {
        let d = repo("frag");
        write(&d, "docs/a.md", "[x](b.md#sec)\n");
        write(&d, "docs/b.md", "hi\n");
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    #[test]
    fn urls_and_anchors_are_ignored() {
        let d = repo("urls");
        write(
            &d,
            "docs/a.md",
            "[x](https://e.com) [y](#s) [z](mailto:a@b.c)\n",
        );
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    #[test]
    fn dead_link_inside_a_fenced_block_is_ignored() {
        let d = repo("code-fence");
        write(&d, "docs/a.md", "```\n[y](gone.md)\n```\n");
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    #[test]
    fn dead_link_inside_an_inline_code_span_is_ignored() {
        let d = repo("code-span");
        write(&d, "docs/a.md", "`[x](gone.md)`\n");
        assert!(dead_links_in(&d, "docs/a.md").unwrap().is_empty());
    }

    // --- gated_files + problems ---

    #[test]
    fn gate_reports_a_dead_link_in_tracked_markdown() {
        let d = repo("gate");
        commit(&d, "docs/a.md", "[x](gone.md)\n");
        assert_eq!(
            problems(&d).unwrap(),
            vec!["docs/a.md:1 -> gone.md".to_string()]
        );
    }

    #[test]
    fn gate_skips_the_archive_tree() {
        let d = repo("excluded-archive");
        commit(&d, "docs/archive/old.md", "[x](gone.md)\n");
        assert!(problems(&d).unwrap().is_empty());
    }

    #[test]
    fn gate_skips_the_superpowers_tree() {
        let d = repo("excluded-superpowers");
        commit(&d, "docs/superpowers/plan.md", "[x](gone.md)\n");
        assert!(problems(&d).unwrap().is_empty());
    }

    #[test]
    fn gate_skips_untracked_files() {
        let d = repo("untracked");
        write(&d, "docs/loose.md", "[x](gone.md)\n"); // never `git add`ed
        assert!(problems(&d).unwrap().is_empty());
    }

    #[test]
    fn gate_skips_tracked_but_deleted_files() {
        let d = repo("deleted");
        commit(&d, "docs/a.md", "[x](gone.md)\n");
        std::fs::remove_file(d.join("docs/a.md")).unwrap();
        assert!(problems(&d).unwrap().is_empty());
    }

    #[test]
    fn gate_is_clean_when_every_link_resolves() {
        let d = repo("clean");
        commit(&d, "docs/b.md", "hi\n");
        commit(&d, "docs/a.md", "[x](b.md)\n");
        assert!(problems(&d).unwrap().is_empty());
    }
}
