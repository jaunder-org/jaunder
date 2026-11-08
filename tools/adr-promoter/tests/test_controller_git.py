"""Our Git/formatter argument selection, exit interpretation and refusal policy."""

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


@pytest.mark.parametrize("code", [0, 1, 128])
@pytest.mark.parametrize("operation,arguments,positive", [
    ("merge", ("merge-tree", "--write-tree", "main", "head"), 1),
    ("ancestor", ("merge-base", "--is-ancestor", "old", "main"), 0),
])
def test_conflict_proof_uses_exact_objects_and_interprets_exit_status(monkeypatch, tmp_path, code, operation, arguments, positive):
    calls = []
    def process(repo, *args):
        assert repo == tmp_path
        calls.append(args)
        return subprocess.CompletedProcess(args, code, b"", b"fixture error")
    monkeypatch.setattr(git, "process", process)
    invoke = (lambda: git.merge_conflicts(tmp_path, "main", "head")) if operation == "merge" else (
        lambda: git.strict_ancestor(tmp_path, "old", "main"))
    if code == 128:
        with pytest.raises(RuntimeError, match="fixture error"):
            invoke()
    else:
        assert invoke() is (code == positive)
    assert calls == [arguments]
