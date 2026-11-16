"""Qualification preflight: clean tree, resolved commits, origin, one refetch."""

import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from conftest import HARNESS, SOURCE, TARGET, Checkout, Workflow

CLEAN = ["git", "status", "--porcelain", "--untracked-files=all"]
REFETCH = ["git", "fetch", "--quiet", "--no-write-fetch-head", "--refetch", "origin"]
HOST = ["nix", "run", ".#qualification-host", "--"]


def resolve(revision: str) -> list[str]:
    return [
        "git",
        "rev-parse",
        "--verify",
        "--end-of-options",
        f"{revision}^{{commit}}",
    ]


HARNESS_QUERY = ["git", "rev-parse", "--verify", "HEAD^{commit}"]
ORIGIN = ["git", "remote", "get-url", "origin"]


def test_acceptance_fetches_the_complete_distinct_set_once(workflow: Workflow) -> None:
    result = workflow.dev("production-baseline-accept", "source", "target")
    assert result.returncode == 0, result
    assert workflow.commands() == [
        CLEAN,
        HARNESS_QUERY,
        resolve("source"),
        resolve("target"),
        ORIGIN,
        [*REFETCH, HARNESS, SOURCE, TARGET],
        [*HOST, "--expected-harness", HARNESS, "--source", SOURCE, "--target", TARGET],
    ]


def test_acceptance_refuses_an_empty_target_before_any_work(
    workflow: Workflow,
) -> None:
    result = workflow.dev("production-baseline-accept", "HEAD", "")
    assert result.returncode == 1
    assert result.stderr == (
        b"Error: production-baseline accept requires a nonempty target\n"
    )
    assert workflow.commands() == []
    assert workflow.checkout.runs() == []


@pytest.mark.parametrize("marker", ["dirty-status", "wrong-origin"])
def test_preflight_refuses_before_fetch_or_host(
    workflow: Workflow, marker: str
) -> None:
    (workflow.root / marker).write_text("refuse")
    result = workflow.dev("production-baseline-discover", "source")
    assert result.returncode == 1, result
    assert not any(
        argv[0] == "nix" or argv[1] == "fetch" for argv in workflow.commands()
    )
    # The refusal belongs to the step whose output it judged.
    step = {"dirty-status": "clean-tree", "wrong-origin": "origin"}[marker]
    attributes = workflow.checkout.merged()[step]["args"]
    assert (attributes["jaunder.status"], attributes["process.exit.code"]) == (
        "failed",
        1,
    )


def test_identical_resolved_source_and_target_or_a_failed_fetch_refuse(
    workflow: Workflow,
) -> None:
    result = workflow.dev(
        "production-baseline-accept", "source", "source", TARGET=SOURCE
    )
    assert result.returncode == 1
    assert b"distinct resolved" in result.stderr
    assert not any(argv[0] == "nix" for argv in workflow.commands())
    target = workflow.checkout.merged()["target"]["args"]
    assert (target["jaunder.status"], target["process.exit.code"]) == ("failed", 1)
    result = workflow.dev(
        "production-baseline-discover",
        "source",
        failure=" ".join([*REFETCH, HARNESS, SOURCE]),
    )
    assert result.returncode == 17
    assert not any(argv[0] == "nix" for argv in workflow.commands())


def test_discovery_deduplicates_the_harness_and_styling_and_smoke_stay_local(
    workflow: Workflow,
) -> None:
    result = workflow.dev("production-baseline-discover", "HEAD")
    assert result.returncode == 0, result
    assert [argv for argv in workflow.commands() if argv[1] == "fetch"] == [
        [*REFETCH, HARNESS]
    ]
    assert workflow.commands()[-1] == [
        *HOST,
        "--expected-harness",
        HARNESS,
        "--source",
        HARNESS,
    ]
    result = workflow.dev("production-baseline-qualify-styling")
    assert result.returncode == 0, result
    assert workflow.commands() == [
        CLEAN,
        HARNESS_QUERY,
        [*HOST, "--expected-harness", HARNESS],
    ]
    result = workflow.dev("production-baseline-smoke")
    assert result.returncode == 0, result
    assert workflow.commands() == [["nix", "run", ".#qualification-host"]]


# Real Git: the fetch goes to a local upstream instead of GitHub, and the host
# prints its arguments.
GIT = """\
import os, sys
args = sys.argv[1:]
if args[:1] == ["fetch"]:
    args = [os.environ["UPSTREAM"] if arg == "origin" else arg for arg in args]
os.execv(os.environ["REAL_GIT"], ["git", *args])
"""


def test_real_git_refuses_an_unpushed_harness_and_resolves_product_aliases(
    checkout: Checkout, tmp_path: Path
) -> None:
    root = checkout.root
    upstream = tmp_path / "upstream.git"

    def git(*args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(root), *args], check=True, capture_output=True, text=True
        ).stdout.strip()

    git("config", "user.name", "Fixture")
    git("config", "user.email", "fixture@example.invalid")
    # `dev` writes its trace under .xtask, which Jaunder's checkout ignores.
    (root / ".git" / "info" / "exclude").write_text(".xtask/\n")
    git("commit", "--quiet", "--allow-empty", "-m", "source")
    source = git("rev-parse", "HEAD")
    git("tag", "source-alias")
    git("commit", "--quiet", "--allow-empty", "-m", "target")
    target = git("rev-parse", "HEAD")
    git("tag", "target-alias")
    git("init", "--quiet", "--bare", str(upstream))
    git("push", "--quiet", str(upstream), "HEAD:refs/heads/main")
    git("remote", "add", "origin", "https://github.com/jaunder-org/jaunder.git")
    git("commit", "--quiet", "--allow-empty", "-m", "harness")
    harness = git("rev-parse", "HEAD")
    checkout.executable("git", f"#!{sys.executable}\n{GIT}")
    checkout.executable("nix", "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
    real_git = shutil.which("git") or "git"

    def run(*args: str) -> subprocess.CompletedProcess[bytes]:
        return checkout.dev(*args, REAL_GIT=real_git, UPSTREAM=str(upstream))

    accept = ("production-baseline-accept", "source-alias", "target-alias")
    for protocol in ["0", "2"]:
        git("config", "protocol.version", protocol)
        refused = run(*accept)
        assert refused.returncode != 0, refused
        assert b"qualification-host" not in refused.stdout
    git("push", "--quiet", str(upstream), "HEAD:refs/heads/main")
    (root / ".git" / "FETCH_HEAD").write_text("preserve fetch head\n")
    admitted = run(*accept)
    assert admitted.returncode == 0, admitted
    assert admitted.stdout.decode() == (
        f"run\n.#qualification-host\n--\n--expected-harness\n{harness}\n"
        f"--source\n{source}\n--target\n{target}\n"
    )
    assert (root / ".git" / "FETCH_HEAD").read_text() == "preserve fetch head\n"
    assert run("production-baseline-accept", "source-alias", source).returncode != 0
    assert run("production-baseline-discover", "not-a-revision").returncode != 0
    git("commit", "--quiet", "--allow-empty", "-m", "local-only product")
    local_product = git("rev-parse", "HEAD")
    git("update-ref", "HEAD", harness)
    for protocol in ["0", "2"]:
        git("config", "protocol.version", protocol)
        for products in [(local_product, target), (source, local_product)]:
            refused = run("production-baseline-accept", *products)
            assert refused.returncode != 0, refused
            assert b"qualification-host" not in refused.stdout
    git("remote", "remove", "origin")
    assert run("production-baseline-qualify-styling").returncode == 0
