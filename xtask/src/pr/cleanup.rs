//! Explicit, fail-closed post-merge checkout cleanup.
//!
//! This is deliberately separate from the watch/land state machine: GitHub is only
//! observed here, while the injected checkout capability owns every local mutation.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::result::{CommandResult, StepResult};

use super::gh::{self, ApiError};
use super::snapshot::parse_remote;

const PRECHECK: &str = "pr-cleanup-precheck";
const FETCH: &str = "fetch-origin";
const VERIFY: &str = "verify-origin-main";
const DETACH: &str = "detach-origin-main";
const DELETE: &str = "delete-local-branch";
const CLEAN: &str = "cargo-clean";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSubject {
    pub number: u64,
    pub state: String,
    pub base_ref: String,
    pub head_ref: String,
    pub head_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutIdentity {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

/// Read-only GitHub boundary for cleanup-specific evidence.
pub trait CleanupSource {
    fn explicit(&self, number: u64) -> Result<CleanupSubject, ApiError>;
    fn merged_for(&self, branch: &str, sha: &str) -> Result<CleanupSubject, ApiError>;
}

/// Local facts and mutations. The executor owns ordering; implementations cannot
/// accidentally make a later operation happen after an earlier failure.
pub trait CleanupCheckout {
    fn identity(&self) -> Result<CheckoutIdentity>;
    fn is_dirty(&self) -> Result<bool>;
    fn fetch_origin(&self) -> Result<()>;
    fn head_is_ancestor_of_origin_main(&self, head: &str) -> Result<bool>;
    fn detach_origin_main(&self) -> Result<()>;
    fn delete_branch(&self, branch: &str) -> Result<()>;
    fn cargo_clean(&self) -> Result<()>;
}

pub struct GhCleanupSource;

enum GraphQlArgument {
    String(String),
    Integer(String),
}

fn graphql_args(query: &str, owner: &str, repo: &str, extra: &[GraphQlArgument]) -> Vec<String> {
    let mut args = vec![
        "api".into(),
        "graphql".into(),
        "-f".into(),
        format!("query={query}"),
        "-f".into(),
        format!("owner={owner}"),
        "-f".into(),
        format!("repo={repo}"),
    ];
    for argument in extra {
        match argument {
            GraphQlArgument::String(value) => {
                args.extend(["-f".into(), value.clone()]);
            }
            GraphQlArgument::Integer(value) => {
                args.extend(["-F".into(), value.clone()]);
            }
        }
    }
    args
}

impl GhCleanupSource {
    fn repository() -> Result<(String, String), ApiError> {
        let dir = Path::new(".");
        let url = crate::git::remote_url(dir, "origin")
            .map_err(|e| ApiError::Transport(format!("reading origin remote: {e:#}")))?
            .ok_or_else(|| ApiError::Malformed("origin remote is not configured".into()))?;
        parse_remote(&url)
            .ok_or_else(|| ApiError::Malformed("origin remote is not a GitHub repository".into()))
    }

    fn query(
        query: &str,
        owner: &str,
        repo: &str,
        extra: &[GraphQlArgument],
    ) -> Result<Value, ApiError> {
        let args = graphql_args(query, owner, repo, extra);
        gh::run_gh(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

const EXPLICIT_QUERY: &str = "query($owner:String!,$repo:String!,$number:Int!){repository(owner:$owner,name:$repo){pullRequest(number:$number){number state baseRefName headRefName headRefOid}}}";
const FIRST_PAGE_QUERY: &str = "query($owner:String!,$repo:String!){repository(owner:$owner,name:$repo){pullRequests(first:100,states:MERGED){nodes{number state baseRefName headRefName headRefOid}pageInfo{hasNextPage endCursor}}}}";
const NEXT_PAGE_QUERY: &str = "query($owner:String!,$repo:String!,$cursor:String!){repository(owner:$owner,name:$repo){pullRequests(first:100,after:$cursor,states:MERGED){nodes{number state baseRefName headRefName headRefOid}pageInfo{hasNextPage endCursor}}}}";

fn subject(value: &Value) -> Result<CleanupSubject, ApiError> {
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::Malformed("cleanup PR has no numeric number".into()))?;
    let required = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ApiError::Malformed(format!("cleanup PR #{number} has no {name}")))
    };
    Ok(CleanupSubject {
        number,
        state: required("state")?,
        base_ref: required("baseRefName")?,
        head_ref: required("headRefName")?,
        head_sha: required("headRefOid")?,
    })
}

fn explicit_subject(value: &Value) -> Result<CleanupSubject, ApiError> {
    let node = value
        .pointer("/data/repository/pullRequest")
        .filter(|v| !v.is_null())
        .ok_or_else(|| ApiError::Malformed("no pullRequest node in cleanup response".into()))?;
    subject(node)
}

fn page(value: &Value) -> Result<(Vec<CleanupSubject>, bool, Option<String>), ApiError> {
    let connection = value
        .pointer("/data/repository/pullRequests")
        .ok_or_else(|| ApiError::Malformed("no pullRequests node in cleanup response".into()))?;
    let nodes = connection
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Malformed("cleanup pullRequests nodes are not an array".into()))?;
    let nodes = nodes.iter().map(subject).collect::<Result<Vec<_>, _>>()?;
    let info = connection
        .get("pageInfo")
        .ok_or_else(|| ApiError::Malformed("cleanup pullRequests has no pageInfo".into()))?;
    let next = info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ApiError::Malformed("cleanup pullRequests pageInfo has no hasNextPage".into())
        })?;
    let cursor = info
        .get("endCursor")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if next && cursor.as_deref().is_none_or(str::is_empty) {
        return Err(ApiError::Malformed(
            "cleanup pullRequests next page has no endCursor".into(),
        ));
    }
    Ok((nodes, next, cursor))
}

