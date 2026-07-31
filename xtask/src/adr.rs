//! ADR numbering commands.
//!
//! - `cargo xtask adr promote`: number the numberless drafts in
//!   `docs/adr/drafts/` at ship, graduating each into `docs/adr/NNNN-<slug>.md`.
//!   The number is assigned as late as possible, so the ADR's first appearance
//!   in git history is already collision-free (issue #219).
//! - `cargo xtask adr renumber`: resolve an ADR number collision by bumping the
//!   branch's newly-added ADR to the next free number and rewriting references.
//!   The ADR already reachable from `origin/main` is immutable; only branch
//!   additions move. Path-form references (which carry the slug) are rewritten
//!   repo-wide; bare `ADR-NNNN` references are rewritten only in branch-touched
//!   files, so `main`'s references to the other number are never clobbered.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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

/// Replace the leading number of `filename`, preserving the separator, slug, and
/// extension: `replace_number("0034-bar.md", 35) -> "0035-bar.md"`.
pub fn replace_number(filename: &str, new: u32) -> String {
    let rest = filename.trim_start_matches(|c: char| c.is_ascii_digit());
    format!("{}{rest}", pad(new))
}

/// Replace every occurrence of `old_stem` with `new_stem`. The stem carries the
/// slug (`0034-bar`), so it is unambiguous and safe to rewrite repo-wide.
///
/// This is a plain substring replace, which assumes ADR slugs are unique and not
/// prefixes of one another (e.g. no `0034-bar` alongside `0034-bartender`). That
/// holds because a collision is on the *number*, and the slugs of two
/// same-numbered ADRs are written by different authors for different decisions —
/// a shared prefix would be a coincidence, and even then only the over-matched
/// reference (not the file) would be affected, which the duplicate-prefix check
/// would still surface. Worth tightening to a boundary match if that ever bites.
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

/// Rewrite a `proposed` status token to `accepted`, in place — the acceptance event
/// that promotion *is*, finally written down. `None` when there is no status line or
/// its token is something else.
///
/// Only `proposed` moves. `superseded`, `rejected` and `deprecated` on a draft are
/// deliberate authorial statements — an ADR written to record a reversal, or to
/// document a decision already dead — and promotion must not overwrite an author's
/// explicit claim with a default.
///
/// The edit is scoped to the status line's byte span (located via the one shared
/// [`adr_readme::status_line`] parse) and replaces only the token within it, so the
/// line's indentation, prefix and any trailing content survive, and prose elsewhere
/// in the draft that happens to contain the word "proposed" is untouched.
pub fn rewrite_status(body: &str) -> Option<String> {
    let (index, rest) = adr_readme::status_line(body)?;
    if rest != "proposed" {
        return None;
    }
    // Byte span of the target line, excluding its terminator. `split_inclusive`
    // counts lines exactly as `status_line`'s `lines()` did, so `index` lines up.
    let start: usize = body.split_inclusive('\n').take(index).map(str::len).sum();
    let len = body[start..].find('\n').unwrap_or(body.len() - start);
    let line = &body[start..start + len];
    Some(format!(
        "{}{}{}",
        &body[..start],
        line.replacen("proposed", "accepted", 1),
        &body[start + len..]
    ))
}

/// Replace bare `ADR-NNNN` references for `old` -> `new`. The padded `ADR-` prefix
/// keeps `10034`-style substrings from matching. The caller scopes this to
/// branch-touched files because the bare form lacks a slug.
pub fn rewrite_bare(content: &str, old: u32, new: u32) -> String {
    content.replace(&format!("ADR-{}", pad(old)), &format!("ADR-{}", pad(new)))
}

/// Filename without its final extension: `0034-bar.md` -> `0034-bar`.
fn stem(filename: &str) -> &str {
    filename.rsplit_once('.').map_or(filename, |(s, _)| s)
}

