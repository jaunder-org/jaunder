//! `cargo xtask adr renumber`: resolve an ADR number collision by bumping the
//! branch's newly-added ADR to the next free number and rewriting references.
//! The ADR already reachable from `origin/main` is immutable; only branch
//! additions move. Path-form references (which carry the slug) are rewritten
//! repo-wide; bare `ADR-NNNN` references are rewritten only in branch-touched
//! files, so `main`'s references to the other number are never clobbered.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;
use crate::result::StepResult;

const ADR_DIR: &str = "docs/adr";

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

/// Trimmed stdout of a git command in `repo`. A non-zero exit is an error, except
/// that when `allow_no_match` is set, exit code 1 is tolerated and yields empty
/// output — that is `git grep`'s "no match" signal. Any other non-zero (notably
/// 128 on a real grep error) still fails, so a genuine failure is never mistaken
/// for "nothing to rewrite".
fn git_out(repo: &Path, args: &[&str], allow_no_match: bool) -> Result<String> {
    let out = crate::git::at(repo)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() {
        let no_match = out.status.code() == Some(1);
        if !(allow_no_match && no_match) {
            anyhow::bail!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Non-empty lines of a git command's stdout.
fn git_lines(repo: &Path, args: &[&str], allow_no_match: bool) -> Result<Vec<String>> {
    Ok(git_out(repo, args, allow_no_match)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
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
    let base = git_out(repo, &["merge-base", main_ref, "HEAD"], false)
        .context("finding merge-base with main")?;
    let range = format!("{base}..HEAD");

    // ADR files this branch ADDED (filenames only).
    let added: Vec<String> = git_lines(
        repo,
        &[
            "diff",
            "--diff-filter=A",
            "--name-only",
            &range,
            "--",
            ADR_DIR,
        ],
        false,
    )?
    .into_iter()
    .filter_map(|p| p.rsplit('/').next().map(str::to_string))
    .collect();

    // Files this branch touched at all — the scope for bare-ref rewrites.
    let touched: Vec<String> = git_lines(repo, &["diff", "--name-only", &range], false)?;

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
        git_out(repo, &["mv", &old_rel, &new_rel], false)?;

        // 2. Path-form (slug-bearing) refs: rewrite repo-wide.
        for file in git_lines(repo, &["grep", "-l", "--fixed-strings", &old_stem], true)? {
            rewrite_file(repo, &file, |c| rewrite_stem(c, &old_stem, &new_stem))?;
        }

        // 3. Bare `ADR-NNNN` refs: rewrite only in branch-touched files (the moved
        //    ADR's own content counts — match its old and new paths too).
        let bare_token = format!("ADR-{}", pad(num));
        for file in git_lines(repo, &["grep", "-l", "--fixed-strings", &bare_token], true)? {
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn rewrite_bare_replaces_only_the_padded_token() {
        let content = "ADR-0034 governs this. Unrelated number 10034 stays.";
        let out = rewrite_bare(content, 34, 35);
        assert_eq!(out, "ADR-0035 governs this. Unrelated number 10034 stays.");
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = crate::git::at(dir).args(args).status().unwrap().success();
        assert!(ok, "git {args:?} failed");
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
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
}