fn resolve_merged_pages(
    branch: &str,
    sha: &str,
    mut fetch: impl FnMut(Option<&str>) -> Result<Value, ApiError>,
) -> Result<CleanupSubject, ApiError> {
    let mut value = fetch(None)?;
    let mut matches = Vec::new();
    loop {
        let (nodes, has_next, cursor) = page(&value)?;
        matches.extend(
            nodes
                .into_iter()
                .filter(|item| item.head_ref == branch && item.head_sha == sha),
        );
        if !has_next {
            break;
        }
        value = fetch(cursor.as_deref())?;
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(ApiError::NotFound),
        count => Err(ApiError::Malformed(format!(
            "{count} merged PRs match checked-out branch and HEAD"
        ))),
    }
}

impl CleanupSource for GhCleanupSource {
    fn explicit(&self, number: u64) -> Result<CleanupSubject, ApiError> {
        let (owner, repo) = Self::repository()?;
        let value = Self::query(
            EXPLICIT_QUERY,
            &owner,
            &repo,
            &[GraphQlArgument::Integer(format!("number={number}"))],
        )?;
        explicit_subject(&value)
    }

    fn merged_for(&self, branch: &str, sha: &str) -> Result<CleanupSubject, ApiError> {
        let (owner, repo) = Self::repository()?;
        resolve_merged_pages(branch, sha, |cursor| match cursor {
            None => Self::query(FIRST_PAGE_QUERY, &owner, &repo, &[]),
            Some(cursor) => Self::query(
                NEXT_PAGE_QUERY,
                &owner,
                &repo,
                &[GraphQlArgument::String(format!("cursor={cursor}"))],
            ),
        })
    }
}

/// Production local capability. Every Git process uses the shared environment-
/// scrubbing constructor, so hooks cannot redirect cleanup into another checkout.
pub struct LocalCheckout {
    dir: PathBuf,
}

impl LocalCheckout {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    fn git(&self, args: &[&str]) -> Result<std::process::Output> {
        crate::git::at(&self.dir)
            .args(args)
            .output()
            .with_context(|| format!("running git {}", args.join(" ")))
    }
    fn must_git(&self, args: &[&str]) -> Result<()> {
        let out = self.git(args)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(anyhow!(
                String::from_utf8_lossy(&out.stderr).trim().to_owned()
            ))
        }
    }
}

