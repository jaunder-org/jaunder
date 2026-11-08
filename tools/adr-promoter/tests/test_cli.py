"""CLI outcomes and package mutation wiring in nonprivileged temporary Git."""

import json
import os
import subprocess
import sys

import pytest
from github import GithubException

from adr_promoter import __main__, controller
from test_promotion import promotion_fixture, ALPHA_EXPECTED, README_EXPECTED, git, tracked_repo


def invoke(root, *arguments):
    environment = os.environ.copy()
    environment["PYTHONPATH"] = os.pathsep.join(sys.path)
    environment["GITHUB_EVENT_PATH"] = str(root / "event.json")
    environment["GITHUB_REPOSITORY"] = "fixture/repo"
    for token in ("GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"):
        environment.pop(token, None)
    return subprocess.run([sys.executable, "-m", "adr_promoter", *arguments],
                          capture_output=True, text=True, env=environment, cwd=root, check=False)


def test_help_and_unknown_action(tmp_path):
    result = invoke(tmp_path, "--help")
    assert result.returncode == 0 and "controller" in result.stdout
    assert invoke(tmp_path, "unknown").returncode == 2


def test_controller_ignored_event_exits_zero_without_token_or_mutation(tmp_path):
    event = {"action": "opened", "pull_request": {"number": 7,
             "head": {"ref": "automation/adr-promoter", "sha": "head"},
             "base": {"ref": "main", "sha": "base"}}}
    tracked_repo(tmp_path, {"event.json": json.dumps(event).encode()})
    result = invoke(tmp_path, "controller")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "event does not target the ADR promoter\n"


def test_cli_mutations_operate_on_current_fixture_root(tmp_path):
    promotion_fixture(tmp_path)
    result = invoke(tmp_path, "promote")
    assert result.returncode == 0, result.stderr
    assert "staged" in result.stdout
    assert (tmp_path / "docs/adr/0002-alpha.md").read_bytes() == ALPHA_EXPECTED
    assert (tmp_path / "docs/README.md").read_bytes() == README_EXPECTED
    result = invoke(tmp_path, "sync-readme")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "3 rows, already in sync\n"


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


@pytest.mark.parametrize("event", [b"{broken", b'{"extra":NaN}'])
def test_malformed_event_fails_before_any_api(tmp_path, event):
    tracked_repo(tmp_path, {"event.json": event})
    result = invoke(tmp_path, "controller")
    assert result.returncode == 1 and result.stdout == ""
