//! Git helpers for the verify gate: working-tree cleanliness (the `validate`
//! backstop) and self-healing `core.hooksPath` installation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Repo-relative hooks directory the gate routes git to. Relative (not absolute)
/// so each worktree resolves to its own `.githooks` checkout.
pub const HOOKS_PATH: &str = ".githooks";

/// A `git -C <dir>` command scrubbed of the ambient env vars that redirect git at
/// a different repository. A git hook (e.g. `.githooks/pre-push`) exports
/// `GIT_DIR`/`GIT_INDEX_FILE`; those would make `git -C <dir>` operate on the
/// hook's repo instead of `dir`. Clearing them pins the target to `-C <dir>`.
pub fn at(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// True when `git status --porcelain` output denotes a dirty tree. Porcelain lists
/// staged + unstaged tracked changes AND untracked non-gitignored files (`??`), and
/// omits gitignored paths — exactly the surface the Nix coverage source picks up.
/// Any non-blank line means dirty.
pub fn porcelain_is_dirty(porcelain: &str) -> bool {
    porcelain.lines().any(|line| !line.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusSnapshot {
    pub paths: BTreeMap<String, GitPathStatus>,
    /// Evidence which could not be represented safely.  A precommit snapshot is
    /// deliberately incomplete in this case, so both routing and reconciliation
    /// must fail closed.
    pub uncertainty: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitPathStatus {
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub delete_or_rename: bool,
    pub unsupported_change: bool,
    pub index_mode: Option<u32>,
    pub staged_change: Option<u8>,
    pub raw_change: Option<u8>,
    pub worktree_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecommitBroadReason {
    UncertainStatus,
    EmptyState,
    UntrackedPath,
    UnstagedPath,
    DeleteOrRename,
    UnsupportedChange,
    UnsupportedIndexMode,
    NonMarkdownPath,
}

impl PrecommitBroadReason {
    const fn detail(self) -> &'static str {
        match self {
            Self::UncertainStatus => "uncertain-status",
            Self::EmptyState => "empty-state",
            Self::UntrackedPath => "untracked-path",
            Self::UnstagedPath => "unstaged-path",
            Self::DeleteOrRename => "delete-or-rename",
            Self::UnsupportedChange => "unsupported-change",
            Self::UnsupportedIndexMode => "unsupported-index-mode",
            Self::NonMarkdownPath => "non-markdown-path",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecommitChangeClass {
    StagedMarkdownOnly,
    Broad(PrecommitBroadReason),
}

impl PrecommitChangeClass {
    pub fn detail(self) -> String {
        match self {
            Self::StagedMarkdownOnly => {
                "class=staged-markdown-only reason=isolated-staged-markdown".to_owned()
            }
            Self::Broad(reason) => format!("class=broad reason={}", reason.detail()),
        }
    }
}

pub fn classify_precommit_change(snapshot: &GitStatusSnapshot) -> PrecommitChangeClass {
    if !snapshot.uncertainty.is_empty() {
        return PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus);
    }
    if snapshot.paths.is_empty() {
        return PrecommitChangeClass::Broad(PrecommitBroadReason::EmptyState);
    }
    for status in snapshot.paths.values() {
        if status.untracked {
            return PrecommitChangeClass::Broad(PrecommitBroadReason::UntrackedPath);
        }
    }
    for status in snapshot.paths.values() {
        if status.unstaged || !status.staged {
            return PrecommitChangeClass::Broad(PrecommitBroadReason::UnstagedPath);
        }
    }
    for status in snapshot.paths.values() {
        if status.delete_or_rename {
            return PrecommitChangeClass::Broad(PrecommitBroadReason::DeleteOrRename);
        }
    }
    for status in snapshot.paths.values() {
        if status.unsupported_change {
            return PrecommitChangeClass::Broad(PrecommitBroadReason::UnsupportedChange);
        }
    }
    for status in snapshot.paths.values() {
        if !matches!(status.index_mode, Some(0o100644 | 0o100755)) {
            return PrecommitChangeClass::Broad(PrecommitBroadReason::UnsupportedIndexMode);
        }
    }
    if snapshot.paths.keys().any(|path| !path.ends_with(".md")) {
        return PrecommitChangeClass::Broad(PrecommitBroadReason::NonMarkdownPath);
    }
    PrecommitChangeClass::StagedMarkdownOnly
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecommitStagePlan {
    pub stage_paths: Vec<String>,
    pub failures: Vec<String>,
}

/// Whether `core.hooksPath` needs (re)pointing at [`HOOKS_PATH`], given its current
/// value (`None` = unset).
pub fn needs_hooks_path(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != HOOKS_PATH,
        None => true,
    }
}

/// `git status --porcelain` text. Errors only if git itself cannot run.
pub fn working_tree_status(dir: &Path) -> Result<String> {
    output(dir, &["status", "--porcelain"])
}

/// Snapshot the complete dirty working tree and its staged index evidence.
/// NUL-delimited porcelain preserves whitespace and prevents a path from being
/// mistaken for status syntax; cached raw diff supplies the index mode.
pub fn status_snapshot(dir: &Path) -> Result<GitStatusSnapshot> {
    let status = at(dir)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ])
        .output()
        .with_context(|| "running NUL-delimited git status")?;
    if !status.status.success() {
        anyhow::bail!(
            "git status --porcelain=v1 -z failed: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        );
    }
    let raw = at(dir)
        .args([
            "diff",
            "--cached",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-renames",
        ])
        .output()
        .with_context(|| "running NUL-delimited cached raw diff")?;
    if !raw.status.success() {
        anyhow::bail!(
            "git diff --cached --raw -z failed: {}",
            String::from_utf8_lossy(&raw.stderr).trim()
        );
    }
    let mut snapshot = parse_snapshot(&status.stdout, &raw.stdout);
    if snapshot.uncertainty.is_empty() {
        for (path, entry) in &mut snapshot.paths {
            if fingerprintable(entry) || fingerprintable_unstaged(dir, path, entry) {
                entry.worktree_fingerprint = Some(output(dir, &["hash-object", "--", path])?);
            }
        }
    }
    Ok(snapshot)
}

fn fingerprintable(entry: &GitPathStatus) -> bool {
    !entry.untracked
        && !entry.delete_or_rename
        && !entry.unsupported_change
        && matches!(entry.index_mode, Some(0o100644 | 0o100755))
}

fn fingerprintable_unstaged(dir: &Path, path: &str, entry: &GitPathStatus) -> bool {
    !entry.staged
        && entry.unstaged
        && !entry.untracked
        && !entry.delete_or_rename
        && !entry.unsupported_change
        && std::fs::symlink_metadata(dir.join(path))
            .is_ok_and(|metadata| metadata.file_type().is_file())
}

/// Legacy line-oriented parser retained for staging-plan unit fixtures. Production
/// snapshots must use [`status_snapshot`].
pub fn parse_status_snapshot(porcelain: &str) -> GitStatusSnapshot {
    let mut nul = porcelain.as_bytes().to_vec();
    for byte in &mut nul {
        if *byte == b'\n' {
            *byte = 0;
        }
    }
    parse_snapshot_with_raw_evidence(&nul, &[], false)
}

fn parse_snapshot(porcelain: &[u8], raw: &[u8]) -> GitStatusSnapshot {
    parse_snapshot_with_raw_evidence(porcelain, raw, true)
}

fn nul_records<'a>(bytes: &'a [u8], uncertainty: &mut Vec<String>, label: &str) -> Vec<&'a [u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut records = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
    if bytes.ends_with(&[0]) {
        records.pop();
    } else {
        uncertainty.push(format!("unterminated-{label}-record"));
    }
    if records.iter().any(|record| record.is_empty()) {
        uncertainty.push(format!("interior-empty-{label}-record"));
    }
    records
        .into_iter()
        .filter(|record| !record.is_empty())
        .collect()
}

fn parse_snapshot_with_raw_evidence(
    porcelain: &[u8],
    raw: &[u8],
    require_raw_evidence: bool,
) -> GitStatusSnapshot {
    let mut snapshot = GitStatusSnapshot {
        paths: BTreeMap::new(),
        uncertainty: Vec::new(),
    };
    let mut staged_from_status = BTreeSet::new();
    let mut seen_status_records = BTreeSet::new();
    let mut records = nul_records(porcelain, &mut snapshot.uncertainty, "status").into_iter();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            snapshot
                .uncertainty
                .push("malformed-status-record".to_owned());
            continue;
        }
        let index = record[0];
        let worktree = record[1];
        if !valid_status_pair(index, worktree) {
            snapshot.uncertainty.push("unknown-status-pair".to_owned());
            continue;
        }
        let Some(path) = std::str::from_utf8(&record[3..])
            .ok()
            .filter(|path| !path.is_empty())
        else {
            snapshot
                .uncertainty
                .push("non-utf8-or-empty-status-path".to_owned());
            continue;
        };
        let rename_like = matches!(index, b'R' | b'C');
        let old_path = if rename_like { records.next() } else { None };
        let mut add = |path: &str| {
            if !seen_status_records.insert((path.to_owned(), index, worktree)) {
                snapshot
                    .uncertainty
                    .push("duplicate-status-path".to_owned());
            }
            let entry = snapshot.paths.entry(path.to_owned()).or_default();
            if index == b'?' && worktree == b'?' {
                entry.untracked = true;
                return;
            }
            entry.staged |= index != b' ';
            entry.unstaged |= worktree != b' ';
            entry.delete_or_rename |=
                matches!(index, b'D' | b'R') || matches!(worktree, b'D' | b'R');
            entry.unsupported_change |=
                matches!(index, b'T' | b'U' | b'C') || matches!(worktree, b'T' | b'U' | b'C');
            if index != b' ' {
                entry.staged_change = Some(index);
            }
            if entry.staged {
                staged_from_status.insert(path.to_owned());
            }
        };
        add(path);
        match old_path {
            Some(old_path) => match std::str::from_utf8(old_path)
                .ok()
                .filter(|path| !path.is_empty())
            {
                Some(old_path) => add(old_path),
                None => snapshot
                    .uncertainty
                    .push("non-utf8-or-empty-rename-path".to_owned()),
            },
            None if rename_like => snapshot.uncertainty.push("missing-rename-path".to_owned()),
            None => {}
        }
    }

    let mut staged_from_raw = BTreeSet::new();
    let mut records = nul_records(raw, &mut snapshot.uncertainty, "raw").into_iter();
    let mut seen_raw_paths = BTreeSet::new();
    while let Some(header) = records.next() {
        let Some((old_mode, new_mode, status)) = parse_raw_header(header) else {
            snapshot.uncertainty.push("malformed-raw-record".to_owned());
            continue;
        };
        let Some(path_bytes) = records.next() else {
            snapshot.uncertainty.push("missing-raw-path".to_owned());
            continue;
        };
        let Some(path) = std::str::from_utf8(path_bytes)
            .ok()
            .filter(|path| !path.is_empty())
        else {
            snapshot
                .uncertainty
                .push("non-utf8-or-empty-raw-path".to_owned());
            continue;
        };
        let rename_like = matches!(status, b'R' | b'C');
        let old_path = if rename_like { records.next() } else { None };
        if !seen_raw_paths.insert(path.to_owned()) {
            snapshot.uncertainty.push("duplicate-raw-path".to_owned());
        }
        let entry = snapshot.paths.entry(path.to_owned()).or_default();
        entry.delete_or_rename |= matches!(status, b'D' | b'R');
        entry.unsupported_change |= !matches!(status, b'A' | b'M' | b'D' | b'R');
        entry.index_mode = Some(new_mode);
        entry.raw_change = Some(status);
        staged_from_raw.insert(path.to_owned());
        if rename_like {
            match old_path.and_then(|value| std::str::from_utf8(value).ok()) {
                Some(old_path) if !old_path.is_empty() => {
                    if !seen_raw_paths.insert(old_path.to_owned()) {
                        snapshot.uncertainty.push("duplicate-raw-path".to_owned());
                    }
                    let old = snapshot.paths.entry(old_path.to_owned()).or_default();
                    old.index_mode = Some(old_mode);
                    staged_from_raw.insert(old_path.to_owned());
                }
                _ => snapshot
                    .uncertainty
                    .push("non-utf8-or-empty-raw-rename-path".to_owned()),
            }
        }
    }
    if require_raw_evidence {
        for entry in snapshot.paths.values() {
            if entry
                .staged_change
                .zip(entry.raw_change)
                .is_some_and(|(status, raw)| status != raw)
            {
                snapshot
                    .uncertainty
                    .push("status-raw-change-mismatch".to_owned());
            }
        }
        if staged_from_status != staged_from_raw {
            snapshot
                .uncertainty
                .push("status-raw-population-mismatch".to_owned());
        }
    }
    snapshot.uncertainty.sort();
    snapshot.uncertainty.dedup();
    snapshot
}

fn valid_status_pair(index: u8, worktree: u8) -> bool {
    if (index, worktree) == (b'?', b'?') {
        return true;
    }
    matches!(index, b' ' | b'M' | b'A' | b'D' | b'R' | b'C' | b'U' | b'T')
        && matches!(worktree, b' ' | b'M' | b'A' | b'D' | b'U' | b'T')
}

fn parse_raw_header(header: &[u8]) -> Option<(u32, u32, u8)> {
    let text = std::str::from_utf8(header).ok()?;
    let mut fields = text.strip_prefix(':')?.split(' ');
    let old_mode = parse_raw_mode(fields.next()?)?;
    let new_mode = parse_raw_mode(fields.next()?)?;
    let old_object = fields.next()?;
    let new_object = fields.next()?;
    if !valid_object_id(old_object) || !valid_object_id(new_object) {
        return None;
    }
    let status = fields.next()?;
    if fields.next().is_some() || status.is_empty() {
        return None;
    }
    let (kind, score) = status.split_at(1);
    let status = kind.as_bytes()[0];
    match status {
        b'R' | b'C' if score.parse::<u8>().is_ok_and(|score| score <= 100) => {}
        b'A' | b'M' | b'D' | b'T' | b'U' if score.is_empty() => {}
        _ => return None,
    }
    valid_raw_transition(old_mode, new_mode, old_object, new_object, status)
        .then_some((old_mode, new_mode, status))
}

fn parse_raw_mode(mode: &str) -> Option<u32> {
    (mode.len() == 6 && mode.bytes().all(|byte| matches!(byte, b'0'..=b'7')))
        .then(|| u32::from_str_radix(mode, 8).ok())
        .flatten()
}

fn valid_object_id(object_id: &str) -> bool {
    matches!(object_id.len(), 40 | 64) && object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_raw_transition(
    old_mode: u32,
    new_mode: u32,
    old_object: &str,
    new_object: &str,
    status: u8,
) -> bool {
    if old_object.len() != new_object.len() {
        return false;
    }
    let old_zero = old_object.bytes().all(|byte| byte == b'0');
    let new_zero = new_object.bytes().all(|byte| byte == b'0');
    match status {
        b'A' => old_mode == 0 && known_mode(new_mode) && old_zero && !new_zero,
        b'D' => known_mode(old_mode) && new_mode == 0 && !old_zero && new_zero,
        b'M' => {
            known_mode(old_mode)
                && known_mode(new_mode)
                && object_type(old_mode) == object_type(new_mode)
                && !old_zero
                && !new_zero
                && (old_mode != new_mode || old_object != new_object)
        }
        b'T' => {
            known_mode(old_mode)
                && known_mode(new_mode)
                && object_type(old_mode) != object_type(new_mode)
                && !old_zero
                && !new_zero
        }
        b'U' => known_mode(old_mode) && known_mode(new_mode) && !old_zero && !new_zero,
        _ => false,
    }
}

fn known_mode(mode: u32) -> bool {
    matches!(mode, 0o100644 | 0o100755 | 0o120000 | 0o160000)
}

fn object_type(mode: u32) -> u8 {
    match mode {
        0o100644 | 0o100755 => 1,
        0o120000 => 2,
        0o160000 => 3,
        _ => 0,
    }
}

pub fn precommit_stage_plan(
    before: &GitStatusSnapshot,
    after: &GitStatusSnapshot,
) -> PrecommitStagePlan {
    if !before.uncertainty.is_empty() || !after.uncertainty.is_empty() {
        let mut evidence = before
            .uncertainty
            .iter()
            .chain(after.uncertainty.iter())
            .cloned()
            .collect::<Vec<_>>();
        evidence.sort();
        evidence.dedup();
        return PrecommitStagePlan {
            stage_paths: Vec::new(),
            failures: vec![format!(
                "precommit staging requires complete git status evidence: {}",
                evidence.join(", ")
            )],
        };
    }
    let paths: BTreeSet<_> = before.paths.keys().chain(after.paths.keys()).collect();
    let has_delete_or_rename = before
        .paths
        .values()
        .chain(after.paths.values())
        .any(|status| status.delete_or_rename);
    let has_unsupported_evidence =
        before
            .paths
            .values()
            .chain(after.paths.values())
            .any(|status| {
                status.unsupported_change
                    || matches!(
                        status.index_mode,
                        Some(mode) if !matches!(mode, 0o100644 | 0o100755)
                    )
            });
    let mut stage_paths = Vec::new();
    let mut failures = Vec::new();
    for path in paths {
        let before_status = before.paths.get(path);
        let after_status = after.paths.get(path);
        if !precommit_path_changed(before_status, after_status) {
            continue;
        }
        if has_delete_or_rename {
            failures.push(format!(
                "{path}: delete/rename status is unsafe for auto-staging"
            ));
            continue;
        }
        if has_unsupported_evidence {
            failures.push(format!(
                "{path}: unsupported change or index mode is unsafe for auto-staging"
            ));
            continue;
        }
        if after_status.is_some_and(|s| s.untracked) {
            if before_status.is_none() {
                failures.push(format!(
                    "{path}: new untracked file created during precommit"
                ));
            }
            continue;
        }
        if before_status.is_some_and(|s| s.untracked) {
            continue;
        }
        match before_status {
            Some(status) if status.staged && !status.unstaged => stage_paths.push(path.clone()),
            Some(status) if status.staged && status.unstaged => {
                failures.push(format!(
                    "{path}: pre-existing mixed staged/unstaged state made auto-staging unsafe"
                ));
            }
            _ => failures.push(format!("{path}: will not add work the user did not stage")),
        }
    }
    stage_paths.sort();
    stage_paths.dedup();
    failures.sort();
    failures.dedup();
    PrecommitStagePlan {
        stage_paths,
        failures,
    }
}

fn precommit_path_changed(before: Option<&GitPathStatus>, after: Option<&GitPathStatus>) -> bool {
    fn comparable(
        status: Option<&GitPathStatus>,
    ) -> Option<(bool, bool, bool, bool, &Option<String>)> {
        status.map(|s| {
            (
                s.staged,
                s.unstaged,
                s.untracked,
                s.delete_or_rename,
                &s.worktree_fingerprint,
            )
        })
    }
    comparable(before) != comparable(after)
}

pub fn apply_precommit_stage_plan(dir: &Path, plan: &PrecommitStagePlan) -> crate::StepResult {
    let mut failures = plan.failures.clone();
    for path in &plan.stage_paths {
        if let Err(err) = run(dir, &["add", "--", path]) {
            failures.push(err.to_string());
        }
    }
    if !failures.is_empty() {
        failures.sort();
        failures.dedup();
        return crate::StepResult::fail("precommit-staging").detail(failures.join("\n"));
    }
    if plan.stage_paths.is_empty() {
        return crate::StepResult::ok("precommit-staging").detail("no staged fixes");
    }
    crate::StepResult::ok("precommit-staging")
        .detail(format!("staged: {}", plan.stage_paths.join(", ")))
}

/// Tracked files matching `glob`, sorted, relative to `dir`.
///
/// `git ls-files` rather than a filesystem walk: the walk would descend into
/// `target/` and into nested worktrees under `.claude/worktrees/`, so a gate that
/// enumerates "every source file" would police other checkouts' code.
///
/// `ls-files` lists only what is under `dir`, so a caller that means "every tracked
/// file in the repo" must pass [`toplevel`] — a test run with `--manifest-path
/// xtask/Cargo.toml` executes with `xtask/` as its cwd and would otherwise see a
/// partial tree, with paths relative to it rather than to the repo.
pub fn tracked_files(dir: &Path, glob: &str) -> Result<Vec<String>> {
    let out = output(dir, &["ls-files", "--", glob])?;
    let mut files: Vec<String> = out.lines().map(str::to_string).collect();
    files.sort();
    Ok(files)
}

/// The checked-out branch, or `None` when HEAD is detached (`--show-current`
/// prints nothing there). Used by `pr land` to tell "I am standing on this PR's
/// branch" from "I am elsewhere".
pub fn current_branch(dir: &Path) -> Result<Option<String>> {
    let name = output(dir, &["branch", "--show-current"])?;
    Ok((!name.is_empty()).then_some(name))
}

/// The local HEAD commit, or `None` in a repo with no commits yet (exit 128).
pub fn head_sha(dir: &Path) -> Result<Option<String>> {
    output_or(dir, &["rev-parse", "HEAD"], 128)
}

/// A remote's URL, or `None` when that remote is not configured. Read through
/// `config --get` rather than `remote get-url` so the unset case reuses the
/// exit-1-is-a-valid-nothing path already established here.
pub fn remote_url(dir: &Path, name: &str) -> Result<Option<String>> {
    config_get(dir, &format!("remote.{name}.url"))
}

/// Current `core.hooksPath`, or `None` when unset/blank (see [`config_get`]).
pub fn hooks_path(dir: &Path) -> Result<Option<String>> {
    config_get(dir, "core.hooksPath")
}

/// Ensure `core.hooksPath` points at [`HOOKS_PATH`]; set it if unset/wrong. Returns
/// `true` when it changed the config.
pub fn ensure_hooks_path(dir: &Path) -> Result<bool> {
    if needs_hooks_path(hooks_path(dir)?.as_deref()) {
        config_set(dir, "core.hooksPath", HOOKS_PATH)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Trimmed stdout of a git command in `dir`; bail on any non-zero exit. The one
/// place the capture-and-check plumbing lives.
pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<String> {
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Non-empty lines of [`output`].
pub(crate) fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(output(dir, args)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Run a git command in `dir` for effect (no capture); bail on non-zero exit.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<()> {
    let ok = at(dir)
        .args(args)
        .status()
        .with_context(|| format!("running git {args:?}"))?
        .success();
    if !ok {
        anyhow::bail!("git {args:?} failed");
    }
    Ok(())
}

/// Compare-and-delete a remote branch.  This is the only forceful promoter
/// operation: it has an expected old object and cannot update a changed ref.
pub(crate) fn delete_remote_with_lease(
    dir: &Path,
    remote: &str,
    branch: &str,
    expected: &str,
) -> Result<()> {
    let lease = format!("--force-with-lease=refs/heads/{branch}:{expected}");
    let destination = format!(":refs/heads/{branch}");
    run(dir, &["push", &lease, remote, &destination])
}

/// Whether an exact three-way merge of the supplied objects has content
/// conflicts. Exit 1 is Git's documented conflict result; all other failures
/// remain errors rather than becoming conflict authorization.
pub(crate) fn merge_tree_conflicts(dir: &Path, main: &str, head: &str) -> Result<bool> {
    let status = at(dir)
        .args(["merge-tree", "--write-tree", main, head])
        .status()
        .context("running git merge-tree")?;
    match status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => anyhow::bail!("git merge-tree failed ({status})"),
    }
}

/// Trimmed stdout of a git command, or `None` when it exits with `tolerated`
/// instead of bailing — the shared core of the two helpers that read one exit
/// code as a valid "nothing" answer (`grep`'s exit 1 = no match, `config --get`'s
/// exit 1 = unset). Any other non-zero still bails.
fn output_or(dir: &Path, args: &[&str], tolerated: i32) -> Result<Option<String>> {
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    match out.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        )),
        Some(c) if c == tolerated => Ok(None),
        _ => anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// `git grep -l --fixed-strings <pattern>` — files containing `pattern`.
/// Grep's exit 1 = no match → `Ok(vec![])`; exit 128 (or any other non-zero) =
/// real error → `Err` (see [`output_or`]).
pub(crate) fn grep_files(dir: &Path, pattern: &str) -> Result<Vec<String>> {
    let matched = output_or(dir, &["grep", "-l", "--fixed-strings", pattern], 1)?;
    Ok(match matched {
        Some(out) => out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    })
}

/// `git ls-files -- *.md` — tracked Markdown, repo-relative. The pathspec glob
/// matches at any depth, so filtering happens in git rather than in Rust.
pub(crate) fn ls_files_md(dir: &Path) -> Result<Vec<String>> {
    lines(dir, &["ls-files", "--", "*.md"])
}

/// `git mv <from> <to>`.
pub(crate) fn mv(dir: &Path, from: &str, to: &str) -> Result<()> {
    run(dir, &["mv", from, to])
}

/// `git add <path>`.
pub(crate) fn add(dir: &Path, path: &str) -> Result<()> {
    run(dir, &["add", path])
}

/// `git rev-parse --show-toplevel` — the working tree's root.
pub fn toplevel(dir: &Path) -> Result<String> {
    output(dir, &["rev-parse", "--show-toplevel"])
}

/// `git config --get <key>` → the value, or `None` when unset (exit 1) or blank.
/// Bails on any other non-zero (e.g. exit 128 = corrupt config): a broken config
/// surfaces as an error rather than being silently treated as "unset" (see
/// [`output_or`]).
pub(crate) fn config_get(dir: &Path, key: &str) -> Result<Option<String>> {
    Ok(output_or(dir, &["config", "--get", key], 1)?.filter(|s| !s.is_empty()))
}

/// `git config <key> <value>`.
pub(crate) fn config_set(dir: &Path, key: &str, value: &str) -> Result<()> {
    run(dir, &["config", key, value])
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{commit, git_ok, write};

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        crate::test_support::temp_repo("git", tag)
    }

    #[test]
    fn output_returns_trimmed_stdout_and_bails_on_error() {
        let dir = temp_repo("output");
        commit(&dir, "a.txt", "x\n");
        let head = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(head.len(), 40, "full sha, trimmed: {head:?}");
        assert!(output(&dir, &["not-a-subcommand"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lines_drops_blank_lines() {
        let dir = temp_repo("lines");
        commit(&dir, "a.txt", "1\n");
        commit(&dir, "b.txt", "2\n");
        let subjects = lines(&dir, &["log", "--format=%s"]).unwrap();
        assert_eq!(subjects, vec!["c".to_string(), "c".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_ok_on_success_err_on_failure() {
        let dir = temp_repo("run");
        commit(&dir, "a.txt", "x\n");
        assert!(run(&dir, &["status", "--porcelain"]).is_ok());
        assert!(run(&dir, &["mv", "nope", "nowhere"]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grep_files_match_no_match_and_error() {
        let dir = temp_repo("grep");
        commit(&dir, "hay.txt", "a needle here\n");
        commit(&dir, "other.txt", "nothing\n");
        assert_eq!(
            grep_files(&dir, "needle").unwrap(),
            vec!["hay.txt".to_string()]
        );
        assert!(grep_files(&dir, "absent-token").unwrap().is_empty()); // exit 1
        // A nonexistent dir → git can't chdir → exit 128 → Err (NOT an empty
        // match). Deterministic regardless of whether $TMPDIR sits under a repo.
        let missing =
            std::env::temp_dir().join(format!("jaunder-git-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        assert!(grep_files(&missing, "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toplevel_returns_repo_root() {
        let dir = temp_repo("toplevel");
        commit(&dir, "a.txt", "x\n");
        let root = toplevel(&dir).unwrap();
        // Compare canonically — /tmp may be a symlink.
        assert_eq!(
            std::fs::canonicalize(&root).unwrap(),
            std::fs::canonicalize(&dir).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_get_none_when_unset_some_after_set() {
        let dir = temp_repo("config");
        assert_eq!(config_get(&dir, "core.hooksPath").unwrap(), None);
        config_set(&dir, "core.hooksPath", ".githooks").unwrap();
        assert_eq!(
            config_get(&dir, "core.hooksPath").unwrap(),
            Some(".githooks".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_hooks_path_sets_then_is_noop() {
        let dir = temp_repo("ensure-hooks");
        assert!(ensure_hooks_path(&dir).unwrap(), "first call sets it");
        assert!(!ensure_hooks_path(&dir).unwrap(), "second call is a no-op");
        assert_eq!(hooks_path(&dir).unwrap(), Some(HOOKS_PATH.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn current_branch_head_sha_and_remote_read_a_real_repo() {
        let dir = temp_repo("readers");
        commit(&dir, "seed.txt", "x\n");
        assert!(current_branch(&dir).unwrap().is_some());
        let sha = head_sha(&dir).unwrap().expect("a seeded repo has a HEAD");
        assert_eq!(sha.len(), 40, "full sha, not abbreviated");
        assert_eq!(
            remote_url(&dir, "origin").unwrap(),
            None,
            "a throwaway repo has no remote"
        );
        config_set(&dir, "remote.origin.url", "git@github.com:o/r.git").unwrap();
        assert_eq!(
            remote_url(&dir, "origin").unwrap(),
            Some("git@github.com:o/r.git".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn porcelain_blank_is_clean() {
        assert!(!porcelain_is_dirty(""));
        assert!(!porcelain_is_dirty("\n"));
        assert!(!porcelain_is_dirty("   \n  \n"));
    }

    #[test]
    fn porcelain_untracked_is_dirty() {
        assert!(porcelain_is_dirty("?? new_file.rs"));
    }

    #[test]
    fn porcelain_staged_or_modified_is_dirty() {
        assert!(porcelain_is_dirty(" M src/lib.rs"));
        assert!(porcelain_is_dirty("A  staged.rs"));
        assert!(porcelain_is_dirty("?? a\n M b"));
    }

    #[test]
    fn needs_hooks_path_when_unset_or_wrong() {
        assert!(needs_hooks_path(None));
        assert!(needs_hooks_path(Some(".git/hooks")));
        assert!(needs_hooks_path(Some("")));
    }

    #[test]
    fn no_need_when_hooks_path_already_correct() {
        assert!(!needs_hooks_path(Some(".githooks")));
        assert!(!needs_hooks_path(Some(" .githooks \n")));
    }

    #[test]
    fn precommit_status_parser_classifies_index_worktree_and_untracked() {
        let snap = parse_status_snapshot("M  src/a.rs\n M src/b.rs\nMM src/c.rs\n?? scratch.rs\n");
        assert!(snap.paths["src/a.rs"].staged);
        assert!(!snap.paths["src/a.rs"].unstaged);
        assert!(!snap.paths["src/b.rs"].staged);
        assert!(snap.paths["src/b.rs"].unstaged);
        assert!(snap.paths["src/c.rs"].staged);
        assert!(snap.paths["src/c.rs"].unstaged);
        assert!(snap.paths["scratch.rs"].untracked);
    }

    #[test]
    fn precommit_status_parser_marks_delete_and_rename_unsafe() {
        let snap = parse_status_snapshot("D  gone.rs\n D missing.rs\nR  old.rs -> new.rs\n");
        assert!(snap.paths["gone.rs"].delete_or_rename);
        assert!(snap.paths["missing.rs"].delete_or_rename);
        assert!(snap.paths["old.rs -> new.rs"].delete_or_rename);
    }

    fn fp(mut snap: GitStatusSnapshot, path: &str, value: &str) -> GitStatusSnapshot {
        snap.paths.get_mut(path).unwrap().worktree_fingerprint = Some(value.to_string());
        snap
    }

    #[test]
    fn precommit_stage_plan_stages_only_clean_previously_staged_tracked_paths() {
        let before = fp(
            fp(
                parse_status_snapshot("M  a.rs\n M b.rs\n?? scratch.rs\n"),
                "a.rs",
                "old-a",
            ),
            "b.rs",
            "old-b",
        );
        let after = fp(
            fp(
                parse_status_snapshot("MM a.rs\n M b.rs\n?? scratch.rs\n"),
                "a.rs",
                "new-a",
            ),
            "b.rs",
            "old-b",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert_eq!(plan.stage_paths, vec!["a.rs".to_string()]);
        assert!(plan.failures.is_empty());
    }

    #[test]
    fn precommit_stage_plan_rejects_mixed_and_unstaged_only_mutations() {
        let before = fp(
            fp(
                parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"),
                "mixed.rs",
                "old-mixed",
            ),
            "unstaged.rs",
            "old-unstaged",
        );
        let after = fp(
            fp(
                parse_status_snapshot("MM mixed.rs\n M unstaged.rs\n"),
                "mixed.rs",
                "new-mixed",
            ),
            "unstaged.rs",
            "new-unstaged",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("mixed.rs") && f.contains("pre-existing mixed"))
        );
        assert!(
            plan.failures.iter().any(|f| f.contains("unstaged.rs")
                && f.contains("will not add work the user did not stage"))
        );
    }

    #[test]
    fn precommit_stage_plan_rejects_clean_before_tracked_mutation() {
        let before = parse_status_snapshot("");
        let after = fp(
            parse_status_snapshot(" M clean.rs\n"),
            "clean.rs",
            "new-clean",
        );
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures.iter().any(|f| f.contains("clean.rs")
                && f.contains("will not add work the user did not stage"))
        );
    }

    #[test]
    fn precommit_stage_plan_tolerates_old_untracked_and_rejects_new_untracked() {
        let before = parse_status_snapshot("?? old.tmp\n");
        let after = parse_status_snapshot("?? old.tmp\n?? new.tmp\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert_eq!(plan.failures.len(), 1);
        assert!(plan.failures[0].contains("new.tmp"));
        assert!(plan.failures[0].contains("new untracked"));
    }

    #[test]
    fn precommit_stage_plan_ignores_unchanged_delete_or_rename_states() {
        let before = parse_status_snapshot("D  gone.rs\nD  old.rs\nA  new.rs\n");
        let after = parse_status_snapshot("D  gone.rs\nD  old.rs\nA  new.rs\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(plan.failures.is_empty());
    }

    #[test]
    fn precommit_stage_plan_rejects_changed_rename_states() {
        let before = parse_status_snapshot("D  old.rs\nA  new.rs\n");
        let after = parse_status_snapshot("D  old.rs\nA  newer.rs\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("new.rs") && f.contains("delete/rename"))
        );
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("newer.rs") && f.contains("delete/rename"))
        );
    }

    #[test]
    fn precommit_stage_plan_rejects_delete_or_rename_states() {
        let before = parse_status_snapshot("M  keep.rs\n");
        let after = parse_status_snapshot("D  keep.rs\nD  old.rs\nA  new.rs\n");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("keep.rs") && f.contains("delete/rename"))
        );
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("new.rs") && f.contains("delete/rename"))
        );
    }

    #[test]
    fn precommit_stage_plan_preserves_delete_recreate_as_delete_rename() {
        let after = parse_status_snapshot("D  a.rs\n?? a.rs\n");
        assert!(after.paths["a.rs"].delete_or_rename);
        let plan = precommit_stage_plan(&parse_status_snapshot(""), &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|f| f.contains("a.rs") && f.contains("delete/rename"))
        );
    }

    #[test]
    fn precommit_apply_restages_only_the_previously_staged_file() {
        let dir = temp_repo("precommit-restage");
        commit(&dir, "a.rs", "fn a(){}\n");
        commit(&dir, "b.rs", "fn b(){}\n");

        write(&dir, "a.rs", "fn a() { }\n");
        git_ok(&dir, &["add", "a.rs"]);
        write(&dir, "b.rs", "fn b() { }\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "fn a() { }\n// formatted\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(step.ok, "{step:?}");
        assert_eq!(
            output(&dir, &["diff", "--cached", "--name-only"]).unwrap(),
            "a.rs"
        );
        assert_eq!(output(&dir, &["diff", "--name-only"]).unwrap(), "b.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_snapshot_forces_rename_detection_not_copy_detection() {
        let dir = temp_repo("status-no-copies");
        commit(&dir, "a.rs", "same\n");
        git_ok(&dir, &["config", "status.renames", "copies"]);
        write(&dir, "b.rs", "same\n");
        git_ok(&dir, &["add", "b.rs"]);

        let snap = status_snapshot(&dir).unwrap();

        assert!(snap.paths.contains_key("b.rs"));
        assert!(!snap.paths.keys().any(|path| path.contains(" -> ")));
        assert!(snap.paths["b.rs"].worktree_fingerprint.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_stages_safe_paths_even_when_other_failures_exist() {
        let dir = temp_repo("precommit-partial-failure");
        commit(&dir, "a.rs", "one\n");
        write(&dir, "a.rs", "two\n");
        git_ok(&dir, &["add", "a.rs"]);
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "three\n");
        write(&dir, "new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert_eq!(output(&dir, &["show", ":a.rs"]).unwrap(), "three");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_mixed_staged_unstaged_file() {
        let dir = temp_repo("precommit-mixed");
        commit(&dir, "a.rs", "one\n");
        write(&dir, "a.rs", "two\n");
        git_ok(&dir, &["add", "a.rs"]);
        write(&dir, "a.rs", "three\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "four\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(
            step.detail
                .as_deref()
                .unwrap()
                .contains("pre-existing mixed")
        );
        assert_eq!(output(&dir, &["show", ":a.rs"]).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "four\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_unstaged_only_file() {
        let dir = temp_repo("precommit-unstaged");
        commit(&dir, "a.rs", "one\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "a.rs", "two\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(
            step.detail
                .as_deref()
                .unwrap()
                .contains("will not add work the user did not stage")
        );
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_new_untracked_file_without_staging_old_untracked() {
        let dir = temp_repo("precommit-untracked");
        commit(&dir, "tracked.rs", "one\n");
        write(&dir, "old.tmp", "old\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn precommit_apply_refuses_new_file_inside_preexisting_untracked_dir() {
        let dir = temp_repo("precommit-untracked-dir");
        commit(&dir, "tracked.rs", "one\n");
        write(&dir, "scratch/old.tmp", "old\n");
        let before = status_snapshot(&dir).unwrap();

        write(&dir, "scratch/new.tmp", "new\n");
        let after = status_snapshot(&dir).unwrap();
        let plan = precommit_stage_plan(&before, &after);
        let step = apply_precommit_stage_plan(&dir, &plan);

        assert!(!step.ok);
        assert!(step.detail.as_deref().unwrap().contains("scratch/new.tmp"));
        assert!(step.detail.as_deref().unwrap().contains("new untracked"));
        assert!(
            output(&dir, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    fn snapshot(status: &[u8], raw: &[u8]) -> GitStatusSnapshot {
        parse_snapshot(status, raw)
    }

    fn raw(mode: &str, status: char, path: &str) -> Vec<u8> {
        const ZERO: &str = "0000000000000000000000000000000000000000";
        const OLD: &str = "1111111111111111111111111111111111111111";
        const NEW: &str = "2222222222222222222222222222222222222222";
        let (old_mode, new_mode, old_object, new_object) = match status {
            'A' => ("000000", mode, ZERO, NEW),
            'D' => (mode, "000000", OLD, ZERO),
            _ => (mode, mode, OLD, NEW),
        };
        format!(":{old_mode} {new_mode} {old_object} {new_object} {status}\0{path}\0").into_bytes()
    }
    fn raw_modes(old_mode: &str, new_mode: &str, status: char, path: &str) -> Vec<u8> {
        const OLD: &str = "1111111111111111111111111111111111111111";
        const NEW: &str = "2222222222222222222222222222222222222222";
        format!(":{old_mode} {new_mode} {OLD} {NEW} {status}\0{path}\0").into_bytes()
    }
    #[test]
    fn staged_regular_markdown_addition_and_modification_route_narrowly() {
        for (status, raw_status) in [
            (b"A  guide.md\0".as_slice(), 'A'),
            (b"M  guide.md\0".as_slice(), 'M'),
        ] {
            let snapshot = snapshot(status, &raw("100644", raw_status, "guide.md"));
            assert_eq!(
                classify_precommit_change(&snapshot),
                PrecommitChangeClass::StagedMarkdownOnly
            );
        }
        assert_eq!(
            PrecommitChangeClass::StagedMarkdownOnly.detail(),
            "class=staged-markdown-only reason=isolated-staged-markdown"
        );
    }

    #[test]
    fn classifier_uses_stable_broad_reason_precedence() {
        let cases = [
            (
                snapshot(b"bad\0", b""),
                PrecommitBroadReason::UncertainStatus,
            ),
            (snapshot(b"", b""), PrecommitBroadReason::EmptyState),
            (
                snapshot(b"?? scratch.md\0", b""),
                PrecommitBroadReason::UntrackedPath,
            ),
            (
                snapshot(b" M guide.md\0", b""),
                PrecommitBroadReason::UnstagedPath,
            ),
            (
                snapshot(b"D  guide.md\0", &raw("100644", 'D', "guide.md")),
                PrecommitBroadReason::DeleteOrRename,
            ),
            (
                snapshot(
                    b"T  guide.md\0",
                    &raw_modes("100644", "120000", 'T', "guide.md"),
                ),
                PrecommitBroadReason::UnsupportedChange,
            ),
            (
                snapshot(b"M  guide.md\0", &raw("120000", 'M', "guide.md")),
                PrecommitBroadReason::UnsupportedIndexMode,
            ),
            (
                snapshot(b"M  guide.MD\0", &raw("100644", 'M', "guide.MD")),
                PrecommitBroadReason::NonMarkdownPath,
            ),
        ];
        for (snapshot, reason) in cases {
            let class = classify_precommit_change(&snapshot);
            assert_eq!(class, PrecommitChangeClass::Broad(reason));
            assert_eq!(
                class.detail(),
                format!("class=broad reason={}", reason.detail())
            );
        }
    }
    #[test]
    fn classifier_fails_closed_for_non_utf8_and_population_conflicts() {
        let non_utf8 = snapshot(b"A  \xff.md\0", &raw("100644", 'A', "safe.md"));
        let conflict = snapshot(b"A  guide.md\0", &raw("100644", 'A', "other.md"));
        assert_eq!(
            classify_precommit_change(&non_utf8),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
        assert_eq!(
            classify_precommit_change(&conflict),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
    }

    #[test]
    fn malformed_status_and_raw_evidence_are_uncertain() {
        let unknown_status = snapshot(b"Z  guide.md\0", &raw("100644", 'M', "guide.md"));
        let malformed_raw = snapshot(
            b"M  guide.md\0",
            b":100644 100644 short invalid M\0guide.md\0",
        );
        assert_eq!(
            classify_precommit_change(&unknown_status),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
        assert_eq!(
            classify_precommit_change(&malformed_raw),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
    }

    #[test]
    fn duplicate_status_and_raw_paths_are_uncertain() {
        let duplicate_status = snapshot(
            b"M  guide.md\0M  guide.md\0",
            &raw("100644", 'M', "guide.md"),
        );
        let duplicate_raw = snapshot(
            b"M  guide.md\0",
            b":100644 100644 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 M\0guide.md\0:100644 100644 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 M\0guide.md\0",
        );
        assert!(
            duplicate_status
                .uncertainty
                .iter()
                .any(|reason| reason == "duplicate-status-path")
        );
        assert!(
            duplicate_raw
                .uncertainty
                .iter()
                .any(|reason| reason == "duplicate-raw-path")
        );
    }

    #[test]
    fn reconciliation_rejects_changed_path_with_unsupported_index_evidence() {
        let mut before = parse_status_snapshot("M  guide.md\n");
        before.paths.get_mut("guide.md").unwrap().index_mode = Some(0o120000);
        before
            .paths
            .get_mut("guide.md")
            .unwrap()
            .worktree_fingerprint = Some("before".to_owned());
        let mut after = before.clone();
        after
            .paths
            .get_mut("guide.md")
            .unwrap()
            .worktree_fingerprint = Some("after".to_owned());
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(plan.failures[0].contains("unsupported change or index mode"));
    }

    #[test]
    fn interior_empty_nul_records_are_uncertain() {
        let status_empty = snapshot(
            b"M  first.md\0\0M  second.md\0",
            &raw("100644", 'M', "first.md"),
        );
        let raw_empty = snapshot(
            b"M  guide.md\0",
            b":100644 100644 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 M\0\0guide.md\0",
        );
        assert!(!status_empty.uncertainty.is_empty());
        assert!(!raw_empty.uncertainty.is_empty());
    }

    #[test]
    fn mixed_raw_object_id_widths_are_uncertain() {
        let raw = b":100644 100644 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222222222222222222222222222 M\0guide.md\0";
        let snapshot = snapshot(b"M  guide.md\0", raw);
        assert_eq!(
            classify_precommit_change(&snapshot),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
    }

    #[test]
    fn impossible_raw_mode_transition_is_uncertain() {
        let raw = b":000000 100644 1111111111111111111111111111111111111111 1111111111111111111111111111111111111111 M\0guide.md\0";
        let snapshot = snapshot(b"M  guide.md\0", raw);
        assert_eq!(
            classify_precommit_change(&snapshot),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UncertainStatus)
        );
    }

    #[test]
    fn reconciliation_detects_mutation_of_preexisting_unstaged_regular_file() {
        let dir = temp_repo("precommit-unstaged-fingerprint");
        commit(&dir, "a.rs", "one\n");
        write(&dir, "a.rs", "two\n");
        let before = status_snapshot(&dir).expect("before snapshot");
        write(&dir, "a.rs", "three\n");
        let after = status_snapshot(&dir).expect("after snapshot");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(
            plan.failures
                .iter()
                .any(|failure| failure.contains("will not add work"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn gitlink_mode_is_broad_without_needing_a_worktree_fingerprint() {
        let snapshot = snapshot(b"M  nested\0", &raw("160000", 'M', "nested"));
        assert!(snapshot.uncertainty.is_empty());
        assert_eq!(
            classify_precommit_change(&snapshot),
            PrecommitChangeClass::Broad(PrecommitBroadReason::UnsupportedIndexMode)
        );
    }

    #[test]
    fn unsupported_index_modes_are_not_fingerprinted() {
        let entry = GitPathStatus {
            staged: true,
            index_mode: Some(0o160000),
            ..GitPathStatus::default()
        };
        assert!(!fingerprintable(&entry));
    }

    #[test]
    fn staging_reconciliation_refuses_incomplete_evidence() {
        let before = snapshot(b"bad\0", b"");
        let after = snapshot(b"bad\0", b"");
        let plan = precommit_stage_plan(&before, &after);
        assert!(plan.stage_paths.is_empty());
        assert!(plan.failures[0].contains("complete git status evidence"));
    }
    #[test]
    fn merge_tree_classifies_exact_conflicts_and_non_conflicting_ancestry() {
        let dir = temp_repo("promoter-merge-tree");
        commit(&dir, "shared.txt", "base\n");
        git_ok(&dir, &["branch", "candidate"]);
        write(&dir, "shared.txt", "main\n");
        git_ok(&dir, &["commit", "-am", "main"]);
        let main = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        git_ok(&dir, &["switch", "-q", "candidate"]);
        write(&dir, "shared.txt", "candidate\n");
        git_ok(&dir, &["commit", "-am", "candidate"]);
        let candidate = output(&dir, &["rev-parse", "HEAD"]).unwrap();

        assert!(merge_tree_conflicts(&dir, &main, &candidate).unwrap());
        assert!(!merge_tree_conflicts(&dir, &candidate, &candidate).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn leased_deletion_refuses_changed_head_and_ordinary_push_recreates_absent_branch() {
        let dir = temp_repo("promoter-lease");
        commit(&dir, "seed.txt", "one\n");
        let remote = dir.join("remote.git");
        git_ok(&dir, &["init", "--bare", "remote.git"]);
        git_ok(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git_ok(
            &dir,
            &["push", "origin", "HEAD:refs/heads/automation/adr-promoter"],
        );
        let original = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        commit(&dir, "seed.txt", "two\n");
        git_ok(
            &dir,
            &["push", "origin", "HEAD:refs/heads/automation/adr-promoter"],
        );

        assert!(
            delete_remote_with_lease(&dir, "origin", "automation/adr-promoter", &original).is_err()
        );
        let changed = output(&dir, &["rev-parse", "HEAD"]).unwrap();
        delete_remote_with_lease(&dir, "origin", "automation/adr-promoter", &changed).unwrap();
        git_ok(
            &dir,
            &["push", "origin", "HEAD:refs/heads/automation/adr-promoter"],
        );
        assert_eq!(
            output(
                &remote,
                &["rev-parse", "refs/heads/automation/adr-promoter"]
            )
            .unwrap(),
            changed
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