impl CleanupCheckout for LocalCheckout {
    fn identity(&self) -> Result<CheckoutIdentity> {
        Ok(CheckoutIdentity {
            branch: crate::git::current_branch(&self.dir)?,
            head_sha: crate::git::head_sha(&self.dir)?,
        })
    }
    fn is_dirty(&self) -> Result<bool> {
        let out = self.git(&["status", "--porcelain", "--untracked-files=all"])?;
        if !out.status.success() {
            return Err(anyhow!(
                String::from_utf8_lossy(&out.stderr).trim().to_owned()
            ));
        }
        Ok(crate::git::porcelain_is_dirty(&String::from_utf8_lossy(
            &out.stdout,
        )))
    }
    fn fetch_origin(&self) -> Result<()> {
        self.must_git(&["fetch", "origin"])
    }
    fn head_is_ancestor_of_origin_main(&self, head: &str) -> Result<bool> {
        let out = self.git(&["merge-base", "--is-ancestor", head, "origin/main"])?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(anyhow!(
                String::from_utf8_lossy(&out.stderr).trim().to_owned()
            )),
        }
    }
    fn detach_origin_main(&self) -> Result<()> {
        self.must_git(&["switch", "--detach", "origin/main"])
    }
    fn delete_branch(&self, branch: &str) -> Result<()> {
        self.must_git(&["branch", "-d", "--", branch])
    }
    fn cargo_clean(&self) -> Result<()> {
        let status = Command::new("cargo")
            .arg("clean")
            .current_dir(&self.dir)
            .status()
            .context("running cargo clean")?;
        if status.success() {
            Ok(())
        } else {
            Err(anyhow!("cargo clean exited {status}"))
        }
    }
}

fn fail(result: &mut CommandResult, name: &str, started: Instant, detail: impl Into<String>) {
    result.push(
        StepResult::fail(name)
            .detail(detail)
            .with_duration(started.elapsed()),
    );
}

fn operation(
    result: &mut CommandResult,
    name: &str,
    detail: impl Into<String>,
    run: impl FnOnce() -> Result<()>,
) -> bool {
    let started = Instant::now();
    match run() {
        Ok(()) => {
            result.push(
                StepResult::ok(name)
                    .detail(detail)
                    .with_duration(started.elapsed()),
            );
            true
        }
        Err(error) => {
            fail(result, name, started, format!("{error:#}"));
            false
        }
    }
}

