"""Controller Git/formatter argv proof using fake children only."""

import subprocess

import pytest

from adr_promoter import git


def test_lease_names_the_exact_observed_oid(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(git, "run", lambda repo, *args, **kwargs: calls.append(args) or b"")
    git.delete_remote(tmp_path, "automation/adr-promoter", "observed")
    assert calls == [("push", "--force-with-lease=refs/heads/automation/adr-promoter:observed",
                      "origin", ":refs/heads/automation/adr-promoter")]


def test_formatter_receives_only_staged_markdown_and_restages_after_success(monkeypatch, tmp_path):
    calls = []
    def run(repo, *args, **kwargs):
        calls.append(args)
        return b"docs/one.md\x00docs/space name.md\x00" if args[0] == "diff" else b""
    def process(arguments, **kwargs):
        calls.append(tuple(arguments))
        assert kwargs["cwd"] == tmp_path
        return subprocess.CompletedProcess(arguments, 0)
    monkeypatch.setattr(git, "run", run)
    monkeypatch.setattr(subprocess, "run", process)
    git.format_staged_markdown(tmp_path)
    assert calls == [
        ("diff", "--cached", "--name-only", "-z", "--diff-filter=ACMR", "--", "*.md"),
        ("prettier", "--write", "docs/one.md", "docs/space name.md"),
        ("add", "--", "docs/one.md", "docs/space name.md"),
    ]


def test_dirty_generation_refuses_before_fetch_or_switch(monkeypatch, tmp_path):
    calls = []
    def run(repo, *arguments, **kwargs):
        calls.append(arguments)
        return b"?? dirty.md\n"
    monkeypatch.setattr(git, "run", run)
    with pytest.raises(RuntimeError, match="clean working tree"):
        git.prepare_main(tmp_path, "main")
    assert calls == [("status", "--porcelain")]


def test_generation_selects_fresh_main_and_deterministic_app_commit(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(git, "run", lambda repo, *args, **kwargs: calls.append(args) or b"")
    git.prepare_main(tmp_path, "main")
    git.commit_promotion(tmp_path, "jaunder-adr-promoter[bot]", "docs(adr): promote pending ADR drafts")
    git.push_promoter(tmp_path, "automation/adr-promoter")
    assert calls == [
        ("status", "--porcelain"), ("fetch", "origin", "main"), ("switch", "--detach", "origin/main"),
        ("-c", "user.name=jaunder-adr-promoter[bot]", "-c",
         "user.email=jaunder-adr-promoter[bot]@users.noreply.github.com", "commit", "--no-verify", "-m",
         "docs(adr): promote pending ADR drafts"),
        ("push", "origin", "HEAD:refs/heads/automation/adr-promoter"),
    ]


def test_no_markdown_skips_formatter_and_restage(monkeypatch, tmp_path):
    monkeypatch.setattr(git, "run", lambda *args, **kwargs: b"")
    monkeypatch.setattr(subprocess, "run", lambda *args, **kwargs: pytest.fail("unexpected formatter"))
    git.format_staged_markdown(tmp_path)


def test_formatter_failure_never_restages(monkeypatch, tmp_path):
    calls = []
    def run(repo, *arguments, **kwargs):
        calls.append(arguments)
        return b"one.md\x00"
    monkeypatch.setattr(git, "run", run)
    monkeypatch.setattr(subprocess, "run", lambda args, **kwargs: subprocess.CompletedProcess(args, 3))
    with pytest.raises(RuntimeError, match="Prettier failed"):
        git.format_staged_markdown(tmp_path)
    assert len(calls) == 1 and calls[0][0] == "diff"


@pytest.mark.parametrize("code", [2, 128])
@pytest.mark.parametrize("operation", ["merge", "ancestor"])
def test_git_errors_cannot_be_misclassified_as_conflict_proof(monkeypatch, tmp_path, code, operation):
    monkeypatch.setattr(git, "process", lambda *args: subprocess.CompletedProcess(args, code, b"", b"fixture error"))
    with pytest.raises(RuntimeError, match="fixture error"):
        if operation == "merge":
            git.merge_conflicts(tmp_path, "main", "head")
        else:
            git.strict_ancestor(tmp_path, "old", "main")


def test_real_disposable_git_proves_parent_ancestry_and_exact_merge(tmp_path):
    from test_promotion import tracked_repo, write, git as command
    tracked_repo(tmp_path, {"value.txt": b"base\n"})
    base = command(tmp_path, "rev-parse", "HEAD").decode().strip()
    command(tmp_path, "switch", "-c", "left")
    write(tmp_path, "value.txt", b"left\n")
    command(tmp_path, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c",
            "core.hooksPath=/dev/null", "commit", "-qam", "left")
    left = command(tmp_path, "rev-parse", "HEAD").decode().strip()
    command(tmp_path, "switch", "--detach", base)
    command(tmp_path, "switch", "-c", "right")
    write(tmp_path, "value.txt", b"right\n")
    command(tmp_path, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c",
            "core.hooksPath=/dev/null", "commit", "-qam", "right")
    right = command(tmp_path, "rev-parse", "HEAD").decode().strip()
    assert git.sole_parent(tmp_path, right) == base
    assert git.strict_ancestor(tmp_path, base, left)
    assert not git.strict_ancestor(tmp_path, left, left)
    assert git.merge_conflicts(tmp_path, left, right)
    assert not git.merge_conflicts(tmp_path, base, right)


def test_real_local_bare_ref_lease_refuses_changed_oid_then_verifies_absence(tmp_path):
    from test_promotion import tracked_repo, git as command
    local = tmp_path / "local"
    local.mkdir()
    tracked_repo(local, {"value.txt": b"fixture\n"})
    remote = tmp_path / "remote.git"
    command(local, "init", "--bare", str(remote))
    command(local, "remote", "add", "origin", str(remote))
    head = command(local, "rev-parse", "HEAD").decode().strip()
    command(local, "push", "origin", "HEAD:refs/heads/automation/adr-promoter")
    with pytest.raises(RuntimeError):
        git.delete_remote(local, "automation/adr-promoter", "0" * 40)
    assert command(local, "ls-remote", "--heads", "origin", "refs/heads/automation/adr-promoter").split()[0].decode() == head
    git.delete_remote(local, "automation/adr-promoter", head)
    assert command(local, "ls-remote", "--heads", "origin", "refs/heads/automation/adr-promoter") == b""
