"""CI's documentation-only and dependency scopes, on real repositories.

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
        "docs_only=true\ndependencies=false\n",
        "ci-scope: docs_only=True reason=isolated-staged-markdown dependencies=False\n",
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
        "docs_only=false\ndependencies=false\n",
        f"ci-scope: docs_only=False reason={reason} dependencies=False\n",
    )


def test_a_deleted_markdown_symlink_runs_everything(
    git: Git, capsys: pytest.CaptureFixture[str]
) -> None:
    symlink()
    commit(git)
    base = changed(git, lambda: Path("link.md").unlink())
    assert answer(base, capsys)[0] == "docs_only=false\ndependencies=false\n"


@pytest.mark.parametrize(
    ("base", "reason", "dependencies"),
    [("", "no-base", False), ("0" * 40, "base-unavailable", True)],
    ids=["push", "unfetchable"],
)
def test_without_a_usable_base_everything_runs(
    git: Git,
    base: str,
    reason: str,
    dependencies: bool,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A push leaves advisories to the daily scan of `main`; a pull request or
    merge-queue entry whose base cannot be read is checked against them."""
    changed(git, lambda: Path("guide.md").write_text("# Changed\n"))
    assert answer(base, capsys) == (
        f"docs_only=false\ndependencies={str(dependencies).lower()}\n",
        f"ci-scope: docs_only=False reason={reason} dependencies={dependencies}\n",
    )


def test_an_uncertain_diff_runs_everything(
    git: Git, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    base = changed(git, lambda: Path("guide.md").write_text("# Changed\n"))
    malformed = subprocess.CompletedProcess([], 0, b":not a raw record\0guide.md\0")
    monkeypatch.setattr(scope, "_git", lambda *_: malformed)
    assert answer(base, capsys) == (
        "docs_only=false\ndependencies=true\n",
        "ci-scope: docs_only=False reason=uncertain-status dependencies=True\n",
    )


def write(path: str, text: str = "") -> Callable[[], None]:
    def change() -> None:
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        Path(path).write_text(text)

    return change


@pytest.mark.parametrize(
    ("change", "dependencies"),
    [
        (write("Cargo.toml", "[workspace]\n"), True),
        (write("server/Cargo.toml", "[package]\n"), True),
        (write("Cargo.lock", "version = 4\n"), True),
        (write("deny.toml", "[bans]\n"), True),
        (lambda: Path("main.rs").write_text("fn main() { }\n"), False),
        (write("tools/Cargo.lock", "version = 4\n"), True),
        (write("tools/dev-tool/Cargo.toml", "[package]\n"), True),
        (write("server/Cargo.lock.md", "# Not a lock\n"), False),
    ],
    ids=[
        "root-manifest",
        "member-manifest",
        "root-lock",
        "deny-policy",
        "source",
        "tools-lock",
        "tools-manifest",
        "lookalike",
    ],
)
def test_advisories_run_when_any_workspace_dependencies_change(
    git: Git,
    change: Callable[[], None],
    dependencies: bool,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """cargo-deny checks both workspaces under the root policy, so any
    manifest or lock, and that policy, decide whether advisories run."""
    base = changed(git, change)
    assert answer(base, capsys)[0].splitlines()[1] == (
        f"dependencies={str(dependencies).lower()}"
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
    assert (result.returncode, result.stdout) == (
        0,
        "docs_only=true\ndependencies=false\n",
    ), result
