//! ADR numbering commands.
//!
//! - `cargo xtask adr promote`: number tracked drafts in `docs/adr/drafts/`,
//!   graduating each into `docs/adr/NNNN-<slug>.md` and staging the complete
//!   source-to-destination promotion.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::adr_readme;
use crate::doc_links;
use crate::git;
use crate::ids;
use crate::result::StepResult;

const ADR_DIR: &str = "docs/adr";
const DRAFTS_DIR: &str = "docs/adr/drafts";

/// Four-digit zero-padded number, e.g. `34 -> "0034"`.
pub fn pad(n: u32) -> String {
    format!("{n:04}")
}

/// Replace every occurrence of `old_stem` with `new_stem`.
pub fn rewrite_stem(content: &str, old_stem: &str, new_stem: &str) -> String {
    content.replace(old_stem, new_stem)
}

/// Rewrite every inline link target in `body`, removing one leading `../`.
///
/// A draft moves up exactly one directory at promotion (`docs/adr/drafts/x.md` ->
/// `docs/adr/NNNN-x.md`), so each of its relative targets is off by exactly one
/// level. Stating that invariant directly covers more than a sibling-ADR-specific
/// rewrite would — `../template.md` is the shape `drafts/README.md` models for
/// authors, and it breaks the same way.
///
/// Only targets inside `](...)` are touched, and only outside code spans and fenced
/// blocks: a draft may legitimately discuss `../` in prose or show it in a shell
/// snippet, and a blanket string replace would corrupt those. Targets that cannot
/// lose a level (`..`, `../`, a bare name, a non-initial `../`) and non-relative
/// targets (URLs, anchors) are left alone.
pub fn strip_one_level(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut cursor = 0;
    for link in doc_links::links_in(body) {
        if !doc_links::is_relative_target(&link.target) {
            continue;
        }
        // `..` has no prefix to strip; `../` would strip to nothing.
        let Some(rest) = link.target.strip_prefix("../") else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        out.push_str(&body[cursor..link.span.start]);
        out.push_str(rest);
        cursor = link.span.end;
    }
    out.push_str(&body[cursor..]);
    out
}

/// Rewrite a `proposed` status token to `accepted` — the acceptance event that
/// promotion *is*, finally written down. `None` when there is no status line, or
/// when its token is anything else.
///
/// Only `proposed` moves. `superseded`, `rejected` and `deprecated` on a draft are
/// deliberate authorial statements — an ADR written to record a reversal, or to
/// document a decision already dead — and promotion must not overwrite an author's
/// explicit claim with a default. The guard is whole-remainder equality, so a
/// multi-token status (`proposed (pending #742)`) is left alone too: it is
/// malformed, and `adr-format` should say so on a stable tree rather than have
/// promotion half-fix it.
///
/// The edit is confined to the status line's byte span, taken from the one shared
/// [`adr_readme::status_line`] parse, so prose elsewhere in the draft that happens
/// to contain the word "proposed" is untouched.
pub(crate) fn accept_proposed_status(body: &str) -> Option<String> {
    let status = adr_readme::status_line(body)?;
    if status.rest != adr_readme::PROPOSED {
        return None;
    }
    let line = &body[status.span.clone()];
    Some(format!(
        "{}{}{}",
        &body[..status.span.start],
        line.replacen(adr_readme::PROPOSED, adr_readme::ACCEPTED, 1),
        &body[status.span.end..]
    ))
}

/// ADR filenames currently in `repo`'s `docs/adr`.
fn adr_filenames(repo: &Path) -> Result<Vec<String>> {
    regular_file_names(&repo.join(ADR_DIR))
}

fn regular_file_names(dir: &Path) -> Result<Vec<String>> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    regular_file_names_from(
        dir,
        entries,
        std::fs::DirEntry::path,
        std::fs::DirEntry::file_type,
        |entry| entry.file_name(),
    )
}

fn regular_file_names_from<T>(
    dir: &Path,
    entries: impl IntoIterator<Item = std::io::Result<T>>,
    path_of: impl Fn(&T) -> PathBuf,
    file_type: impl Fn(&T) -> std::io::Result<std::fs::FileType>,
    file_name: impl Fn(T) -> OsString,
) -> Result<Vec<String>> {
    entries
        .into_iter()
        .map(|entry| entry.with_context(|| format!("reading entry under {}", dir.display())))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| {
            let path = path_of(&entry);
            let is_file = file_type(&entry)
                .with_context(|| format!("reading file type {}", path.display()))?
                .is_file();
            Ok(is_file.then(|| file_name(entry).to_string_lossy().into_owned()))
        })
        .collect::<Result<Vec<_>>>()
        .map(|names| names.into_iter().flatten().collect())
}

