"""CI's documentation-only scope, on real repositories.

Each test commits a base, changes it, and asks `dev.scope` about HEAD. The
repository is its own `origin`, so the base fetch is real too.
"""

import os
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

from dev import scope

Git = Callable[..., str]


@pytest.fixture
def git(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Git:
    monkeypatch.chdir(tmp_path)

    def run(*args: str) -> str:
        return subprocess.run(
            ["git", *args], capture_output=True, check=True, text=True
        ).stdout.strip()

    run("init", "-q")
    run("config", "user.name", "fixture")
    run("config", "user.email", "fixture@example.invalid")
    run("remote", "add", "origin", str(tmp_path))
    Path("guide.md").write_text("# Guide\n")
    Path("main.rs").write_text("fn main() {}\n")
    commit(run)
    return run


def commit(git: Git) -> str:
    git("add", "-A")
    git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture", "--allow-empty")
    return git("rev-parse", "HEAD")


def answer(base: str, capsys: pytest.CaptureFixture[str]) -> tuple[str, str]:
    scope.ci_scope(base)
    captured = capsys.readouterr()
    return captured.out, captured.err


def changed(git: Git, change: Callable[[], object]) -> str:
    """The base commit, with `change` committed on top of it."""
    base = git("rev-parse", "HEAD")
    change()
    commit(git)
    return base


def symlink() -> None:
    os.symlink("guide.md", "link.md")


def mixed() -> None:
    Path("guide.md").write_text("# Changed\n")
    Path("main.rs").unlink()


def typechange() -> None:
    Path("guide.md").unlink()
    os.symlink("main.rs", "guide.md")


def add_nested() -> None:
    Path("docs").mkdir()
    Path("docs/new.md").write_text("")


@pytest.mark.parametrize(
    "change",
    [
        lambda: Path("guide.md").write_text("# Changed\n"),
        add_nested,
        lambda: Path("guide.md").unlink(),
        lambda: Path("guide.md").rename("renamed.md"),
    ],
    ids=["modify", "add", "delete", "rename"],
)
def test_a_markdown_only_change_is_docs_only(
    git: Git, change: Callable[[], object], capsys: pytest.CaptureFixture[str]
) -> None:
    base = changed(git, change)
    assert answer(base, capsys) == (
        "docs_only=true\n",
        "ci-scope: docs_only=True reason=isolated-staged-markdown\n",
    )


@pytest.mark.parametrize(
    ("change", "reason"),
    [
        (lambda: Path("main.rs").write_text("fn main() { }\n"), "non-markdown-path"),
        (mixed, "non-markdown-path"),
        (lambda: Path("guide.MD").write_text(""), "non-markdown-path"),
        (symlink, "unsupported-index-mode"),
        (typechange, "unsupported-change"),
        (lambda: None, "empty-state"),
    ],
    ids=["code", "mixed", "case", "symlink", "typechange", "empty"],
)
def test_any_other_change_runs_everything(
    git: Git,
    change: Callable[[], object],
    reason: str,
    capsys: pytest.CaptureFixture[str],
) -> None:
    base = changed(git, change)
    assert answer(base, capsys) == (
        "docs_only=false\n",
        f"ci-scope: docs_only=False reason={reason}\n",
    )


def test_a_deleted_markdown_symlink_runs_everything(
    git: Git, capsys: pytest.CaptureFixture[str]
) -> None:
    symlink()
    commit(git)
    base = changed(git, lambda: Path("link.md").unlink())
    assert answer(base, capsys)[0] == "docs_only=false\n"


@pytest.mark.parametrize(
    ("base", "reason"),
    [("", "no-base"), ("0" * 40, "base-unavailable")],
    ids=["push", "unfetchable"],
)
def test_without_a_usable_base_everything_runs(
    git: Git, base: str, reason: str, capsys: pytest.CaptureFixture[str]
) -> None:
    changed(git, lambda: Path("guide.md").write_text("# Changed\n"))
    assert answer(base, capsys) == (
        "docs_only=false\n",
        f"ci-scope: docs_only=False reason={reason}\n",
    )


def test_an_uncertain_diff_runs_everything(
    git: Git, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    base = changed(git, lambda: Path("guide.md").write_text("# Changed\n"))
    malformed = subprocess.CompletedProcess([], 0, b":not a raw record\0guide.md\0")
    monkeypatch.setattr(scope, "_git", lambda *_: malformed)
    assert answer(base, capsys) == (
        "docs_only=false\n",
        "ci-scope: docs_only=False reason=uncertain-status\n",
    )


@pytest.mark.parametrize(
    "isolated", [False, True], ids=["inherited-environment", "standard-library"]
)
def test_ci_runs_it_as_a_module(git: Git, isolated: bool) -> None:
    """As the slim verdict job does, with its own Python. Isolated, there are
    no site-packages, so only the standard library can be imported."""
    base = changed(git, lambda: Path("guide.md").write_text("# Changed\n"))
    tools_dev = str(Path(__file__).resolve().parents[1])
    flags, environment = (
        (["-S"], {"PATH": os.environ["PATH"]}) if isolated else ([], dict(os.environ))
    )
    result = subprocess.run(
        [sys.executable, *flags, "-m", "dev.scope", "--base", base],
        capture_output=True,
        text=True,
        check=False,
        env=environment | {"PYTHONPATH": tools_dev},
    )
    assert (result.returncode, result.stdout) == (0, "docs_only=true\n"), result
