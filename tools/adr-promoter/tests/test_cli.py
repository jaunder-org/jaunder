"""A tracer must not report successful privileged actions it cannot perform."""

import json
import os
import shutil
import subprocess
import sys
from test_promotion import promotion_fixture, ALPHA_EXPECTED, README_EXPECTED, git, tracked_repo


def invoke(root, *arguments):
    environment = os.environ.copy()
    # Carry the offline check environment's selected dependency paths to children.
    environment["PYTHONPATH"] = os.pathsep.join(sys.path)
    environment["PATH"] = str(root / "bin") + os.pathsep + environment["PATH"]
    environment["GITHUB_EVENT_PATH"] = str(root / "event.json")
    environment["GITHUB_REPOSITORY"] = "fixture/repo"
    for token in ("GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"):
        environment.pop(token, None)
    return subprocess.run(
        [sys.executable, "-m", "adr_promoter", *arguments],
        capture_output=True,
        text=True,
        env=environment,
        cwd=root,
        check=False,
    )


def test_help_identifies_the_package(tmp_path):
    result = invoke(tmp_path, "--help")
    assert result.returncode == 0
    assert "adr-promoter" in result.stdout
    assert "controller" in result.stdout


def controller_fixture(root, action, code, stdout, event=None):
    real_git = shutil.which("git")
    if event is None:
        event = json.dumps({"action": action, "pull_request": {"number": 7,
            "head": {"ref": "automation/adr-promoter", "sha": "head"},
            "base": {"ref": "main", "sha": "base"}}}).encode()
    gh = f'''#!{sys.executable}
import json, pathlib, sys
root = pathlib.Path(__file__).resolve().parents[1]
with (root / "gh.calls").open("a") as calls:
    calls.write(json.dumps(sys.argv[1:]) + "\\n")
print({stdout!r})
sys.exit({code})
'''.encode()
    local_git = f'''#!{sys.executable}
import os, sys
arguments = sys.argv[1:]
command = arguments[2:] if arguments[:1] == ["-C"] else arguments
if not command or command[0] not in ("rev-parse", "ls-files"):
    sys.exit("fixture blocked unexpected Git operation")
os.execv({real_git!r}, [{real_git!r}, *arguments])
'''.encode()
    tracked_repo(root, {"event.json": event, "bin/gh": gh, "bin/git": local_git})
    (root / "bin/gh").chmod(0o755)
    (root / "bin/git").chmod(0o755)


def test_controller_ignored_event_exits_zero_without_gh_or_mutation(tmp_path):
    controller_fixture(tmp_path, "opened", 91, "unexpected gh invocation")
    result = invoke(tmp_path, "controller")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "event does not target the ADR promoter\n"
    assert not (tmp_path / "gh.calls").exists()


def test_unknown_action_is_an_argument_error(tmp_path):
    result = invoke(tmp_path, "unknown")
    assert result.returncode == 2
    assert "invalid choice" in result.stderr


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
    assert result.returncode == 1
    assert "docs/README.md" in result.stderr
    assert result.stdout == ""


def test_controller_not_rearmed_is_success_not_an_operational_error(tmp_path):
    controller_fixture(tmp_path, "dequeued", 1, '{"status":"404"}')
    result = invoke(tmp_path, "controller")
    assert result.returncode == 0, result.stderr
    assert result.stdout == "promoter PR not re-armed: pull request is absent\n"
    assert json.loads((tmp_path / "gh.calls").read_text()) == ["api", "/repos/fixture/repo/pulls/7"]


def test_controller_api_failure_is_visible_and_nonzero(tmp_path):
    controller_fixture(tmp_path, "dequeued", 0, '{"errors":[{"message":"bad query"}]}')
    result = invoke(tmp_path, "controller")
    assert result.returncode == 1
    assert "graphql" in result.stderr and result.stdout == ""


def test_controller_malformed_event_fails_before_any_api(tmp_path):
    controller_fixture(tmp_path, "dequeued", 91, "unexpected gh", event=b"{broken")
    result = invoke(tmp_path, "controller")
    assert result.returncode == 1
    assert result.stdout == ""
    assert not (tmp_path / "gh.calls").exists()


def test_nonstandard_json_event_cannot_fall_through_to_generation(tmp_path):
    controller_fixture(tmp_path, "dequeued", 91, "unexpected gh", event=b'{"extra":NaN}')
    result = invoke(tmp_path, "controller")
    assert result.returncode == 1 and "invalid JSON constant" in result.stderr
    assert result.stdout == "" and not (tmp_path / "gh.calls").exists()