/// Read `rel` under `repo`, apply `f`, and write it back only if it changed.
fn rewrite_file(repo: &Path, rel: &str, f: impl Fn(&str) -> String) -> Result<()> {
    let path: PathBuf = repo.join(rel);
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let updated = f(&content);
    if updated != content {
        std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// One draft's graduation, threaded through promote's three passes: Pass A assigns
/// the number, Pass B writes the file and records whether it accepted the status,
/// Pass C rewrites references and reports. One value rather than a tuple plus a
/// side-table, so a pass cannot read the number from one place and the acceptance
/// flag from another that has drifted.
struct Promotion {
    slug: String,
    num: u32,
    /// `NNNN-<slug>.md` — the filename the draft graduates into.
    new_name: String,
    /// Whether Pass B rewrote `proposed` -> `accepted` in this file.
    accepted: bool,
}

/// Slugs of the draft ADRs under `repo`'s `docs/adr/drafts`, sorted for a
/// deterministic assignment order. The tracked `README.md` explainer and any
/// non-`.md` entry are skipped; `<slug>.md` yields `slug`.
fn draft_slugs(repo: &Path) -> Result<Vec<String>> {
    let mut slugs: Vec<String> = regular_file_names(&repo.join(DRAFTS_DIR))?
        .into_iter()
        .filter(|name| name != "README.md")
        .filter_map(|name| name.strip_suffix(".md").map(str::to_string))
        .collect();
    slugs.sort();
    Ok(slugs)
}

/// Entry point for `cargo xtask adr promote`: operate on the current repo.
pub fn promote() -> StepResult {
    match run_promote(Path::new(".")) {
        Ok(summary) => StepResult::ok("adr-promote").detail(summary),
        Err(e) => StepResult::fail("adr-promote").detail(format!("{e:#}")),
    }
}

/// Promote the required first-line draft heading token and leave the body intact.
///
/// This deliberately does not use a whole-body replacement: ADRs about the draft
/// workflow may discuss the literal `ADR-DRAFT` token in prose or code spans.
fn promote_heading(body: &str, number: u32, draft_rel: &str) -> Result<String> {
    let required = "# ADR-DRAFT: ";
    let Some(rest) = body.strip_prefix(required) else {
        bail!("{draft_rel} must start with `{required}` and a non-empty title");
    };
    let title = rest.split_once('\n').map_or(rest, |(line, _)| line);
    if title.trim().is_empty() {
        bail!("{draft_rel} must start with `{required}` and a non-empty title");
    }
    Ok(format!("# ADR-{}: {rest}", pad(number)))
}

/// Number every tracked draft in `docs/adr/drafts`, graduate it into
/// `docs/adr/NNNN-<slug>.md`, record its acceptance in the status line, rewrite its
/// path-form references, sync the README table, and stage the complete result.
///
/// A draft has no ADR number before promotion. Its tracked path is moved with
/// `git mv`, then the destination content and every projection are rewritten and
/// staged. There is no bare `ADR-NNNN` form to rewrite, since a draft is
/// referenced only by its `drafts/<slug>` path.
pub(crate) fn run_promote(repo: &Path) -> Result<String> {
    let slugs = draft_slugs(repo)?;
    if slugs.is_empty() {
        return Ok("no ADR drafts to promote".to_string());
    }

    // Pass A — assign every draft a number before rewriting anything, so a draft
    // that references another draft can resolve to the assigned number. `all`
    // grows with each assignment.
    let mut all = adr_filenames(repo)?;
    let mut assigned: Vec<Promotion> = Vec::new();
    for slug in &slugs {
        let num = ids::next_number(&all);
        let new_name = format!("{}-{slug}.md", pad(num));
        all.push(new_name.clone());
        assigned.push(Promotion {
            slug: slug.clone(),
            num,
            new_name,
            accepted: false,
        });
    }

    let mut promoted_bodies: Vec<(String, String)> = Vec::new();
    for p in &assigned {
        let draft_rel = format!("{DRAFTS_DIR}/{}.md", p.slug);
        let body = std::fs::read_to_string(repo.join(&draft_rel))
            .with_context(|| format!("reading {draft_rel}"))?;
        let numbered = promote_heading(&body, p.num, &draft_rel)?;
        promoted_bodies.push((draft_rel, numbered));
    }

    // Feature pull requests commit drafts before promotion. Validate the whole
    // input set before moving any source so an accidentally untracked draft cannot
    // leave an earlier draft half-promoted.
    let tracked = git::ls_files_md(repo)?;
    for (draft_rel, _) in &promoted_bodies {
        if !tracked.contains(draft_rel) {
            bail!("{draft_rel} must be tracked before promotion");
        }
    }

    // Pass B — move each tracked draft in the index, then graduate its heading
    // and status at the destination. Staging the rewritten destination makes the
    // path-form rewrite below see cross-references between graduated drafts.
    for (p, (draft_rel, numbered)) in assigned.iter_mut().zip(promoted_bodies) {
        let new_rel = format!("{ADR_DIR}/{}", p.new_name);
        // The file moves up one directory here, so its own relative links are
        // rewritten at the same moment — not after Pass C, which would see targets
        // that have already been rewritten to their assigned numbers.
        let relinked = strip_one_level(&numbered);
        // Numbering is the acceptance event; record it in the status line. The flag
        // rides on the promotion itself, so Pass C — which owns the summary — cannot
        // drift out of step with what Pass B actually wrote.
        let graduated = match accept_proposed_status(&relinked) {
            Some(accepted) => {
                p.accepted = true;
                accepted
            }
            None => relinked,
        };
        git::mv(repo, &draft_rel, &new_rel)?;
        std::fs::write(repo.join(&new_rel), graduated)
            .with_context(|| format!("writing {new_rel}"))?;
        git::add(repo, &new_rel)?;
    }

    // Pass C — rewrite path-form references repo-wide. `drafts/<slug>` carries the
    // slug, so it is unambiguous. The graduated files are staged (tracked), so a
    // draft-to-draft reference is rewritten too.
    let mut summary = Vec::new();
    for p in &assigned {
        let draft_stem = format!("drafts/{}", p.slug);
        let new_stem = format!("{}-{}", pad(p.num), p.slug);
        for file in git::grep_files(repo, &draft_stem)? {
            rewrite_file(repo, &file, |c| rewrite_stem(c, &draft_stem, &new_stem))?;
            git::add(repo, &file)?;
        }
        let status_note = if p.accepted {
            format!(
                " (status: {} -> {})",
                adr_readme::PROPOSED,
                adr_readme::ACCEPTED
            )
        } else {
            String::new()
        };
        summary.push(format!(
            "{DRAFTS_DIR}/{}.md -> {ADR_DIR}/{}{status_note}",
            p.slug, p.new_name
        ));
    }

    // Keep the README table in lockstep: each graduated ADR adds a row, seeded
    // from its heading. Tolerate a markerless README (a scratch/test repo).
    let table_note = if crate::adr_readme::readme_has_markers(repo)? {
        let note = crate::adr_readme::sync_readme_at(repo)?;
        git::add(repo, crate::adr_readme::README)?;
        format!("README table synced ({note})")
    } else {
        "README table not synced (no adr-table markers)".to_string()
    };

    // Check the graduated files in their FINAL form — after Pass C's reference
    // rewrite and the README sync — so a link Pass C is about to fix is never
    // reported. Warn rather than fail: the tree is already mutated and staged, so
    // bailing here would leave a half-promoted checkout. `doc-links` turns the same
    // condition into a hard failure at the next commit, on a stable re-runnable tree.
    let mut warnings = Vec::new();
    for p in &assigned {
        let rel = format!("{ADR_DIR}/{}", p.new_name);
        // `?` here would be the very failure the comment above rules out — the file
        // is unreadable only after promote has already written and staged it, so
        // bailing would abandon a half-promoted tree. Report it as a warning instead.
        let dead = match doc_links::dead_links_in(repo, &rel) {
            Ok(dead) => dead,
            Err(e) => {
                warnings.push(format!("{rel}: unreadable ({e:#})"));
                continue;
            }
        };
        if !dead.is_empty() {
            let targets: Vec<String> = dead.into_iter().map(|d| d.target).collect();
            warnings.push(format!("{rel}: {}", targets.join(", ")));
        }
    }
    let warn_note = if warnings.is_empty() {
        String::new()
    } else {
        format!("; warning: unresolved link(s) — {}", warnings.join("; "))
    };

    Ok(format!(
        "{} — {table_note}; staged{warn_note}",
        summary.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{git_ok as git, write};
    use std::path::Path;

    #[test]
    fn pad_is_four_digits() {
        assert_eq!(pad(34), "0034");
        assert_eq!(pad(5), "0005");
    }

    #[test]
    fn rewrite_stem_replaces_path_form_refs() {
        let content = "See [the ADR](docs/adr/0034-bar.md) and 0034-bar.md again.";
        let out = rewrite_stem(content, "0034-bar", "0035-bar");
        assert_eq!(
            out,
            "See [the ADR](docs/adr/0035-bar.md) and 0035-bar.md again."
        );
    }

    #[test]
    fn strip_one_level_drops_a_single_leading_parent() {
        assert_eq!(strip_one_level("[x](../0001-foo.md)"), "[x](0001-foo.md)");
    }

    #[test]
    fn strip_one_level_drops_only_one_of_two() {
        assert_eq!(
            strip_one_level("[x](../../CONTRIBUTING.md)"),
            "[x](../CONTRIBUTING.md)"
        );
    }

    #[test]
    fn strip_one_level_leaves_bare_targets_alone() {
        assert_eq!(strip_one_level("[x](template.md)"), "[x](template.md)");
    }

    #[test]
    fn strip_one_level_leaves_dot_dot_edge_cases_alone() {
        assert_eq!(strip_one_level("[x](..)"), "[x](..)");
        assert_eq!(strip_one_level("[x](../)"), "[x](../)");
        assert_eq!(strip_one_level("[x](a/../b.md)"), "[x](a/../b.md)");
    }

    #[test]
    fn strip_one_level_ignores_urls_and_anchors() {
        let body = "[x](https://e.com/../a) [y](#s)";
        assert_eq!(strip_one_level(body), body);
    }

    #[test]
    fn strip_one_level_spares_links_inside_code() {
        // Real `](...)` links, so this fails against any implementation that
        // rewrites targets without honouring the code carve-out.
        let body = "prose ../foo\n\n```\n[a](../x.md)\n```\n\n`[b](../y.md)`\n";
        assert_eq!(strip_one_level(body), body);
    }

    #[test]
    fn strip_one_level_rewrites_every_link_in_one_pass() {
        assert_eq!(
            strip_one_level("[a](../x.md) and [b](../y.md)"),
            "[a](x.md) and [b](y.md)"
        );
    }

    #[test]
    fn promote_strips_one_level_from_sibling_links() {
        let tmp = promote_repo("strip-sibling");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\nSee [foo](../0001-foo.md).\n",
        );
        promote_tracked(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("](0001-foo.md)"), "body: {body}");
        assert!(!body.contains("](../0001-foo.md)"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_strips_one_level_from_non_adr_links() {
        let tmp = promote_repo("strip-general");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n[t](../template.md) [c](../../CONTRIBUTING.md)\n",
        );
        promote_tracked(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("](template.md)"), "body: {body}");
        assert!(body.contains("](../CONTRIBUTING.md)"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_rewrites_only_the_heading_token() {
        let tmp = promote_repo("heading-only");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\nThe literal `ADR-DRAFT` token is documented here.\n\n```text\nADR-DRAFT\n```\n",
        );

        promote_tracked(&tmp).unwrap();

        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.starts_with("# ADR-0002: D\n"), "body: {body}");
        assert!(
            body.contains("The literal `ADR-DRAFT` token is documented here."),
            "body: {body}"
        );
        assert!(body.contains("```text\nADR-DRAFT\n```"), "body: {body}");
        assert!(!body.contains("`ADR-0002`"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_rejects_malformed_heading_before_mutating_any_draft() {
        let tmp = promote_repo("bad-heading");
        let valid = "# ADR-DRAFT: Aaa\n";
        let malformed = "# ADR-DRAFT:   \n";
        write(&tmp, "docs/adr/drafts/aaa.md", valid);
        write(&tmp, "docs/adr/drafts/bbb.md", malformed);

        let err = promote_tracked(&tmp).unwrap_err();
        let message = format!("{err:#}");

        assert!(
            message.contains("docs/adr/drafts/bbb.md"),
            "error should name malformed draft: {message}"
        );
        assert!(
            message.contains("non-empty title"),
            "error should require a non-empty title: {message}"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.join("docs/adr/drafts/aaa.md")).unwrap(),
            valid
        );
        assert_eq!(
            std::fs::read_to_string(tmp.join("docs/adr/drafts/bbb.md")).unwrap(),
            malformed
        );
        assert!(!tmp.join("docs/adr/0002-aaa.md").exists());
        assert!(!tmp.join("docs/adr/0003-bbb.md").exists());

        let unstaged = git_stdout(&tmp, &["diff", "--name-only"]);
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        assert!(unstaged.trim().is_empty(), "unstaged diff: {unstaged}");
        assert!(staged.trim().is_empty(), "staged diff: {staged}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Assert on the `warning:` clause, never on the whole summary: Pass C already
    // pushes `drafts/<slug>.md -> docs/adr/NNNN-<slug>.md` into it, so
    // `summary.contains("0002-d.md")` is true no matter what this code does.

    #[test]
    fn promote_warns_on_a_surviving_dead_link() {
        let tmp = promote_repo("warn");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\nSee [gone](nonexistent.md).\n",
        );
        let summary = promote_tracked(&tmp).unwrap(); // Ok, not Err — promote still graduates
        assert!(
            tmp.join("docs/adr/0002-d.md").exists(),
            "file still written"
        );
        assert!(
            summary.contains("warning: unresolved link(s) — docs/adr/0002-d.md: nonexistent.md"),
            "summary: {summary}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_is_silent_when_every_link_resolves() {
        let tmp = promote_repo("no-warn");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\nSee [foo](../0001-foo.md).\n",
        );
        let summary = promote_tracked(&tmp).unwrap();
        assert!(!summary.contains("warning"), "summary: {summary}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tracked_draft_links_and_citations_resolve_before_and_after_promotion() {
        let tmp = promote_repo("tracked-links");
        write(&tmp, "docs/notes.md", "See [Bbb](adr/drafts/bbb.md).\n");
        write(&tmp, "docs/adr/drafts/aaa.md", "# ADR-DRAFT: Aaa\n");
        write(
            &tmp,
            "docs/adr/drafts/bbb.md",
            "# ADR-DRAFT: Bbb\n\n\
             Builds on [the numbered ADR](../0001-foo.md) and [aaa](../drafts/aaa.md).\n",
        );
        git(&tmp, &["add", "docs/notes.md", DRAFTS_DIR]);
        git(&tmp, &["commit", "-qm", "feature: linked ADR drafts"]);

        let before = crate::doc_links::problems(&tmp).unwrap();
        assert!(before.is_empty(), "draft links must resolve: {before:?}");

        let summary = run_promote(&tmp).unwrap();

        assert!(!summary.contains("warning"), "summary: {summary}");
        let bbb = std::fs::read_to_string(tmp.join("docs/adr/0003-bbb.md")).unwrap();
        assert!(bbb.contains("](0001-foo.md)"), "bbb: {bbb}");
        assert!(bbb.contains("](0002-aaa.md)"), "bbb: {bbb}");
        let notes = std::fs::read_to_string(tmp.join("docs/notes.md")).unwrap();
        assert_eq!(notes, "See [Bbb](adr/0003-bbb.md).\n");
        let after = crate::doc_links::problems(&tmp).unwrap();
        assert!(after.is_empty(), "promoted links must resolve: {after:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn a_promoted_template_draft_passes_adr_format() {
        // The composition test. The status rewrite and the `adr-format` gate are
        // each green in isolation while disagreeing about which line is the status
        // line, so only running them end-to-end proves they agree.
        //
        // Teeth: drop the `accept_proposed_status` call from Pass B and THIS fails — the
        // promoted file is still `proposed`, which the gate rejects — where the
        // unit tests would merely report their own narrower loss.
        let tmp = promote_repo("round-trip");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: proposed\n- Date: 2026-07-31\n",
        );
        promote_tracked(&tmp).unwrap();
        let problems = crate::adr_readme::format_problems(&tmp);
        assert!(
            problems.is_empty(),
            "promoted tree must be clean: {problems:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_sets_accepted_on_a_proposed_draft() {
        let tmp = promote_repo("status-proposed");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: proposed\n",
        );
        promote_tracked(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("- Status: accepted"), "body: {body}");
        assert!(!body.contains("proposed"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_preserves_a_deliberate_status() {
        // Every non-`proposed` token on a draft is an authorial statement, not the
        // template default — promotion must not flatten any of them to `accepted`.
        // All three are covered because the rustdoc names all three.
        for (tag, token) in [
            ("sup", "superseded"),
            ("rej", "rejected"),
            ("dep", "deprecated"),
        ] {
            let tmp = promote_repo(tag);
            let draft = format!("# ADR-DRAFT: D\n\n- Status: {token}\n");
            write(&tmp, "docs/adr/drafts/d.md", &draft);
            promote_tracked(&tmp).unwrap();
            let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
            assert!(body.contains(&format!("- Status: {token}")), "body: {body}");
            assert!(!body.contains("accepted"), "{token} body: {body}");
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }

    #[test]
    fn promote_rewrites_an_indented_status_line() {
        // The discriminating case for the shared `status_line` parse: an
        // implementation that matched a column-0 `- Status:` literal passes
        // `promote_sets_accepted_on_a_proposed_draft` and fails here, leaving a
        // promoted ADR that `adr-format` immediately rejects.
        let tmp = promote_repo("status-indented");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n  - Status: proposed\n",
        );
        promote_tracked(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("  - Status: accepted"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_leaves_the_word_proposed_in_prose() {
        // Token-scoped and line-anchored: a whole-body `replace("proposed", …)`
        // passes every other test in this group and corrupts the prose here.
        let tmp = promote_repo("status-prose");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: proposed\n\nWe proposed X in an earlier cycle.\n",
        );
        promote_tracked(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("- Status: accepted"), "body: {body}");
        assert!(
            body.contains("We proposed X in an earlier cycle."),
            "prose must survive: {body}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_summary_names_the_status_transition() {
        // Assert the `(status: …)` clause ALONE — never the whole summary. Pass C
        // always pushes the path pair in (see the standing warning above), so a
        // `summary.contains("0002-d.md")` assertion passes no matter what the
        // status code does.
        let tmp = promote_repo("status-summary");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: proposed\n",
        );
        let summary = promote_tracked(&tmp).unwrap();
        assert!(
            summary.contains("(status: proposed -> accepted)"),
            "summary: {summary}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_summary_is_silent_for_an_already_accepted_draft() {
        let tmp = promote_repo("status-silent");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: accepted\n",
        );
        let summary = promote_tracked(&tmp).unwrap();
        assert!(!summary.contains("status:"), "summary: {summary}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Trimmed stdout of a git command that must succeed — for asserting index
    /// state (`diff --cached`).
    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let out = crate::git::at(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Commit every draft before promotion so behavior tests exercise the tracked
    /// source lifecycle used by feature pull requests.
    fn promote_tracked(repo: &Path) -> Result<String> {
        if !draft_slugs(repo)?.is_empty() {
            git(repo, &["add", DRAFTS_DIR]);
            git(repo, &["commit", "-qm", "feature: ADR drafts"]);
        }
        run_promote(repo)
    }

    /// A committed repo with `docs/adr/0001-foo.md` on `main` — the base state the
    /// promote tests graduate a draft on top of.
    fn promote_repo(tag: &str) -> std::path::PathBuf {
        let tmp = crate::test_support::temp_repo("adr-promote", tag);
        write(
            &tmp,
            "docs/adr/0001-foo.md",
            "# ADR-0001: Foo\n\n- Status: accepted\n",
        );
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "main: 0001-foo"]);
        tmp
    }

    #[test]
    fn promote_numbers_single_draft() {
        let tmp = promote_repo("single");
        // A README with the table so the row-sync path is exercised.
        write(
            &tmp,
            "docs/README.md",
            "# Docs\n\n<!-- adr-table:begin -->\n\n\
             | #   | Title | Status |\n| --- | ----- | ------ |\n\
             | [0001](adr/0001-foo.md) | Foo | accepted |\n\n\
             <!-- adr-table:end -->\n",
        );
        git(&tmp, &["add", "docs/README.md"]);
        git(&tmp, &["commit", "-qm", "main: README table"]);

        // A tracked, numberless draft.
        write(
            &tmp,
            "docs/adr/drafts/my-decision.md",
            "# ADR-DRAFT: My Decision\n\n- Status: proposed\n",
        );

        let summary = promote_tracked(&tmp).unwrap();
        assert!(
            summary.contains("docs/adr/drafts/my-decision.md -> docs/adr/0002-my-decision.md"),
            "summary: {summary}"
        );

        // Graduated to the next free number; draft gone; heading token rewritten.
        assert!(!tmp.join("docs/adr/drafts/my-decision.md").exists());
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-my-decision.md")).unwrap();
        assert!(body.contains("# ADR-0002: My Decision"), "body: {body}");
        assert!(!body.contains("ADR-DRAFT"));

        // README row added, seeded from the heading.
        let readme = std::fs::read_to_string(tmp.join("docs/README.md")).unwrap();
        assert!(
            readme.contains("[0002](adr/0002-my-decision.md)"),
            "readme: {readme}"
        );
        assert!(readme.contains("| My Decision |"), "seeded title");
        // The generated status cell reflects the acceptance the promotion just
        // recorded. This is the only coverage of a *newly promoted* ADR's README
        // status cell — `promote_repo` writes no README, so the round-trip test
        // takes promote's markerless branch and never renders a row at all.
        assert!(
            readme.contains("| My Decision | accepted |"),
            "readme: {readme}"
        );

        // The complete tracked-source rename and the README rewrite are staged,
        // ready to commit.
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-status", "--no-renames"]);
        assert!(
            staged.contains("D\tdocs/adr/drafts/my-decision.md"),
            "staged: {staged}"
        );
        assert!(
            staged.contains("A\tdocs/adr/0002-my-decision.md"),
            "staged: {staged}"
        );
        assert!(staged.contains("M\tdocs/README.md"), "staged: {staged}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_assigns_distinct_numbers_to_multiple_drafts() {
        let tmp = promote_repo("multi");
        write(
            &tmp,
            "docs/adr/drafts/aaa.md",
            "# ADR-DRAFT: Aaa\n\n- Status: proposed\n",
        );
        write(
            &tmp,
            "docs/adr/drafts/bbb.md",
            "# ADR-DRAFT: Bbb\n\n- Status: proposed\n",
        );

        promote_tracked(&tmp).unwrap();

        // Sorted (aaa before bbb) → consecutive numbers; both drafts consumed.
        assert!(tmp.join("docs/adr/0002-aaa.md").exists());
        assert!(tmp.join("docs/adr/0003-bbb.md").exists());
        assert!(!tmp.join("docs/adr/drafts/aaa.md").exists());
        assert!(!tmp.join("docs/adr/drafts/bbb.md").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_rewrites_path_form_references() {
        let tmp = promote_repo("path-refs");
        // A tracked file referencing the draft by path — only tracked files are
        // visible to `git grep`.
        write(
            &tmp,
            "docs/notes.md",
            "Decided in docs/adr/drafts/my-decision.md.\n",
        );
        git(&tmp, &["add", "docs/notes.md"]);
        git(&tmp, &["commit", "-qm", "main: notes"]);

        write(
            &tmp,
            "docs/adr/drafts/my-decision.md",
            "# ADR-DRAFT: My Decision\n\n- Status: proposed\n",
        );

        promote_tracked(&tmp).unwrap();

        let notes = std::fs::read_to_string(tmp.join("docs/notes.md")).unwrap();
        assert_eq!(notes, "Decided in docs/adr/0002-my-decision.md.\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_resolves_draft_referencing_another_draft() {
        let tmp = promote_repo("draft-ref-draft");
        write(
            &tmp,
            "docs/adr/drafts/aaa.md",
            "# ADR-DRAFT: Aaa\n\n- Status: proposed\n",
        );
        // Draft bbb references draft aaa by path; the rewrite must reach the
        // graduated (now-tracked) file, not just pre-existing committed files.
        write(
            &tmp,
            "docs/adr/drafts/bbb.md",
            "# ADR-DRAFT: Bbb\n\nBuilds on docs/adr/drafts/aaa.md.\n",
        );

        promote_tracked(&tmp).unwrap();

        let bbb = std::fs::read_to_string(tmp.join("docs/adr/0003-bbb.md")).unwrap();
        assert!(
            bbb.contains("docs/adr/0002-aaa.md"),
            "cross-reference rewritten: {bbb}"
        );
        assert!(!bbb.contains("drafts/aaa"), "no stale draft path: {bbb}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_keeps_the_tracked_drafts_readme() {
        let tmp = promote_repo("draft-readme");
        write(&tmp, "docs/adr/drafts/README.md", "# ADR drafts\n");
        git(&tmp, &["add", "docs/adr/drafts/README.md"]);
        git(&tmp, &["commit", "-qm", "docs: draft explainer"]);

        let summary = run_promote(&tmp).unwrap();

        assert_eq!(summary, "no ADR drafts to promote");
        assert!(tmp.join("docs/adr/drafts/README.md").exists());
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        let unstaged = git_stdout(&tmp, &["diff", "--name-only"]);
        assert!(staged.is_empty(), "staged: {staged}");
        assert!(unstaged.is_empty(), "unstaged: {unstaged}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_is_noop_without_drafts() {
        let tmp = promote_repo("noop");
        std::fs::create_dir_all(tmp.join(DRAFTS_DIR)).unwrap();
        let summary = promote_tracked(&tmp).unwrap();
        assert_eq!(summary, "no ADR drafts to promote");
        // Nothing staged.
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        assert!(staged.is_empty(), "staged: {staged}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_rerun_after_committing_is_a_clean_noop() {
        let tmp = promote_repo("clean-rerun");
        write(
            &tmp,
            "docs/adr/drafts/d.md",
            "# ADR-DRAFT: D\n\n- Status: proposed\n",
        );
        promote_tracked(&tmp).unwrap();
        git(&tmp, &["commit", "-qm", "promote ADR"]);

        let summary = run_promote(&tmp).unwrap();

        assert_eq!(summary, "no ADR drafts to promote");
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        let unstaged = git_stdout(&tmp, &["diff", "--name-only"]);
        assert!(staged.is_empty(), "staged: {staged}");
        assert!(unstaged.is_empty(), "unstaged: {unstaged}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_picks_next_after_committed_adr() {
        // A branch that already committed a higher-numbered ADR: the draft must
        // pick up after it, not collide with the base `0001`.
        let tmp = promote_repo("after-committed");
        write(
            &tmp,
            "docs/adr/0005-x.md",
            "# ADR-0005: X\n\n- Status: accepted\n",
        );
        git(&tmp, &["add", "docs/adr/0005-x.md"]);
        git(&tmp, &["commit", "-qm", "branch: 0005-x"]);

        write(
            &tmp,
            "docs/adr/drafts/later.md",
            "# ADR-DRAFT: Later\n\n- Status: proposed\n",
        );

        promote_tracked(&tmp).unwrap();

        assert!(tmp.join("docs/adr/0006-later.md").exists());
        assert!(!tmp.join("docs/adr/drafts/later.md").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn adr_filenames_skips_the_drafts_subdir() {
        // The promotion base set must not see draft entries: `adr_filenames` is
        // non-recursive and file-only, so the `docs/adr/drafts/` subdirectory
        // (and anything inside it) is excluded — the same rule that keeps a
        // numberless draft invisible to the ADR gates (#219).
        let tmp =
            std::env::temp_dir().join(format!("jaunder-adr-drafts-vis-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("docs/adr/drafts")).unwrap();
        write(&tmp, "docs/adr/0001-a.md", "# ADR-0001: A\n");
        write(
            &tmp,
            "docs/adr/drafts/some-decision.md",
            "# ADR-DRAFT: Some\n",
        );

        let names = adr_filenames(&tmp).unwrap();
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(names, vec!["0001-a.md".to_string()]);
    }

    #[test]
    fn fail_closed_population_missing_adr_and_draft_directories() {
        let repo = Path::new("missing-adr-enumeration-root");
        for error in [
            adr_filenames(repo).unwrap_err(),
            draft_slugs(repo).unwrap_err(),
        ] {
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind),
                Some(std::io::ErrorKind::NotFound)
            );
        }
    }

    #[test]
    fn fail_closed_population_unreadable_adr_file_type() {
        struct Fake {
            path: PathBuf,
            name: OsString,
        }
        let dir = Path::new("docs/adr");
        let error = regular_file_names_from(
            dir,
            [Ok(Fake {
                path: dir.join("0001-unreadable.md"),
                name: OsString::from("0001-unreadable.md"),
            })],
            |entry| entry.path.clone(),
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected",
                ))
            },
            |entry| entry.name,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("0001-unreadable.md"));
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }
}