/// Execute cleanup against injectable read-only and local capabilities.
pub fn execute_with<S: CleanupSource, C: CleanupCheckout>(
    source: &S,
    checkout: &C,
    number: Option<u64>,
) -> CommandResult {
    let mut result = CommandResult::new("pr-cleanup");
    let start = Instant::now();
    let identity = match checkout.identity() {
        Ok(value) => value,
        Err(error) => {
            fail(
                &mut result,
                PRECHECK,
                start,
                format!("could not capture checkout identity: {error:#}"),
            );
            return result;
        }
    };
    let (Some(branch), Some(head_sha)) = (identity.branch.as_deref(), identity.head_sha.as_deref())
    else {
        fail(
            &mut result,
            PRECHECK,
            start,
            "a branch and local HEAD must be checked out",
        );
        return result;
    };
    let observed = match number {
        Some(number) => source.explicit(number),
        None => source.merged_for(branch, head_sha),
    };
    let observed = match observed {
        Ok(value) => value,
        Err(error) => {
            fail(
                &mut result,
                PRECHECK,
                start,
                format!("could not establish merged PR evidence: {}", error.detail()),
            );
            return result;
        }
    };
    if let Some(requested) = number.filter(|requested| observed.number != *requested) {
        fail(
            &mut result,
            PRECHECK,
            start,
            format!(
                "GitHub returned PR #{} for requested PR #{requested}",
                observed.number
            ),
        );
        return result;
    }
    let refusal = if observed.state != "MERGED" {
        Some(format!(
            "PR #{} is {}, not merged",
            observed.number, observed.state
        ))
    } else if observed.base_ref != "main" {
        Some(format!(
            "PR #{} targets {}, not main",
            observed.number, observed.base_ref
        ))
    } else if observed.head_ref != branch {
        Some(format!(
            "checked-out branch {branch} does not equal PR head {}",
            observed.head_ref
        ))
    } else if observed.head_sha != head_sha {
        Some(format!(
            "local HEAD {head_sha} does not equal PR head {}",
            observed.head_sha
        ))
    } else {
        match checkout.is_dirty() {
            Ok(true) => Some("working tree has staged, unstaged, or untracked changes".into()),
            Ok(false) => None,
            Err(error) => Some(format!("could not inspect working tree: {error:#}")),
        }
    };
    if let Some(detail) = refusal {
        fail(&mut result, PRECHECK, start, detail);
        return result;
    }
    result.push(
        StepResult::ok(PRECHECK)
            .detail(format!("PR #{}: {branch} @ {head_sha}", observed.number))
            .with_duration(start.elapsed()),
    );
    if !operation(&mut result, FETCH, "fetched origin", || {
        checkout.fetch_origin()
    }) {
        return result;
    }
    if !operation(
        &mut result,
        VERIFY,
        format!("captured head {head_sha} is an ancestor of origin/main"),
        || {
            checkout
                .head_is_ancestor_of_origin_main(head_sha)
                .and_then(|ok| {
                    ok.then_some(()).ok_or_else(|| {
                        anyhow!("captured PR head {head_sha} is not an ancestor of origin/main")
                    })
                })
        },
    ) {
        return result;
    }
    if !operation(&mut result, DETACH, "detached at origin/main", || {
        checkout.detach_origin_main()
    }) {
        return result;
    }
    if !operation(
        &mut result,
        DELETE,
        format!("deleted local branch {branch}"),
        || checkout.delete_branch(branch),
    ) {
        return result;
    }
    operation(&mut result, CLEAN, "ran cargo clean", || {
        checkout.cargo_clean()
    });
    result
}