/// Entry point for `cargo xtask adr renumber`: operate on the current repo
/// against `origin/main`.
pub fn renumber() -> StepResult {
    match run_renumber(Path::new("."), "origin/main") {
        Ok(summary) => StepResult::ok("adr-renumber").detail(summary),
        Err(e) => StepResult::fail("adr-renumber").detail(format!("{e:#}")),
    }
}

/// ADR filenames currently in `repo`'s `docs/adr`.
fn adr_filenames(repo: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(repo.join(ADR_DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
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

/// Bump each colliding branch-added ADR to the next free number and rewrite
/// references. `main_ref` is the integration branch (`origin/main` in practice;
/// a local `main` in tests). Returns a human summary of the moves.
fn run_renumber(repo: &Path, main_ref: &str) -> Result<String> {
    let base = git::merge_base(repo, main_ref, "HEAD").context("finding merge-base with main")?;
    let range = format!("{base}..HEAD");

    // ADR files this branch ADDED (filenames only).
    let added: Vec<String> = git::diff_added(repo, &range, ADR_DIR)?
        .into_iter()
        .filter_map(|p| p.rsplit('/').next().map(str::to_string))
        .collect();

    // Files this branch touched at all — the scope for bare-ref rewrites.
    let touched: Vec<String> = git::diff_names(repo, &range)?;

    let mut all = adr_filenames(repo);
    let mut summary = Vec::new();

    for added_name in &added {
        let Some(num) = ids::leading_number(added_name) else {
            continue;
        };
        // Collision iff another ADR in the working tree shares this number.
        let collides = all
            .iter()
            .filter(|n| ids::leading_number(n) == Some(num))
            .count()
            > 1;
        if !collides {
            continue;
        }

        let new_num = ids::next_number(&all);
        let new_name = replace_number(added_name, new_num);
        let old_stem = stem(added_name).to_string();
        let new_stem = stem(&new_name).to_string();
        let old_rel = format!("{ADR_DIR}/{added_name}");
        let new_rel = format!("{ADR_DIR}/{new_name}");

        // 1. Move the colliding newcomer.
        git::mv(repo, &old_rel, &new_rel)?;

        // 2. Path-form (slug-bearing) refs: rewrite repo-wide.
        for file in git::grep_files(repo, &old_stem)? {
            rewrite_file(repo, &file, |c| rewrite_stem(c, &old_stem, &new_stem))?;
        }

        // 3. Bare `ADR-NNNN` refs: rewrite only in branch-touched files (the moved
        //    ADR's own content counts — match its old and new paths too).
        let bare_token = format!("ADR-{}", pad(num));
        for file in git::grep_files(repo, &bare_token)? {
            let touched_by_branch =
                touched.iter().any(|t| t == &file) || file == new_rel || file == old_rel;
            if touched_by_branch {
                rewrite_file(repo, &file, |c| rewrite_bare(c, num, new_num))?;
            }
        }

        // Reflect the rename so a second newcomer gets a fresh number.
        all.retain(|n| n != added_name);
        all.push(new_name.clone());
        summary.push(format!("{added_name} -> {new_name}"));
    }

    if summary.is_empty() {
        return Ok("no ADR collisions to resolve".to_string());
    }

    // Keep the README ADR table in lockstep with the renamed/renumbered files: a
    // bump changes a number, a link target, and (for a brand-new ADR) adds a row.
    // Tolerate a README without the table markers — a scratch/test repo may omit
    // them — by noting the skip; a genuine sync failure (unreadable README, a
    // malformed table) still fails the renumber rather than hiding in the summary.
    let table_note = if crate::adr_readme::readme_has_markers(repo)? {
        format!(
            "README table synced ({})",
            crate::adr_readme::sync_readme_at(repo)?
        )
    } else {
        "README table not synced (no adr-table markers)".to_string()
    };

    // The rename is staged (`git mv`); the reference rewrites and the table
    // regen are written to the worktree but left unstaged, so flag the mixed
    // state for the caller.
    Ok(format!(
        "{} — {table_note}; review and `git add` the renamed files, rewritten references, and README before committing",
        summary.join("; ")
    ))
}

/// Slugs of the draft ADRs under `repo`'s `docs/adr/drafts`, sorted for a
/// deterministic assignment order. The tracked `README.md` explainer and any
/// non-`.md` entry are skipped; `<slug>.md` yields `slug`.
fn draft_slugs(repo: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(repo.join(DRAFTS_DIR)) else {
        return Vec::new();
    };
    let mut slugs: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "README.md")
        .filter_map(|n| n.strip_suffix(".md").map(str::to_string))
        .collect();
    slugs.sort();
    slugs
}

/// Entry point for `cargo xtask adr promote`: operate on the current repo.
pub fn promote() -> StepResult {
    match run_promote(Path::new(".")) {
        Ok(summary) => StepResult::ok("adr-promote").detail(summary),
        Err(e) => StepResult::fail("adr-promote").detail(format!("{e:#}")),
    }
}

/// Number every draft in `docs/adr/drafts`, graduate it into
/// `docs/adr/NNNN-<slug>.md`, record its acceptance in the status line, rewrite its
/// path-form references, sync the README table, and stage the result. Numbers are
/// assigned at ship (post-rebase), so the ADR's first appearance in git history is
/// already collision-free.
///
/// Unlike `renumber`, the source is an *untracked* draft: it is written under its
/// number, the draft is dropped, and the result is staged with `git add` (no
/// `git mv`); and there is no bare `ADR-NNNN` form to rewrite, since a draft is
/// referenced only by its `drafts/<slug>` path.
fn run_promote(repo: &Path) -> Result<String> {
    let slugs = draft_slugs(repo);
    if slugs.is_empty() {
        return Ok("no ADR drafts to promote".to_string());
    }

    // Pass A — assign every draft a number before rewriting anything, so a draft
    // that references another draft can resolve to the assigned number. `all`
    // grows with each assignment, exactly as the renumber loop does.
    let mut all = adr_filenames(repo);
    let mut assigned: Vec<(String, u32, String)> = Vec::new();
    for slug in &slugs {
        let num = ids::next_number(&all);
        let new_name = format!("{}-{slug}.md", pad(num));
        all.push(new_name.clone());
        assigned.push((slug.clone(), num, new_name));
    }

    // Pass B — graduate each draft (heading token `ADR-DRAFT` -> `ADR-NNNN`,
    // status `proposed` -> `accepted`, write it under its number, drop the draft)
    // and stage it. Staging first makes the path-form rewrite below see
    // cross-references between graduated drafts (which now live in tracked files).
    //
    // Which drafts had their status rewritten, so Pass C — which owns the summary —
    // can report the transition. Keyed by slug rather than carried as a parallel
    // vector, so the two passes cannot drift out of step.
    let mut accepted_here: BTreeSet<String> = BTreeSet::new();
    for (slug, num, new_name) in &assigned {
        let draft_rel = format!("{DRAFTS_DIR}/{slug}.md");
        let new_rel = format!("{ADR_DIR}/{new_name}");
        let body = std::fs::read_to_string(repo.join(&draft_rel))
            .with_context(|| format!("reading {draft_rel}"))?;
        let numbered = body.replace("ADR-DRAFT", &format!("ADR-{}", pad(*num)));
        // The file moves up one directory here, so its own relative links are
        // rewritten at the same moment — not after Pass C, which would see targets
        // that have already been rewritten to their assigned numbers.
        let relinked = strip_one_level(&numbered);
        // Numbering is the acceptance event; record it in the status line.
        let graduated = match rewrite_status(&relinked) {
            Some(accepted) => {
                accepted_here.insert(slug.clone());
                accepted
            }
            None => relinked,
        };
        std::fs::write(repo.join(&new_rel), graduated)
            .with_context(|| format!("writing {new_rel}"))?;
        std::fs::remove_file(repo.join(&draft_rel))
            .with_context(|| format!("removing {draft_rel}"))?;
        git::add(repo, &new_rel)?;
    }

    // Pass C — rewrite path-form references repo-wide. `drafts/<slug>` carries the
    // slug, so it is unambiguous (same substring-replace assumption
    // `rewrite_stem` documents). The graduated files are staged (tracked), so a
    // draft-to-draft reference is rewritten too.
    let mut summary = Vec::new();
    for (slug, num, new_name) in &assigned {
        let draft_stem = format!("drafts/{slug}");
        let new_stem = format!("{}-{slug}", pad(*num));
        for file in git::grep_files(repo, &draft_stem)? {
            rewrite_file(repo, &file, |c| rewrite_stem(c, &draft_stem, &new_stem))?;
            git::add(repo, &file)?;
        }
        let status_note = if accepted_here.contains(slug) {
            " (status: proposed -> accepted)"
        } else {
            ""
        };
        summary.push(format!(
            "{DRAFTS_DIR}/{slug}.md -> {ADR_DIR}/{new_name}{status_note}"
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
    for (_slug, _num, new_name) in &assigned {
        let rel = format!("{ADR_DIR}/{new_name}");
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
    fn replace_number_keeps_slug_and_extension() {
        assert_eq!(replace_number("0034-bar.md", 35), "0035-bar.md");
        assert_eq!(
            replace_number("0034-multi-word-slug.md", 35),
            "0035-multi-word-slug.md"
        );
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
        run_promote(&tmp).unwrap();
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
        run_promote(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("](template.md)"), "body: {body}");
        assert!(body.contains("](../CONTRIBUTING.md)"), "body: {body}");
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
        let summary = run_promote(&tmp).unwrap(); // Ok, not Err — promote still graduates
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
        let summary = run_promote(&tmp).unwrap();
        assert!(!summary.contains("warning"), "summary: {summary}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_checks_links_after_pass_c() {
        // `../drafts/aaa.md` is the cross-draft form that survives promotion: Pass B
        // strips it to `drafts/aaa.md`, Pass C rewrites that to `0002-aaa.md`, which
        // resolves from docs/adr/. So a correctly-ordered check finds nothing.
        //
        // This is the discriminating shape. Run the check before Pass C and the
        // target is still `drafts/aaa.md`, pointing at the draft Pass B already
        // deleted — a premature check warns and this test fails.
        let tmp = promote_repo("order");
        write(&tmp, "docs/adr/drafts/aaa.md", "# ADR-DRAFT: Aaa\n");
        write(
            &tmp,
            "docs/adr/drafts/bbb.md",
            "# ADR-DRAFT: Bbb\n\nBuilds on [aaa](../drafts/aaa.md).\n",
        );
        let summary = run_promote(&tmp).unwrap();
        assert!(!summary.contains("warning"), "premature check: {summary}");
        let bbb = std::fs::read_to_string(tmp.join("docs/adr/0003-bbb.md")).unwrap();
        assert!(bbb.contains("](0002-aaa.md)"), "bbb: {bbb}");
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
        run_promote(&tmp).unwrap();
        let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
        assert!(body.contains("- Status: accepted"), "body: {body}");
        assert!(!body.contains("proposed"), "body: {body}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_preserves_a_deliberate_status() {
        // `superseded`/`rejected` on a draft are authorial statements, not the
        // template default — promotion must not flatten them to `accepted`.
        for (tag, token) in [("sup", "superseded"), ("rej", "rejected")] {
            let tmp = promote_repo(tag);
            let draft = format!("# ADR-DRAFT: D\n\n- Status: {token}\n");
            write(&tmp, "docs/adr/drafts/d.md", &draft);
            run_promote(&tmp).unwrap();
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
        run_promote(&tmp).unwrap();
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
        run_promote(&tmp).unwrap();
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
        let summary = run_promote(&tmp).unwrap();
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
        let summary = run_promote(&tmp).unwrap();
        assert!(!summary.contains("status:"), "summary: {summary}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rewrite_bare_replaces_only_the_padded_token() {
        let content = "ADR-0034 governs this. Unrelated number 10034 stays.";
        let out = rewrite_bare(content, 34, 35);
        assert_eq!(out, "ADR-0035 governs this. Unrelated number 10034 stays.");
    }

    /// Trimmed stdout of a git command that must succeed — for asserting index
    /// state (`diff --cached`).
    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let out = crate::git::at(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
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
    fn renumber_bumps_newcomer_and_rewrites_refs() {
        let tmp = std::env::temp_dir().join(format!("jaunder-adr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q", "-b", "main"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);

        // main: ADR-0034-foo plus a doc that references it by both forms.
        write(&tmp, "docs/adr/0034-foo.md", "# ADR-0034: Foo\n");
        write(
            &tmp,
            "CONTRIBUTING.md",
            "See ADR-0034 at docs/adr/0034-foo.md.\n",
        );
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "main: 0034-foo"]);

        // branch: a colliding ADR-0034-bar plus a NEW file referencing it.
        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        write(
            &tmp,
            "docs/adr/0034-bar.md",
            "# ADR-0034: Bar\nsee docs/adr/0034-bar.md\n",
        );
        write(
            &tmp,
            "docs/notes.md",
            "Decided in ADR-0034 (docs/adr/0034-bar.md).\n",
        );
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "feature: 0034-bar"]);

        let summary = run_renumber(&tmp, "main").unwrap();
        assert!(
            summary.contains("0034-bar.md -> 0035-bar.md"),
            "summary: {summary}"
        );

        // The newcomer moved; main's ADR is untouched.
        assert!(tmp.join("docs/adr/0035-bar.md").exists());
        assert!(!tmp.join("docs/adr/0034-bar.md").exists());
        assert!(tmp.join("docs/adr/0034-foo.md").exists());

        // Branch-added file: both forms rewritten to 0035.
        let notes = std::fs::read_to_string(tmp.join("docs/notes.md")).unwrap();
        assert_eq!(notes, "Decided in ADR-0035 (docs/adr/0035-bar.md).\n");

        // The moved ADR's own title (bare form, branch-touched) rewritten.
        let bar = std::fs::read_to_string(tmp.join("docs/adr/0035-bar.md")).unwrap();
        assert!(bar.contains("# ADR-0035: Bar"));
        assert!(bar.contains("docs/adr/0035-bar.md"));

        // main's pre-existing file keeps its bare ADR-0034 (NOT branch-touched).
        let contributing = std::fs::read_to_string(tmp.join("CONTRIBUTING.md")).unwrap();
        assert_eq!(contributing, "See ADR-0034 at docs/adr/0034-foo.md.\n");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn renumber_syncs_the_readme_table() {
        // A bump must move the row's number + link target and add a row for a
        // brand-new ADR (seeded from its heading), leaving the existing row intact.
        let tmp = std::env::temp_dir().join(format!("jaunder-adr-readme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q", "-b", "main"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);

        // main: ADR-0034-foo with a status line + a README carrying the marked
        // table with foo's (curated) row.
        write(
            &tmp,
            "docs/adr/0034-foo.md",
            "# ADR-0034: Foo\n\n- Status: accepted\n",
        );
        write(
            &tmp,
            "docs/README.md",
            "# Docs\n\n<!-- adr-table:begin -->\n\n\
             | #   | Title | Status |\n| --- | ----- | ------ |\n\
             | [0034](adr/0034-foo.md) | Foo | accepted |\n\n\
             <!-- adr-table:end -->\n",
        );
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "main: 0034-foo + README"]);

        // branch: a colliding ADR-0034-bar (no README row — the point of the flow).
        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        write(
            &tmp,
            "docs/adr/0034-bar.md",
            "# ADR-0034: Bar\n\n- Status: accepted\n",
        );
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "feature: 0034-bar"]);

        run_renumber(&tmp, "main").unwrap();

        let readme = std::fs::read_to_string(tmp.join("docs/README.md")).unwrap();
        // Bar's row was added under its bumped number, seeded from the heading.
        assert!(
            readme.contains("[0035](adr/0035-bar.md)"),
            "README: {readme}"
        );
        assert!(readme.contains("| Bar |"), "seeded title from heading");
        // Foo's existing row is untouched; no stale 0034-bar link remains.
        assert!(readme.contains("[0034](adr/0034-foo.md)"));
        assert!(!readme.contains("0034-bar.md"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn renumber_assigns_distinct_numbers_to_multiple_newcomers() {
        // Guards the `all`-mutation loop: two newcomers colliding on the same number
        // must each get a distinct fresh number, not the same one. `added` arrives in
        // git's sorted order (bar before baz), so the assignment is deterministic.
        let tmp = std::env::temp_dir().join(format!("jaunder-adr-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q", "-b", "main"]);
        git(&tmp, &["config", "user.email", "t@t"]);
        git(&tmp, &["config", "user.name", "t"]);

        write(&tmp, "docs/adr/0034-foo.md", "# ADR-0034: Foo\n");
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "main: 0034-foo"]);

        git(&tmp, &["checkout", "-q", "-b", "feature"]);
        write(&tmp, "docs/adr/0034-bar.md", "# ADR-0034: Bar\n");
        write(&tmp, "docs/adr/0034-baz.md", "# ADR-0034: Baz\n");
        git(&tmp, &["add", "."]);
        git(&tmp, &["commit", "-qm", "feature: two colliding ADRs"]);

        run_renumber(&tmp, "main").unwrap();

        // main's ADR untouched; both newcomers got distinct fresh numbers.
        assert!(tmp.join("docs/adr/0034-foo.md").exists());
        assert!(!tmp.join("docs/adr/0034-bar.md").exists());
        assert!(!tmp.join("docs/adr/0034-baz.md").exists());
        assert!(tmp.join("docs/adr/0035-bar.md").exists());
        assert!(tmp.join("docs/adr/0036-baz.md").exists());

        let _ = std::fs::remove_dir_all(&tmp);
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

        // An untracked, numberless draft.
        write(
            &tmp,
            "docs/adr/drafts/my-decision.md",
            "# ADR-DRAFT: My Decision\n\n- Status: proposed\n",
        );

        let summary = run_promote(&tmp).unwrap();
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

        // The graduated file and the README are staged, ready to commit.
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        assert!(
            staged.contains("docs/adr/0002-my-decision.md"),
            "staged: {staged}"
        );
        assert!(staged.contains("docs/README.md"), "staged: {staged}");

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

        run_promote(&tmp).unwrap();

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

        run_promote(&tmp).unwrap();

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

        run_promote(&tmp).unwrap();

        let bbb = std::fs::read_to_string(tmp.join("docs/adr/0003-bbb.md")).unwrap();
        assert!(
            bbb.contains("docs/adr/0002-aaa.md"),
            "cross-reference rewritten: {bbb}"
        );
        assert!(!bbb.contains("drafts/aaa"), "no stale draft path: {bbb}");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn promote_is_noop_without_drafts() {
        let tmp = promote_repo("noop");
        let summary = run_promote(&tmp).unwrap();
        assert_eq!(summary, "no ADR drafts to promote");
        // Nothing staged.
        let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
        assert!(staged.is_empty(), "staged: {staged}");

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

        run_promote(&tmp).unwrap();

        assert!(tmp.join("docs/adr/0006-later.md").exists());
        assert!(!tmp.join("docs/adr/drafts/later.md").exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn adr_filenames_skips_the_drafts_subdir() {
        // The renumber/promote base set must not see draft entries: `adr_filenames`
        // is non-recursive and file-only, so the `docs/adr/drafts/` subdirectory
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

        let names = adr_filenames(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(names, vec!["0001-a.md".to_string()]);
    }
}
