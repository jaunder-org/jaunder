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
    ("ancestor", ("merge-base", "--is-ancestor", "old", "main"), True),
    ("cites", ("grep", "-q", "--fixed-strings", "-e", "drafts/a", "-e", "drafts/b", "tree"), True),
    ("merge", ("merge-tree", "--write-tree", "main", "head"), "tree-oid"),
])
def test_probe_uses_exact_objects_and_interprets_exit_status(monkeypatch, tmp_path, code, operation, arguments, positive):
    calls = []
    def process(repo, *args):
        assert repo == tmp_path
        calls.append(args)
        return subprocess.CompletedProcess(args, code, b"tree-oid\nconflict info\n", b"fixture error")
    monkeypatch.setattr(git, "process", process)
    invoke = {
        "ancestor": lambda: git.strict_ancestor(tmp_path, "old", "main"),
        "cites": lambda: git.tree_contains(tmp_path, "tree", ["drafts/a", "drafts/b"]),
        "merge": lambda: git.merged_tree(tmp_path, "main", "head"),
    }[operation]
    if code == 128:
        with pytest.raises(RuntimeError, match="fixture error"):
            invoke()
    else:
        negative = None if operation == "merge" else False
        assert invoke() == (positive if code == 0 else negative)
    assert calls == [arguments]


def test_no_promoted_drafts_or_unmoved_main_short_circuit_without_running_git(monkeypatch, tmp_path):
    monkeypatch.setattr(git, "process", lambda *args: pytest.fail("unexpected git"))
    assert git.tree_contains(tmp_path, "tree", []) is False
    assert git.strict_ancestor(tmp_path, "main", "main") is False


def test_clean_merge_without_a_tree_is_an_error(monkeypatch, tmp_path):
    monkeypatch.setattr(git, "process", lambda repo, *args: subprocess.CompletedProcess(args, 0, b"\n", b""))
    with pytest.raises(RuntimeError, match="no tree"):
        git.merged_tree(tmp_path, "main", "head")


def test_deleted_drafts_exclude_rename_destinations(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setattr(git, "run", lambda repo, *args, **kwargs: calls.append(args) or b"docs/adr/drafts/a.md\x00")
    assert git.deleted_paths(tmp_path, "parent", "head", "docs/adr/drafts/") == ["docs/adr/drafts/a.md"]
    assert calls == [("diff", "--no-renames", "--diff-filter=D", "--name-only", "-z", "parent", "head",
                      "--", "docs/adr/drafts/")]


def test_stale_proof_detects_a_citation_main_added_after_the_promotion(tmp_path):
    """Our composition (deleted drafts -> stems -> merged-tree grep) on real objects."""
    from adr_promoter import controller
    from adr_promoter.promotion import promote
    from test_promotion import git as fixture_git, tracked_repo, write
    commit = ("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
              "-c", "core.hooksPath=/dev/null", "commit", "-qm")
    tracked_repo(tmp_path, {
        "docs/adr/0001-old.md": b"# ADR-0001: Old\n\n- Status: accepted\n",
        "docs/adr/drafts/alpha.md": b"# ADR-DRAFT: Alpha\n\n- Status: proposed\n",
        "docs/ARCHITECTURE.md": b"See [alpha](adr/drafts/alpha.md).\n",
    })
    fixture_git(tmp_path, "branch", "-M", "main")
    fixture_git(tmp_path, "remote", "add", "origin", str(tmp_path))
    fixture_git(tmp_path, "switch", "-q", "--detach")
    promote(tmp_path)
    fixture_git(tmp_path, *commit, "promote")
    head = fixture_git(tmp_path, "rev-parse", "HEAD").decode().strip()
    fixture_git(tmp_path, "switch", "-q", "main")
    write(tmp_path, "docs/other.md", b"Unrelated.\n")
    fixture_git(tmp_path, "add", ".")
    fixture_git(tmp_path, *commit, "unrelated")
    assert controller._stale(tmp_path, head) is False
    write(tmp_path, "docs/adr/drafts/beta.md", b"Cites [alpha](../drafts/alpha.md).\n")
    fixture_git(tmp_path, "add", ".")
    fixture_git(tmp_path, *commit, "cite")
    assert controller._stale(tmp_path, head) is True