pub fn execute(number: Option<u64>) -> CommandResult {
    execute_with(&GhCleanupSource, &LocalCheckout::at("."), number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_complete_page_evidence() {
        let (items, next, cursor) = page(&json!({"data":{"repository":{"pullRequests":{"nodes":[{"number":1,"state":"MERGED","baseRefName":"main","headRefName":"topic","headRefOid":"abc"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}})).unwrap();
        assert_eq!(items[0].head_sha, "abc");
        assert!(!next);
        assert_eq!(cursor, None);
    }

    fn merged_page(nodes: Vec<Value>, has_next: bool, cursor: Option<&str>) -> Value {
        json!({"data":{"repository":{"pullRequests":{"nodes":nodes,"pageInfo":{"hasNextPage":has_next,"endCursor":cursor}}}}})
    }

    fn node(number: u64, branch: &str, sha: &str) -> Value {
        json!({"number":number,"state":"MERGED","baseRefName":"main","headRefName":branch,"headRefOid":sha})
    }

    #[test]
    fn merged_resolution_exhausts_pages_for_a_unique_late_match() {
        let mut pages = std::collections::VecDeque::from([
            merged_page(vec![node(1, "other", "other")], true, Some("page-2")),
            merged_page(vec![node(1155, "topic", "abc")], false, None),
        ]);
        let mut cursors = Vec::new();
        let subject = resolve_merged_pages("topic", "abc", |cursor| {
            cursors.push(cursor.map(str::to_owned));
            Ok(pages.pop_front().expect("one request per page"))
        })
        .unwrap();
        assert_eq!(subject.number, 1155);
        assert_eq!(cursors, [None, Some("page-2".into())]);
    }

    #[test]
    fn merged_resolution_rejects_ambiguity_discovered_after_first_page() {
        let mut pages = std::collections::VecDeque::from([
            merged_page(vec![node(1155, "topic", "abc")], true, Some("page-2")),
            merged_page(vec![node(1156, "topic", "abc")], false, None),
        ]);
        let error = resolve_merged_pages("topic", "abc", |_| {
            Ok(pages.pop_front().expect("one request per page"))
        })
        .unwrap_err();
        assert!(matches!(error, ApiError::Malformed(_)));
    }

    #[test]
    fn graphql_transport_marks_pr_number_as_an_integer() {
        let args = graphql_args(
            EXPLICIT_QUERY,
            "jaunder-org",
            "jaunder",
            &[GraphQlArgument::Integer("number=1155".into())],
        );
        assert_eq!(
            args[8..].iter().map(String::as_str).collect::<Vec<_>>(),
            ["-F", "number=1155"]
        );
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = crate::git::at(dir).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-b", "main"]);
        git(
            repo.path(),
            &["config", "user.email", "cleanup@example.test"],
        );
        git(repo.path(), &["config", "user.name", "Cleanup Test"]);
        std::fs::write(repo.path().join("tracked"), "main").unwrap();
        git(repo.path(), &["add", "tracked"]);
        git(repo.path(), &["commit", "-m", "main"]);
        repo
    }

    #[test]
    fn local_fetch_advances_origin_main_without_moving_local_checkout() {
        let checkout_dir = repository();
        let origin_dir = tempfile::tempdir().unwrap();
        let origin = origin_dir.path().join("origin.git");
        git(
            origin_dir.path(),
            &[
                "init",
                "--bare",
                "-b",
                "main",
                origin.file_name().unwrap().to_str().unwrap(),
            ],
        );
        git(
            checkout_dir.path(),
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(checkout_dir.path(), &["push", "-u", "origin", "main"]);
        let local_main_before = crate::git::head_sha(checkout_dir.path()).unwrap().unwrap();

        let actor_root = tempfile::tempdir().unwrap();
        let actor = actor_root.path().join("actor");
        git(
            actor_root.path(),
            &[
                "clone",
                origin.to_str().unwrap(),
                actor.file_name().unwrap().to_str().unwrap(),
            ],
        );
        git(&actor, &["config", "user.email", "cleanup@example.test"]);
        git(&actor, &["config", "user.name", "Cleanup Test"]);
        std::fs::write(actor.join("tracked"), "advanced").unwrap();
        git(&actor, &["commit", "-am", "advance main"]);
        git(&actor, &["push", "origin", "main"]);
        let remote_main = crate::git::head_sha(&actor).unwrap().unwrap();

        let checkout = LocalCheckout::at(checkout_dir.path());
        let identity_before = checkout.identity().unwrap();
        checkout.fetch_origin().unwrap();
        assert_eq!(
            crate::git::head_sha(checkout_dir.path()).unwrap().unwrap(),
            local_main_before
        );
        assert_eq!(checkout.identity().unwrap(), identity_before);
        let fetched = crate::git::at(checkout_dir.path())
            .args(["rev-parse", "refs/remotes/origin/main"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&fetched.stdout).trim(), remote_main);
    }

    #[test]
    fn local_checkout_proves_ancestry_detaches_and_safely_deletes() {
        let repo = repository();
        git(repo.path(), &["branch", "topic"]);
        git(repo.path(), &["switch", "topic"]);
        std::fs::write(repo.path().join("tracked"), "topic").unwrap();
        git(repo.path(), &["commit", "-am", "topic"]);
        let topic = crate::git::head_sha(repo.path()).unwrap().unwrap();
        git(repo.path(), &["switch", "-c", "main-next"]);
        std::fs::write(repo.path().join("tracked"), "origin main").unwrap();
        git(repo.path(), &["commit", "-am", "advance origin main"]);
        let origin_main = crate::git::head_sha(repo.path()).unwrap().unwrap();
        git(repo.path(), &["switch", "topic"]);
        git(repo.path(), &["switch", "--orphan", "unrelated"]);
        std::fs::write(repo.path().join("tracked"), "unrelated").unwrap();
        git(repo.path(), &["add", "tracked"]);
        git(repo.path(), &["commit", "-m", "unrelated"]);
        let unrelated = crate::git::head_sha(repo.path()).unwrap().unwrap();
        git(repo.path(), &["switch", "topic"]);
        git(
            repo.path(),
            &["update-ref", "refs/remotes/origin/main", &origin_main],
        );
        let checkout = LocalCheckout::at(repo.path());
        assert!(checkout.head_is_ancestor_of_origin_main(&topic).unwrap());
        assert!(
            !checkout
                .head_is_ancestor_of_origin_main(&unrelated)
                .unwrap()
        );
        checkout.detach_origin_main().unwrap();
        let detached = checkout.identity().unwrap();
        assert_eq!(detached.branch, None);
        assert_eq!(detached.head_sha.as_deref(), Some(origin_main.as_str()));
        assert_ne!(detached.head_sha.as_deref(), Some(topic.as_str()));
        checkout.delete_branch("topic").unwrap();
        assert!(
            crate::git::at(repo.path())
                .args(["show-ref", "--verify", "--quiet", "refs/heads/topic"])
                .status()
                .unwrap()
                .code()
                == Some(1)
        );
    }

    #[test]
    fn local_checkout_reports_hidden_untracked_files_and_refuses_cleanup() {
        let repo = repository();
        git(repo.path(), &["config", "status.showUntrackedFiles", "no"]);
        std::fs::write(repo.path().join("untracked"), "dirty").unwrap();
        let checkout = LocalCheckout::at(repo.path());
        assert!(checkout.is_dirty().unwrap());
        let result = execute_with(
            &FakeSource {
                subject: Ok(CleanupSubject {
                    number: 1155,
                    state: "MERGED".into(),
                    base_ref: "main".into(),
                    head_ref: "main".into(),
                    head_sha: crate::git::head_sha(repo.path()).unwrap().unwrap(),
                }),
            },
            &checkout,
            Some(1155),
        );
        assert!(!result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            [PRECHECK]
        );

        std::fs::remove_file(repo.path().join("untracked")).unwrap();
        git(repo.path(), &["branch", "topic"]);
        let linked = repo.path().join("linked");
        git(
            repo.path(),
            &["worktree", "add", linked.to_str().unwrap(), "topic"],
        );
        assert!(checkout.delete_branch("topic").is_err());
    }
    #[derive(Clone)]
    struct FakeSource {
        subject: Result<CleanupSubject, ApiError>,
    }

    impl CleanupSource for FakeSource {
        fn explicit(&self, _: u64) -> Result<CleanupSubject, ApiError> {
            self.subject.clone()
        }

        fn merged_for(&self, _: &str, _: &str) -> Result<CleanupSubject, ApiError> {
            self.subject.clone()
        }
    }

    struct FakeCheckout {
        identity: CheckoutIdentity,
        dirty: Result<bool>,
        fail_at: Option<&'static str>,
        calls: std::cell::RefCell<Vec<String>>,
        verified_heads: std::cell::RefCell<Vec<String>>,
        deleted_branches: std::cell::RefCell<Vec<String>>,
    }

    impl FakeCheckout {
        fn clean() -> Self {
            Self {
                identity: CheckoutIdentity {
                    branch: Some("topic".into()),
                    head_sha: Some("abc".into()),
                },
                dirty: Ok(false),
                fail_at: None,
                calls: std::cell::RefCell::new(Vec::new()),
                verified_heads: std::cell::RefCell::new(Vec::new()),
                deleted_branches: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn call(&self, name: &'static str) -> Result<()> {
            self.calls.borrow_mut().push(name.into());
            if self.fail_at == Some(name) {
                Err(anyhow!("{name} failed"))
            } else {
                Ok(())
            }
        }
    }

    impl CleanupCheckout for FakeCheckout {
        fn identity(&self) -> Result<CheckoutIdentity> {
            Ok(self.identity.clone())
        }
        fn is_dirty(&self) -> Result<bool> {
            self.dirty
                .as_ref()
                .map(|value| *value)
                .map_err(|error| anyhow!("{error:#}"))
        }
        fn fetch_origin(&self) -> Result<()> {
            self.call(FETCH)
        }
        fn head_is_ancestor_of_origin_main(&self, head: &str) -> Result<bool> {
            self.verified_heads.borrow_mut().push(head.into());
            self.call(VERIFY).map(|()| true)
        }
        fn detach_origin_main(&self) -> Result<()> {
            self.call(DETACH)
        }
        fn delete_branch(&self, branch: &str) -> Result<()> {
            self.deleted_branches.borrow_mut().push(branch.into());
            self.call(DELETE)
        }
        fn cargo_clean(&self) -> Result<()> {
            self.call(CLEAN)
        }
    }

    fn merged_subject() -> CleanupSubject {
        CleanupSubject {
            number: 1155,
            state: "MERGED".into(),
            base_ref: "main".into(),
            head_ref: "topic".into(),
            head_sha: "abc".into(),
        }
    }

    #[test]
    fn execution_orders_every_boundary_and_preserves_captured_identity() {
        let source = FakeSource {
            subject: Ok(merged_subject()),
        };
        let checkout = FakeCheckout::clean();
        let result = execute_with(&source, &checkout, Some(1155));
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            [PRECHECK, FETCH, VERIFY, DETACH, DELETE, CLEAN]
        );
        assert_eq!(
            checkout
                .calls
                .borrow()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [FETCH, VERIFY, DETACH, DELETE, CLEAN]
        );
        assert_eq!(*checkout.verified_heads.borrow(), ["abc"]);
        assert_eq!(*checkout.deleted_branches.borrow(), ["topic"]);
    }

    #[test]
    fn preconditions_refuse_without_local_mutation() {
        let cases = [
            CleanupSubject {
                state: "OPEN".into(),
                ..merged_subject()
            },
            CleanupSubject {
                base_ref: "release".into(),
                ..merged_subject()
            },
            CleanupSubject {
                head_ref: "other".into(),
                ..merged_subject()
            },
            CleanupSubject {
                head_sha: "other".into(),
                ..merged_subject()
            },
        ];
        for subject in cases {
            let checkout = FakeCheckout::clean();
            let result = execute_with(
                &FakeSource {
                    subject: Ok(subject),
                },
                &checkout,
                Some(1155),
            );
            assert!(!result.ok);
            assert_eq!(result.steps.len(), 1);
            assert!(checkout.calls.borrow().is_empty());
        }
        let mut detached = FakeCheckout::clean();
        detached.identity.branch = None;
        assert!(
            !execute_with(
                &FakeSource {
                    subject: Ok(merged_subject())
                },
                &detached,
                Some(1155)
            )
            .ok
        );
        let mut dirty = FakeCheckout::clean();
        dirty.dirty = Ok(true);
        assert!(
            !execute_with(
                &FakeSource {
                    subject: Ok(merged_subject())
                },
                &dirty,
                Some(1155)
            )
            .ok
        );
        assert!(dirty.calls.borrow().is_empty());
    }

    #[test]
    fn operation_failure_stops_at_its_boundary() {
        for failed in [FETCH, VERIFY, DETACH, DELETE, CLEAN] {
            let mut checkout = FakeCheckout::clean();
            checkout.fail_at = Some(failed);
            let result = execute_with(
                &FakeSource {
                    subject: Ok(merged_subject()),
                },
                &checkout,
                Some(1155),
            );
            let names = result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(names.last(), Some(&failed));
            assert!(!result.ok);
            assert_eq!(
                checkout.calls.borrow().last().map(String::as_str),
                Some(failed)
            );
        }
    }
    #[test]
    fn rejects_incomplete_page_evidence() {
        assert!(page(&json!({"data":{"repository":{"pullRequests":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":null}}}}})).is_err());
        assert!(
            subject(
                &json!({"number":1,"state":"MERGED","baseRefName":"main","headRefName":"topic"})
            )
            .is_err()
        );
    }
}
