"""CLI outcomes and package mutation wiring in nonprivileged temporary Git."""

import json
import os
import subprocess
import sys

import pytest
from github import GithubException

from adr_promoter import __main__, controller
from test_promotion import OLD, README, README_SYNCED, git, tracked_repo


def invoke(root, *arguments):
    environment = os.environ.copy()
    environment["PYTHONPATH"] = os.pathsep.join(sys.path)
    environment["GITHUB_EVENT_PATH"] = str(root / "event.json")
    environment["GITHUB_REPOSITORY"] = "fixture/repo"
    for token in ("GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"):
        environment.pop(token, None)
    return subprocess.run([sys.executable, "-m", "adr_promoter", *arguments],
                          capture_output=True, text=True, env=environment, cwd=root, check=False)


def test_controller_ignored_event_exits_zero_without_token_or_mutation(tmp_path):
    event = {"action": "opened", "pull_request": {"number": 7,
             "head": {"ref": "automation/adr-promoter", "sha": "head"},
             "base": {"ref": "main", "sha": "base"}}}
    tracked_repo(tmp_path, {"event.json": json.dumps(event).encode()})
    result = invoke(tmp_path, "controller")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "event does not target the ADR promoter\n"


def test_cli_sync_readme_operates_on_current_fixture_root(tmp_path):
    tracked_repo(tmp_path, {"docs/adr/0001-old.md": OLD, "docs/README.md": README})
    result = invoke(tmp_path, "sync-readme")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "1 rows (0 added, 0 removed)\n"
    assert (tmp_path / "docs/README.md").read_bytes() == README_SYNCED


def test_cli_refuses_promote_action(tmp_path):
    result = invoke(tmp_path, "promote")
    assert result.returncode == 2
    assert "invalid choice: 'promote'" in result.stderr


def test_cli_mutation_read_failure_is_visible_nonzero(tmp_path):
    git(tmp_path, "init", "-q")
    result = invoke(tmp_path, "sync-readme")
    assert result.returncode == 1 and "docs/README.md" in result.stderr
    assert result.stdout == ""


def test_operational_sdk_error_exits_nonzero(monkeypatch, tmp_path, capsys):
    tracked_repo(tmp_path, {"fixture.txt": b"seed\n"})
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sys, "argv", ["adr-promoter", "controller"])
    def fail(repo):
        raise GithubException(503, {"message": "unavailable"})
    monkeypatch.setattr(controller, "execute", fail)
    with pytest.raises(SystemExit) as failure:
        __main__.main()
    assert failure.value.code == 1
    output = capsys.readouterr()
    assert not output.out and "unavailable" in output.err
