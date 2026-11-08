"""Argv Git operations scoped to an explicit checkout, including fixture repos."""

import os
import subprocess
from pathlib import Path


REDIRECTS = (
    "GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR", "GIT_NAMESPACE",
)


def environment() -> dict[str, str]:
    # Hook-exported Git variables must never redirect an operation to another repo.
    clean = os.environ.copy()
    for name in REDIRECTS:
        clean.pop(name, None)
    return clean


def process(repo: Path, *arguments: str):
    return subprocess.run(
        ["git", "-C", str(repo), *arguments], env=environment(),
        capture_output=True, check=False,
    )


def run(repo: Path, *arguments: str, allowed=(0,)) -> bytes:
    result = process(repo, *arguments)
    if result.returncode not in allowed:
        raise RuntimeError(
            f"git {arguments!r} failed ({result.returncode}): "
            + result.stderr.decode("utf-8", errors="replace").strip()
        )
    return result.stdout


def paths(repo: Path, *arguments: str, allowed=(0,)) -> list[str]:
    return [os.fsdecode(path) for path in run(repo, *arguments, allowed=allowed).split(b"\x00") if path]


def prepare_main(repo, base):
    if run(repo, "status", "--porcelain").strip():
        raise RuntimeError("ADR promoter requires a clean working tree")
    fetch(repo, base)
    run(repo, "switch", "--detach", f"origin/{base}")


def fetch(repo, reference):
    run(repo, "fetch", "origin", reference)


def refresh_main(repo, base):
    fetch(repo, base)
    return run(repo, "rev-parse", f"origin/{base}").decode().strip()


def sole_parent(repo, commit):
    parents = run(repo, "show", "-s", "--format=%P", commit).decode().split()
    return parents[0] if len(parents) == 1 else None


def _binary_result(repo, arguments):
    result = process(repo, *arguments)
    if result.returncode not in (0, 1):
        raise RuntimeError(f"git {arguments!r} failed ({result.returncode}): " + result.stderr.decode(errors="replace"))
    return result.returncode


def strict_ancestor(repo, ancestor, descendant):
    return ancestor != descendant and _binary_result(repo, ("merge-base", "--is-ancestor", ancestor, descendant)) == 0


def merge_conflicts(repo, main, head):
    # Only Git's documented conflict exit is authorization; not process failure.
    return _binary_result(repo, ("merge-tree", "--write-tree", main, head)) == 1


def delete_remote(repo, branch, expected):
    run(repo, "push", f"--force-with-lease=refs/heads/{branch}:{expected}", "origin", f":refs/heads/{branch}")


def staged_diff(repo):
    return bool(run(repo, "diff", "--cached", "--name-only").strip())


def format_staged_markdown(repo):
    changed = paths(repo, "diff", "--cached", "--name-only", "-z", "--diff-filter=ACMR", "--", "*.md")
    if not changed:
        return
    result = subprocess.run(["prettier", "--write", *changed], cwd=repo, env=environment(), check=False)
    if result.returncode != 0:
        raise RuntimeError(f"Prettier failed for promoted Markdown ({result.returncode})")
    run(repo, "add", "--", *changed)


def commit_promotion(repo, bot, title):
    # The generated rename is incompatible with precommit's restaging owner;
    # ordinary required PR/queue checks still gate this deterministic App commit.
    run(repo, "-c", f"user.name={bot}", "-c", f"user.email={bot}@users.noreply.github.com",
        "commit", "--no-verify", "-m", title)


def head_sha(repo):
    head = run(repo, "rev-parse", "HEAD").decode().strip()
    if not head:
        raise RuntimeError("promoter commit has no HEAD")
    return head


def push_promoter(repo, branch):
    run(repo, "push", "origin", f"HEAD:refs/heads/{branch}")
