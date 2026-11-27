"""Index custody of `dev precommit` and `dev prepush`, on real repositories.

Every host tool but Git is a fake; Git is real, so the snapshots, refusals and
staging act on a real index. `dev` runs as a hook would start it, with GIT_DIR
and GIT_INDEX_FILE pointing at the fixture.
"""

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest
from conftest import FAKE, REAL_GIT, Workflow
from test_gates import CHECK_NO_TEST, PREPUSH_SHARES, STATIC_DOCS, docs_shape

from dev import index
from dev.index import PathStatus, Refused, Snapshot
from dev.rawdiff import Reason

shared_docs = pytest.mark.skipif(
    not PREPUSH_SHARES, reason="the shared docs verdict is x86_64-linux only"
)
# What the fakes and `dev` write into the checkout root.
EXCLUDE = (
    "/order\n/commands\n/cache-env\n/cargo-env\n/.xtask/\n"
    "/producer-resource\n/producer-cleanup\n"
)
MARKDOWN_HOST = [
    ["prettier", "-w", "**/*.md"],
    ["prettier", "-w", "end2end"],
    *docs_shape(),
]


@dataclass
class Repo:
    workflow: Workflow

    @property
    def root(self) -> Path:
        return self.workflow.root

    def git(self, *args: str) -> bytes:
        return subprocess.run(
            ["git", *args], cwd=self.root, capture_output=True, check=True
        ).stdout

    def write(self, path: str, text: str) -> None:
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def commit(self, message: str = "fixture") -> None:
        self.git("add", ".")
        self.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", message)

    def status(self) -> bytes:
        return self.git("status", "--porcelain")

    def hook(self) -> dict[str, str]:
        """What a pre-commit hook exports for this repository."""
        git = self.root / ".git"
        return {"GIT_DIR": str(git), "GIT_INDEX_FILE": str(git / "index")}

    def environment(self, failure: str = "") -> dict[str, str]:
        return {
            "FAKE_LOG": str(self.root / "commands"),
            "FAIL_COMMAND": failure,
            "REAL_GIT": REAL_GIT,
        }

    def dev(
        self, *args: str, failure: str = "", **hook: str
    ) -> subprocess.CompletedProcess[bytes]:
        (self.root / "commands").write_bytes(b"")
        checkout = self.workflow.checkout
        environment = self.environment(failure) | self.hook() | hook
        return checkout.run(
            checkout.command(*args), checkout.environment(**environment)
        )

    def tool(self, name: str, body: str) -> None:
        self.workflow.checkout.tool(name, body)

    def recording(self, name: str) -> Path:
        """The recording fake as `name`, off PATH, for a scripted tool to defer to."""
        fake = self.workflow.checkout.bin.parent / "fake" / name
        fake.parent.mkdir(exist_ok=True)
        fake.write_text(FAKE, encoding="utf-8")
        fake.chmod(0o755)
        return fake

    def fmt(self, body: str) -> None:
        """Scripts `cargo fmt`, the broad host's first step; every other Cargo
        command reaches the recording fake."""
        fake = self.recording("cargo")
        self.tool(
            "cargo",
            "if sys.argv[1:] != ['fmt']:\n"
            f"    os.execv({str(fake)!r}, [{str(fake)!r}, *sys.argv[1:]])\n" + body,
        )


@pytest.fixture
def repo(workflow: Workflow) -> Repo:
    (workflow.checkout.bin / "git").unlink()
    fixture = Repo(workflow)
    fixture.git("config", "user.name", "fixture")
    fixture.git("config", "user.email", "fixture@example.invalid")
    (workflow.root / ".git" / "info" / "exclude").write_text(EXCLUDE)
    fixture.write("guide.txt", "original\n")
    fixture.write("guide.md", "# Guide\n")
    fixture.commit()
    return fixture


def stderr(result: subprocess.CompletedProcess[bytes]) -> str:
    return result.stderr.decode()


# Precommit, through `dev`.


def test_a_broad_change_runs_check_no_test_and_stages_nothing_new(repo: Repo) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    result = repo.dev("precommit")
    assert result.returncode == 0, result
    assert result.stdout.startswith(
        b"precommit-routing: class=broad reason=non-markdown-path\n"
    )
    assert result.stdout.endswith(b"[ ok ] precommit\n")
    assert stderr(result) == "precommit-staging: no staged fixes\n"
    assert repo.workflow.commands() == CHECK_NO_TEST
    assert repo.status() == b"M  guide.txt\n"
    slices = repo.workflow.checkout.merged()
    gate = slices["dev precommit"]["args"]["span_id"]
    for phase in [
        "snapshot-before",
        "precommit-broad-host",
        "snapshot-after",
        "staging",
    ]:
        assert slices[phase]["args"]["parent_id"] == gate, phase
        assert slices[phase]["args"]["jaunder.status"] == "ok", phase
    assert (
        slices["fmt"]["args"]["parent_id"]
        == (slices["precommit-broad-host"]["args"]["span_id"])
    )


def test_a_failed_host_check_stops_the_checks_but_staging_still_runs(
    repo: Repo,
) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    failing = CHECK_NO_TEST[4]
    result = repo.dev("precommit", failure=" ".join(failing))
    assert result.returncode == 1, result
    assert repo.workflow.commands() == CHECK_NO_TEST[:5]
    assert stderr(result).endswith(
        "precommit-staging: no staged fixes\n"
        "[FAIL] precommit — precommit-host: precommit-broad-host exited 17\n"
    )
    slices = repo.workflow.checkout.merged()
    assert slices["precommit-broad-host"]["args"]["jaunder.status"] == "failed"
    assert slices["staging"]["args"]["jaunder.status"] == "ok"
    assert slices["dev precommit"]["args"]["process.exit.code"] == 1


def test_precommit_reconciles_staged_fixes_even_after_a_host_failure(
    repo: Repo,
) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    repo.fmt(
        "pathlib.Path('guide.txt').write_text('formatted\\n')\nsys.exit(23)",
    )
    result = repo.dev("precommit")
    assert result.returncode == 1
    assert stderr(result).count("precommit-staging:") == 1
    assert "precommit-staging: staged: guide.txt\n" in stderr(result)
    assert "precommit-host: precommit-broad-host exited 23" in stderr(result)
    assert repo.git("show", ":guide.txt") == b"formatted\n"
    assert repo.status() == b"M  guide.txt\n"
    assert "static-docs" not in repo.workflow.checkout.merged()


def test_precommit_preserves_the_host_failure_when_the_after_snapshot_fails(
    repo: Repo,
) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    index_before = repo.git("diff", "--cached", "--raw")
    repo.fmt(
        "pathlib.Path('guide.txt').write_text('formatted\\n')\n"
        "os.rename('.git', '.xtask/parked-git')\n"
        "sys.exit(23)",
    )
    (repo.root / ".xtask").mkdir()
    result = repo.dev("precommit")
    (repo.root / ".xtask" / "parked-git").rename(repo.root / ".git")
    assert result.returncode == 1
    text = stderr(result)
    assert "precommit-host: precommit-broad-host exited 23" in text
    assert (
        "precommit-staging: after-snapshot failed; refusing staging: "
        "git status --porcelain=v1 -z failed: " in text
    )
    assert text.count("precommit-staging:") == 1
    assert repo.git("diff", "--cached", "--raw") == index_before
    assert repo.git("show", ":guide.txt") == b"staged\n"
    assert (repo.root / "guide.txt").read_text() == "formatted\n"
    assert repo.status() == b"MM guide.txt\n"


def test_precommit_stages_only_safe_fixes_beside_unowned_changes(repo: Repo) -> None:
    repo.write("unowned.txt", "author\n")
    repo.commit()
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    repo.fmt(
        "for name, text in [('guide.txt', 'formatted'), ('unowned.txt', 'unexpected'),"
        " ('new.txt', 'new')]:\n"
        "    pathlib.Path(name).write_text(text + '\\n')\n"
        "sys.exit()",
    )
    result = repo.dev("precommit")
    assert result.returncode == 1
    text = stderr(result)
    assert "unowned.txt: will not add work the user did not stage" in text
    assert "new.txt: new untracked file created during precommit" in text
    assert text.count("precommit-staging:") == 1
    assert repo.git("show", ":guide.txt") == b"formatted\n"
    assert repo.git("show", ":unowned.txt") == b"author\n"
    assert repo.status() == b"M  guide.txt\n M unowned.txt\n?? new.txt\n"


def test_precommit_never_stages_mixed_work_or_preexisting_untracked_files(
    repo: Repo,
) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    repo.write("guide.txt", "unstaged author work\n")
    repo.write("notes.txt", "untracked author work\n")
    repo.fmt("pathlib.Path('guide.txt').write_text('formatted\\n')")
    result = repo.dev("precommit")
    assert result.returncode == 1
    assert result.stdout.startswith(
        b"precommit-routing: class=broad reason=untracked-path\n"
    )
    assert (
        "guide.txt: pre-existing mixed staged/unstaged state made auto-staging unsafe"
        in stderr(result)
    )
    assert repo.git("show", ":guide.txt") == b"staged\n"
    assert repo.status() == b"MM guide.txt\n?? notes.txt\n"


@pytest.mark.parametrize(
    ("change", "refusal"),
    [
        ("delete", "guide.txt: delete/rename status is unsafe for auto-staging"),
        (
            "symlink",
            "guide.txt: unsupported change or index mode is unsafe for auto-staging",
        ),
    ],
)
def test_precommit_refuses_to_stage_beside_unsafe_index_state(
    repo: Repo, change: str, refusal: str
) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    if change == "delete":
        repo.git("rm", "-q", "guide.md")
    else:
        (repo.root / "link").symlink_to("guide.md")
        repo.git("add", "link")
    repo.fmt("pathlib.Path('guide.txt').write_text('formatted\\n')")
    result = repo.dev("precommit")
    assert result.returncode == 1
    assert refusal in stderr(result)
    assert repo.git("show", ":guide.txt") == b"staged\n"


def test_an_unreadable_index_refuses_before_any_check(repo: Repo) -> None:
    garbage = repo.root / ".git" / "garbage-index"
    garbage.write_text("not an index")
    result = repo.dev("precommit", GIT_INDEX_FILE=str(garbage))
    assert result.returncode == 1
    assert stderr(result).startswith(
        "[FAIL] precommit — precommit before-snapshot: "
        "git status --porcelain=v1 -z failed: "
    )
    assert repo.workflow.commands() == []


def test_a_signalled_host_check_stops_the_gate_without_staging(repo: Repo) -> None:
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    repo.fmt(
        "pathlib.Path('guide.txt').write_text('formatted\\n')\n"
        "os.kill(os.getpid(), signal.SIGKILL)",
    )
    result = repo.dev("precommit")
    assert result.returncode == 128 + 9
    assert "precommit-staging" not in stderr(result)
    assert repo.git("show", ":guide.txt") == b"staged\n"


def test_a_partial_commit_reconciles_the_hooks_temporary_index(repo: Repo) -> None:
    """`git commit <path>` gives the hook a temporary GIT_INDEX_FILE."""
    repo.workflow.checkout.dev_on_path()
    hook = repo.root / ".git" / "hooks" / "pre-commit"
    hook.write_text("#!/bin/sh\nexec dev precommit\n")
    hook.chmod(0o755)
    repo.fmt("pathlib.Path('guide.txt').write_text('formatted\\n')")
    repo.write("other.txt", "staged elsewhere\n")
    repo.git("add", "other.txt")
    repo.write("guide.txt", "partial\n")
    committed = subprocess.run(
        ["git", "commit", "-qm", "partial", "--", "guide.txt"],
        cwd=repo.root,
        env=repo.workflow.checkout.environment(**repo.environment()),
        capture_output=True,
        check=False,
        timeout=60,
    )
    assert committed.returncode == 0, committed
    assert b"precommit-staging: staged: guide.txt\n" in committed.stderr
    assert repo.git("show", "HEAD:guide.txt") == b"formatted\n"
    assert repo.git("diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD") == (
        b"guide.txt\n"
    )
    assert repo.git("show", ":other.txt") == b"staged elsewhere\n"


def test_host_checks_never_inherit_the_hooks_repository(
    repo: Repo, tmp_path: Path
) -> None:
    """A check that initialises a scratch repository (as cargo test and pytest
    do) initialises it, not the hooked repository (which it could make bare)."""
    scratch = tmp_path / "scratch"
    repo.fmt(f"subprocess.run(['git', 'init', '-q', {str(scratch)!r}], check=True)")
    repo.write("guide.txt", "staged\n")
    repo.git("add", "guide.txt")
    result = repo.dev("precommit")
    assert result.returncode == 0, result
    assert repo.git("config", "--get", "core.bare") == b"false\n"
    assert (scratch / ".git").is_dir()
    assert repo.status() == b"M  guide.txt\n"


@shared_docs
def test_a_markdown_change_checks_markdown_then_requests_the_docs_verdict(
    repo: Repo,
) -> None:
    repo.write("guide.md", "# Staged\n")
    repo.git("add", "guide.md")
    result = repo.dev("precommit")
    assert result.returncode == 0, result
    assert result.stdout == (
        b"precommit-routing: class=staged-markdown-only "
        b"reason=isolated-staged-markdown\n[ ok ] precommit\n"
    )
    assert stderr(result) == (
        "precommit-staging: no staged fixes\nstatic-docs-state: unchanged\n"
    )
    assert repo.workflow.commands() == [*MARKDOWN_HOST, *STATIC_DOCS]
    slices = repo.workflow.checkout.merged()
    gate = slices["dev precommit"]["args"]["span_id"]
    for phase in [
        "snapshot-before",
        "precommit-markdown-host",
        "snapshot-after",
        "staging",
        "static-docs-admission",
        "static-docs",
        "static-docs-post-check",
    ]:
        assert slices[phase]["args"]["parent_id"] == gate, phase


@shared_docs
def test_a_markdown_rename_checks_markdown_then_requests_the_docs_verdict(
    repo: Repo,
) -> None:
    repo.git("mv", "guide.md", "renamed.md")
    result = repo.dev("precommit")
    assert result.returncode == 0, result
    assert result.stdout.startswith(
        b"precommit-routing: class=staged-markdown-only "
        b"reason=isolated-staged-markdown\n"
    )
    assert "static-docs-state: unchanged\n" in stderr(result)
    assert repo.workflow.commands() == [*MARKDOWN_HOST, *STATIC_DOCS]
    assert repo.status() == b"R  guide.md -> renamed.md\n"


@shared_docs
def test_a_formatted_markdown_change_is_restaged_before_the_request(
    repo: Repo,
) -> None:
    repo.write("guide.md", "# Staged\n")
    repo.git("add", "guide.md")
    repo.tool("prettier", "pathlib.Path('guide.md').write_text('# Formatted\\n')")
    result = repo.dev("precommit")
    assert result.returncode == 0, result
    assert "precommit-staging: staged: guide.md\n" in stderr(result)
    assert "static-docs-state: unchanged\n" in stderr(result)
    assert repo.git("show", ":guide.md") == b"# Formatted\n"
    assert repo.status() == b"M  guide.md\n"


@shared_docs
def test_a_markdown_host_failure_skips_the_docs_request(repo: Repo) -> None:
    repo.write("guide.md", "# Staged\n")
    repo.git("add", "guide.md")
    result = repo.dev("precommit", failure=" ".join(MARKDOWN_HOST[0]))
    assert result.returncode == 1
    assert repo.workflow.commands() == MARKDOWN_HOST[:1]
    assert "static-docs" not in repo.workflow.checkout.merged()


@shared_docs
def test_a_failed_markdown_request_keeps_its_status_after_the_post_check(
    repo: Repo,
) -> None:
    repo.write("guide.md", "# Staged\n")
    repo.git("add", "guide.md")
    result = repo.dev("precommit", failure=" ".join(STATIC_DOCS[-1]))
    assert result.returncode == 17
    assert stderr(result).endswith("nix failed\nstatic-docs-state: unchanged\n")
    assert "[ ok ] precommit" not in result.stdout.decode()
    assert repo.status() == b"M  guide.md\n"


@shared_docs
def test_a_converting_markdown_checkout_is_not_admitted_to_the_request(
    repo: Repo,
) -> None:
    repo.write(".gitattributes", "*.md text eol=crlf\n")
    repo.commit()
    repo.write("guide.md", "# Guide\r\nChanged\r\n")
    repo.git("add", "guide.md")
    result = repo.dev("precommit")
    assert result.returncode == 1
    assert stderr(result).endswith(
        "[FAIL] precommit — static-docs-admission: "
        "raw worktree bytes differ from staged index blob: guide.md\n"
    )
    assert repo.workflow.commands() == MARKDOWN_HOST


# Prepush's custody around its checks and their shared verdict requests.


def prepush_fixture(repo: Repo, request: str, check: str = "static-docs") -> None:
    """A clean tree with docs inputs, and a `check` request that runs `request`;
    every other Nix call reaches the recording fake."""
    repo.write("docs/adr/0001-choice.md", "# ADR-0001: Choice\n")
    repo.write(".prettierrc.json", "{}\n")
    repo.write("flake.lock", "{}\n")
    repo.commit()
    fake = str(repo.recording("nix"))
    repo.tool(
        "nix",
        f"if sys.argv[1] != 'build' or sys.argv[-1] != '.#checks.x86_64-linux.{check}':\n"
        f"    os.execv({fake!r}, [{fake!r}, *sys.argv[1:]])\n"
        "with open('request-args', 'w') as recorded:\n"
        "    recorded.write('\\n'.join(sys.argv[1:]) + '\\n')\n" + request,
    )
    (repo.root / ".git" / "info" / "exclude").write_text(EXCLUDE + "/request-args\n")


@shared_docs
def test_prepush_runs_the_post_check_after_a_failed_request(repo: Repo) -> None:
    prepush_fixture(
        repo, "print('error: fixture docs failure', file=sys.stderr)\nsys.exit(1)"
    )
    result = repo.dev("prepush")
    # The request's own status, as for any failed step.
    assert result.returncode == 1
    assert stderr(result).endswith(
        "error: fixture docs failure\nprepush-state: unchanged\n"
    )
    assert repo.status() == b""
    slices = repo.workflow.checkout.merged()
    assert slices["static-docs"]["args"]["jaunder.status"] == "failed"
    assert slices["prepush-post-check"]["args"]["jaunder.status"] == "ok"
    # Fail-fast: nothing after the request ran.
    assert "static-elisp" not in slices


@shared_docs
def test_prepush_requests_every_shared_verdict_inside_custody(repo: Repo) -> None:
    prepush_fixture(repo, "")
    result = repo.dev("prepush")
    assert result.returncode == 0, result
    assert stderr(result) == "prepush-state: unchanged\n"
    request = STATIC_DOCS[-1][1:]
    assert (repo.root / "request-args").read_text().splitlines() == request
    slices = repo.workflow.checkout.merged()
    gate = slices["dev prepush"]["args"]["span_id"]
    custody = ["prepush-admission", "prepush-post-check"]
    shared = [
        "rust-format",
        "static-docs",
        "static-elisp",
        "static-typescript",
        "static-ert",
    ]
    for name in ["clean-tree", *custody, *shared, "nextest"]:
        assert slices[name]["args"]["parent_id"] == gate, name
    # Each request falls between the admission and the post-check.
    starts = {name: slices[name]["ts"] for name in [*custody, *shared]}
    assert all(
        starts[custody[0]] < starts[name] < starts[custody[1]] for name in shared
    )


@shared_docs
def test_prepush_keeps_the_request_failure_when_the_post_check_query_fails(
    repo: Repo,
) -> None:
    prepush_fixture(
        repo,
        "pathlib.Path('guide.txt').write_text('request mutation\\n')\n"
        "os.rename('.git', '.xtask/parked-git')\n"
        "print('error: original request failure', file=sys.stderr)\nsys.exit(19)",
    )
    (repo.root / ".xtask").mkdir()
    index_before = repo.git("diff", "--cached", "--raw")
    result = repo.dev("prepush")
    (repo.root / ".xtask" / "parked-git").rename(repo.root / ".git")
    assert result.returncode == 1
    text = stderr(result)
    assert "error: original request failure\n" in text
    assert (
        "[FAIL] prepush — the checks exited 19\n"
        "prepush-state: git status --porcelain=v1 -z failed: " in text
    )
    assert "prepush-state: unchanged" not in text
    assert repo.git("diff", "--cached", "--raw") == index_before
    assert repo.status() == b" M guide.txt\n"


@shared_docs
@pytest.mark.parametrize(
    ("mutation", "expected"),
    [
        ("pathlib.Path('late.md').write_text('changed')", b"?? late.md\n"),
        (
            "pathlib.Path('docs/adr/0001-choice.md').write_text('changed')",
            b" M docs/adr/0001-choice.md\n",
        ),
        (
            "pathlib.Path('.prettierrc.json').write_text('changed')",
            b" M .prettierrc.json\n",
        ),
        ("pathlib.Path('flake.lock').write_text('changed')", b" M flake.lock\n"),
        (
            (
                "pathlib.Path('late.md').write_text('changed')\n"
                "subprocess.run(['git', 'add', '--', 'late.md'], check=True)"
            ),
            b"A  late.md\n",
        ),
    ],
)
def test_a_failed_request_cannot_hide_a_mutation_or_restage_it(
    repo: Repo, mutation: str, expected: bytes
) -> None:
    prepush_fixture(
        repo,
        f"{mutation}\nprint('error: fixture request failed', file=sys.stderr)\n"
        "sys.exit(19)",
        check="static-ert",
    )
    result = repo.dev("prepush")
    assert result.returncode == 1
    assert stderr(result).endswith(
        "[FAIL] prepush — the checks exited 19\n"
        "prepush-state: tracked, staged, or nonignored untracked Git "
        "state changed during prepush\n"
    )
    assert repo.status() == expected


@shared_docs
def test_a_mutating_request_that_succeeds_still_fails(repo: Repo) -> None:
    prepush_fixture(
        repo, "pathlib.Path('flake.lock').write_text('changed')", "rust-format"
    )
    result = repo.dev("prepush")
    assert result.returncode == 1
    assert stderr(result).endswith(
        "[FAIL] prepush — prepush-state: tracked, staged, or "
        "nonignored untracked Git state changed during prepush\n"
    )
    # The checks after the request still ran before the post-check refused.
    assert "nextest" in repo.workflow.checkout.merged()


@shared_docs
def test_a_converting_prepush_checkout_is_refused_before_any_request(
    repo: Repo,
) -> None:
    repo.write(".gitattributes", "*.txt text eol=crlf\n")
    repo.write("notes.txt", "converted\r\n")
    prepush_fixture(repo, "")
    assert repo.status() == b""
    result = repo.dev("prepush")
    assert result.returncode == 1
    assert stderr(result) == (
        "[FAIL] prepush — prepush-admission: "
        "raw worktree bytes differ from staged index blob: notes.txt\n"
    )
    assert repo.workflow.commands() == []


@shared_docs
def test_a_signalled_request_stops_without_the_post_check(repo: Repo) -> None:
    prepush_fixture(repo, "os.kill(os.getpid(), signal.SIGKILL)")
    result = repo.dev("prepush")
    assert result.returncode == 128 + 9
    assert "prepush-state" not in stderr(result)


# The snapshot parser and classification.

ZERO = "0" * 40
OLD = "1" * 40
NEW = "2" * 40


def raw(mode: str, status: str, path: str) -> bytes:
    old_mode, new_mode, old, new = {
        "A": ("000000", mode, ZERO, NEW),
        "D": (mode, "000000", OLD, ZERO),
    }.get(status, (mode, mode, OLD, NEW))
    return f":{old_mode} {new_mode} {old} {new} {status}\0{path}\0".encode()


def raw_modes(old_mode: str, new_mode: str, status: str, path: str) -> bytes:
    return f":{old_mode} {new_mode} {OLD} {NEW} {status}\0{path}\0".encode()


def test_status_records_set_index_worktree_and_untracked_state() -> None:
    taken = index.parse(
        b"M  a.rs\0 M b.rs\0MM c.rs\0?? scratch.rs\0",
        raw("100644", "M", "a.rs") + raw("100644", "M", "c.rs"),
    )
    assert taken.uncertainty == []
    assert (taken.paths["a.rs"].staged, taken.paths["a.rs"].unstaged) == (True, False)
    assert (taken.paths["b.rs"].staged, taken.paths["b.rs"].unstaged) == (False, True)
    assert (taken.paths["c.rs"].staged, taken.paths["c.rs"].unstaged) == (True, True)
    assert taken.paths["scratch.rs"].untracked
    assert list(taken.paths) == ["a.rs", "b.rs", "c.rs", "scratch.rs"]


def test_deletes_and_renames_are_marked_unsafe() -> None:
    taken = index.parse(
        b"D  gone.rs\0 D missing.rs\0R  new.rs\0old.rs\0",
        raw("100644", "D", "gone.rs"),
    )
    assert taken.paths["gone.rs"].delete_or_rename
    assert taken.paths["missing.rs"].delete_or_rename
    assert taken.paths["new.rs"].delete_or_rename
    assert taken.paths["old.rs"].delete_or_rename


def test_regular_markdown_additions_modifications_and_deletes_route_narrowly() -> None:
    for status in ["A", "M", "D"]:
        taken = index.parse(
            f"{status}  guide.md\0".encode(), raw("100644", status, "guide.md")
        )
        assert index.classify(taken) is Reason.ISOLATED_STAGED_MARKDOWN
    assert index.routing(Reason.ISOLATED_STAGED_MARKDOWN) == (
        "class=staged-markdown-only reason=isolated-staged-markdown"
    )


@pytest.mark.parametrize(
    ("status", "raw_diff", "reason"),
    [
        (b"bad\0", b"", Reason.UNCERTAIN_STATUS),
        (b"", b"", Reason.EMPTY_STATE),
        (b"?? scratch.md\0", b"", Reason.UNTRACKED_PATH),
        (b" M guide.md\0", b"", Reason.UNSTAGED_PATH),
        (b"D  guide.txt\0", raw("100644", "D", "guide.txt"), Reason.NON_MARKDOWN_PATH),
        (
            b"D  guide.md\0",
            raw("120000", "D", "guide.md"),
            Reason.UNSUPPORTED_INDEX_MODE,
        ),
        (
            b"T  guide.md\0",
            raw_modes("100644", "120000", "T", "guide.md"),
            Reason.UNSUPPORTED_CHANGE,
        ),
        (
            b"M  guide.md\0",
            raw("120000", "M", "guide.md"),
            Reason.UNSUPPORTED_INDEX_MODE,
        ),
        (b"M  guide.MD\0", raw("100644", "M", "guide.MD"), Reason.NON_MARKDOWN_PATH),
        (b"M  nested\0", raw("160000", "M", "nested"), Reason.UNSUPPORTED_INDEX_MODE),
    ],
)
def test_broad_reasons_keep_their_precedence(
    status: bytes, raw_diff: bytes, reason: Reason
) -> None:
    assert index.classify(index.parse(status, raw_diff)) is reason
    assert index.routing(reason) == f"class=broad reason={reason}"


@pytest.mark.parametrize(
    ("status", "raw_diff", "uncertainty"),
    [
        (
            b"A  \xff.md\0",
            raw("100644", "A", "safe.md"),
            "non-utf8-or-empty-status-path",
        ),
        (
            b"A  guide.md\0",
            raw("100644", "A", "other.md"),
            "status-raw-population-mismatch",
        ),
        (b"Z  guide.md\0", raw("100644", "M", "guide.md"), "unknown-status-pair"),
        (b"MX guide.md\0", b"", "unknown-status-pair"),
        (b"M  guide.md", raw("100644", "M", "guide.md"), "unterminated-status-record"),
        (
            b"M  first.md\0\0",
            raw("100644", "M", "first.md"),
            "interior-empty-status-record",
        ),
        (
            b"M  guide.md\0",
            raw("100644", "M", "guide.md")[:-1],
            "unterminated-raw-record",
        ),
        (
            b"M  guide.md\0M  guide.md\0",
            raw("100644", "M", "guide.md"),
            "duplicate-status-path",
        ),
        (
            b"M  guide.md\0",
            raw("100644", "M", "guide.md") * 2,
            "duplicate-raw-path",
        ),
        (
            b"M  guide.md\0",
            raw("100644", "A", "guide.md"),
            "status-raw-change-mismatch",
        ),
        (b"R  new.md\0", b"", "missing-rename-path"),
        (b"R  new.md\0\xff\0", b"", "non-utf8-or-empty-rename-path"),
        (
            b"M  guide.md\0",
            b":100644 100644 short invalid M\0guide.md\0",
            "malformed-raw-record",
        ),
        (b"M  guide.md\0", b"\xff\0guide.md\0", "malformed-raw-record"),
        (b"M  guide.md\0", b"100644 100644 1 2 M\0guide.md\0", "malformed-raw-record"),
        (
            b"M  guide.md\0",
            f":100644 10064x {OLD} {NEW} M\0guide.md\0".encode(),
            "malformed-raw-record",
        ),
        (
            b"M  guide.md\0",
            f":100644 100644 {OLD} {NEW + NEW[:24]} M\0guide.md\0".encode(),
            "malformed-raw-record",
        ),
        (
            b"M  guide.md\0",
            f":000000 100644 {OLD} {OLD} M\0guide.md\0".encode(),
            "malformed-raw-record",
        ),
        (
            b"R  new.md\0old.md\0",
            f":100644 100644 {OLD} {NEW} R100\0old.md\0new.md\0".encode(),
            "malformed-raw-record",
        ),
        (
            b"M  guide.md\0",
            raw("100644", "M", "guide.md")[: -len(b"guide.md\0")],
            "missing-raw-path",
        ),
        (
            b"M  guide.md\0",
            f":100644 100644 {OLD} {NEW} M\0".encode() + b"\xff\0",
            "non-utf8-or-empty-raw-path",
        ),
    ],
)
def test_evidence_that_cannot_be_represented_is_uncertain(
    status: bytes, raw_diff: bytes, uncertainty: str
) -> None:
    taken = index.parse(status, raw_diff)
    assert uncertainty in taken.uncertainty
    assert index.classify(taken) is Reason.UNCERTAIN_STATUS


@pytest.mark.parametrize(
    ("old_mode", "new_mode", "old", "new", "status", "valid"),
    [
        ("100644", "100644", OLD, NEW, "M", True),
        ("100644", "100755", OLD, OLD, "M", True),
        ("100644", "100644", OLD, OLD, "M", False),
        ("100644", "120000", OLD, NEW, "M", False),
        ("100644", "120000", OLD, NEW, "T", True),
        ("100644", "100755", OLD, NEW, "T", False),
        ("100644", "100644", OLD, NEW, "U", True),
        ("100644", "100644", ZERO, NEW, "U", False),
        ("000000", "100644", ZERO, NEW, "A", True),
        ("000000", "100644", OLD, NEW, "A", False),
        ("100644", "000000", OLD, ZERO, "D", True),
        ("100644", "000000", OLD, NEW, "D", False),
        ("100644", "100644", OLD, NEW, "X", False),
        ("100644", "100644", OLD, NEW, "M1", False),
        ("100000", "100644", OLD, NEW, "M", False),
    ],
)
def test_raw_headers_admit_only_possible_transitions(
    old_mode: str, new_mode: str, old: str, new: str, status: str, valid: bool
) -> None:
    taken = index.parse(
        f"{status[0]}  guide.md\0".encode(),
        f":{old_mode} {new_mode} {old} {new} {status}\0guide.md\0".encode(),
    )
    assert ("malformed-raw-record" not in taken.uncertainty) is valid


# The stage plan.


def entry(fingerprint: str | None = None, **state: bool | int) -> PathStatus:
    status = PathStatus(worktree_fingerprint=fingerprint)
    for name, value in state.items():
        setattr(status, name, value)
    return status


def staged(fingerprint: str | None = None, **state: bool) -> PathStatus:
    return entry(fingerprint, staged=True, index_mode=0o100644, **state)


def snap(**paths: PathStatus) -> Snapshot:
    return Snapshot({path.replace("_", "."): status for path, status in paths.items()})


def test_only_cleanly_staged_tracked_paths_are_restaged() -> None:
    before = snap(
        a_rs=staged("old-a"),
        b_rs=entry("old-b", unstaged=True),
        scratch_rs=entry(untracked=True),
    )
    after = snap(
        a_rs=staged("new-a", unstaged=True),
        b_rs=entry("old-b", unstaged=True),
        scratch_rs=entry(untracked=True),
    )
    assert index.stage_plan(before, after) == index.StagePlan(["a.rs"], [])


def test_mixed_and_unstaged_only_mutations_are_refused() -> None:
    before = snap(
        mixed_rs=staged("old-mixed", unstaged=True),
        unstaged_rs=entry("old-unstaged", unstaged=True),
    )
    after = snap(
        mixed_rs=staged("new-mixed", unstaged=True),
        unstaged_rs=entry("new-unstaged", unstaged=True),
    )
    assert index.stage_plan(before, after) == index.StagePlan(
        [],
        [
            (
                "mixed.rs: pre-existing mixed staged/unstaged state made "
                "auto-staging unsafe"
            ),
            "unstaged.rs: will not add work the user did not stage",
        ],
    )


def test_a_clean_path_a_check_changed_is_refused() -> None:
    plan = index.stage_plan(snap(), snap(clean_rs=entry("new", unstaged=True)))
    assert plan.failures == ["clean.rs: will not add work the user did not stage"]


def test_old_untracked_files_are_tolerated_and_new_ones_refused() -> None:
    before = snap(old_tmp=entry(untracked=True))
    after = snap(old_tmp=entry(untracked=True), new_tmp=entry(untracked=True))
    assert index.stage_plan(before, after) == index.StagePlan(
        [], ["new.tmp: new untracked file created during precommit"]
    )
    changed = snap(old_tmp=entry("other", untracked=True))
    assert index.stage_plan(before, changed) == index.StagePlan([], [])
    assert index.stage_plan(before, snap()) == index.StagePlan([], [])


def test_unchanged_delete_or_rename_state_is_tolerated() -> None:
    state = snap(gone_rs=entry(staged=True, delete_or_rename=True), new_rs=staged())
    assert index.stage_plan(state, state) == index.StagePlan([], [])


def test_any_changed_path_beside_delete_or_rename_state_is_refused() -> None:
    before = snap(keep_rs=staged("old"))
    after = snap(
        keep_rs=entry("old", staged=True, delete_or_rename=True), new_rs=staged()
    )
    assert index.stage_plan(before, after).failures == [
        "keep.rs: delete/rename status is unsafe for auto-staging",
        "new.rs: delete/rename status is unsafe for auto-staging",
    ]


def test_changed_paths_beside_unsupported_index_evidence_are_refused() -> None:
    before = snap(guide_md=entry("before", staged=True, index_mode=0o120000))
    after = snap(guide_md=entry("after", staged=True, index_mode=0o120000))
    assert index.stage_plan(before, after).failures == [
        "guide.md: unsupported change or index mode is unsafe for auto-staging"
    ]


def test_incomplete_evidence_refuses_all_staging() -> None:
    before = Snapshot({}, ["b-reason", "a-reason"])
    after = Snapshot({"x": staged()}, ["a-reason"])
    assert index.stage_plan(before, after) == index.StagePlan(
        [],
        ["precommit staging requires complete git status evidence: a-reason, b-reason"],
    )


# Snapshots, staging and admission on a real repository, in process.


@pytest.fixture
def cwd_repo(repo: Repo, monkeypatch: pytest.MonkeyPatch) -> Repo:
    monkeypatch.chdir(repo.root)
    return repo


def reconcile(repo: Repo, *changes: tuple[str, str]) -> str:
    """Applies the plan from a snapshot before and after `changes`."""
    before = index.snapshot()
    for path, text in changes:
        repo.write(path, text)
    return index.apply(index.stage_plan(before, index.snapshot()))


def test_reconciliation_restages_only_the_previously_staged_file(
    cwd_repo: Repo,
) -> None:
    cwd_repo.write("guide.txt", "staged\n")
    cwd_repo.git("add", "guide.txt")
    cwd_repo.write("guide.md", "# Unstaged\n")
    detail = reconcile(cwd_repo, ("guide.txt", "staged\n// formatted\n"))
    assert detail == "staged: guide.txt"
    assert cwd_repo.git("diff", "--cached", "--name-only") == b"guide.txt\n"
    assert cwd_repo.git("diff", "--name-only") == b"guide.md\n"


def test_safe_paths_are_staged_beside_refusals(cwd_repo: Repo) -> None:
    cwd_repo.write("guide.txt", "two\n")
    cwd_repo.git("add", "guide.txt")
    with pytest.raises(Refused, match="new.tmp: new untracked"):
        reconcile(cwd_repo, ("guide.txt", "three\n"), ("new.tmp", "new\n"))
    assert cwd_repo.git("show", ":guide.txt") == b"three\n"


def test_a_mutated_partially_staged_file_is_refused(cwd_repo: Repo) -> None:
    cwd_repo.write("guide.txt", "two\n")
    cwd_repo.git("add", "guide.txt")
    cwd_repo.write("guide.txt", "three\n")
    with pytest.raises(Refused, match="pre-existing mixed"):
        reconcile(cwd_repo, ("guide.txt", "four\n"))
    assert cwd_repo.git("show", ":guide.txt") == b"two\n"
    assert (cwd_repo.root / "guide.txt").read_text() == "four\n"


@pytest.mark.parametrize("unstaged_before", [False, True])
def test_a_mutated_unstaged_file_is_refused(
    cwd_repo: Repo, unstaged_before: bool
) -> None:
    if unstaged_before:
        cwd_repo.write("guide.txt", "two\n")
    with pytest.raises(Refused, match="will not add work the user did not stage"):
        reconcile(cwd_repo, ("guide.txt", "three\n"))
    assert cwd_repo.git("diff", "--cached", "--name-only") == b""


@pytest.mark.parametrize("old", ["old.tmp", "scratch/old.tmp"])
def test_a_new_untracked_file_is_refused(cwd_repo: Repo, old: str) -> None:
    cwd_repo.write(old, "old\n")
    new = str(Path(old).with_name("new.tmp"))
    with pytest.raises(Refused, match=f"{new}: new untracked"):
        reconcile(cwd_repo, (new, "new\n"))
    assert cwd_repo.git("diff", "--cached", "--name-only") == b""


def test_a_path_git_cannot_add_is_reported(cwd_repo: Repo) -> None:
    with pytest.raises(
        Refused, match=r'^git \["add", "--", "missing"\] failed: .*missing'
    ):
        index.apply(index.StagePlan(["missing"], []))


def test_git_that_cannot_start_refuses_staging(
    cwd_repo: Repo, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("PATH", str(cwd_repo.root / "no-tools"))
    with pytest.raises(Refused, match=r'^running git \["add", "--", "guide.txt"\]'):
        index.apply(index.StagePlan(["guide.txt"], []))


def test_snapshots_never_detect_copies(cwd_repo: Repo) -> None:
    cwd_repo.git("config", "status.renames", "copies")
    cwd_repo.write("copy.txt", "original\n")
    cwd_repo.git("add", "copy.txt")
    taken = index.snapshot()
    assert taken.uncertainty == []
    assert list(taken.paths) == ["copy.txt"]
    assert taken.paths["copy.txt"].worktree_fingerprint is not None


def test_snapshots_fingerprint_only_regular_files(
    cwd_repo: Repo, monkeypatch: pytest.MonkeyPatch
) -> None:
    cwd_repo.write("guide.txt", "unstaged\n")
    (cwd_repo.root / "guide.md").unlink()
    (cwd_repo.root / "guide.md").symlink_to("guide.txt")
    (cwd_repo.root / "directory").mkdir()
    taken = index.snapshot()
    assert taken.paths["guide.txt"].worktree_fingerprint is not None
    # A type change is unsupported, and never fingerprinted.
    assert taken.paths["guide.md"].unsupported_change
    assert taken.paths["guide.md"].worktree_fingerprint is None
    # Neither is an unstaged path that is not a regular file, or is gone.
    unstaged = {path: entry(unstaged=True) for path in ["directory", "vanished"]}
    monkeypatch.setattr(index, "parse", lambda *_: Snapshot(unstaged, []))
    assert index.snapshot().paths == unstaged


def test_a_failed_git_query_refuses(cwd_repo: Repo) -> None:
    os.rename(cwd_repo.root / ".git", cwd_repo.root / "parked-git")
    try:
        with pytest.raises(Refused, match="^git status --porcelain=v1 -z failed: "):
            index.snapshot()
        with pytest.raises(Refused, match="^git ls-files --stage failed: "):
            index._worktree_matches_index()
    finally:
        os.rename(cwd_repo.root / "parked-git", cwd_repo.root / ".git")


def test_the_markdown_admission_needs_byte_identical_staged_markdown(
    cwd_repo: Repo,
) -> None:
    cwd_repo.write("guide.md", "# Guide\nChanged\n")
    cwd_repo.git("add", "guide.md")
    assert index.markdown_admission().paths["guide.md"].staged
    cwd_repo.write("guide.md", "# Guide\nNot staged\n")
    with pytest.raises(Refused, match="^unstaged or nonignored untracked changes"):
        index.markdown_admission()
    cwd_repo.git("add", "guide.md")
    cwd_repo.write("guide.txt", "staged\n")
    cwd_repo.git("add", "guide.txt")
    with pytest.raises(
        Refused,
        match="^post-reconciliation index is no longer Markdown-only: "
        "non-markdown-path$",
    ):
        index.markdown_admission()
    cwd_repo.git("restore", "--staged", "--worktree", "guide.txt")
    cwd_repo.git("rm", "-qf", "guide.md")
    assert "guide.md" in index.markdown_admission().paths


@pytest.mark.parametrize(
    ("attributes", "files"),
    [
        ("*.md text eol=crlf\n", {"untouched.md": "# Untouched\r\n"}),
        (
            "**/.prettierrc.json text eol=crlf\n**/.prettierignore text eol=crlf\n",
            {
                "docs/nested/.prettierrc.json": "{}\r\n",
                "docs/nested/.prettierignore": "*.tmp\r\n",
            },
        ),
        ("*.py text eol=crlf\n", {"tools/dev/dev/static.py": "CHECK = 1\r\n"}),
        # Every tracked file is an input of some shared verdict.
        ("*.el text eol=crlf\n", {"elisp/jaunder.el": "(provide 'jaunder)\r\n"}),
    ],
)
def test_a_clean_but_converting_checkout_is_never_admitted(
    cwd_repo: Repo, attributes: str, files: dict[str, str]
) -> None:
    cwd_repo.write(".gitattributes", attributes)
    for path, text in files.items():
        cwd_repo.write(path, text)
    cwd_repo.commit()
    assert cwd_repo.status() == b""
    with pytest.raises(Refused, match="^raw worktree bytes differ"):
        index.markdown_admission()
    with pytest.raises(Refused, match="^raw worktree bytes differ"):
        index.clean_admission()


def test_uncertain_evidence_is_never_admitted(
    cwd_repo: Repo, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        index, "parse", lambda *_: Snapshot({}, ["unterminated-status-record"])
    )
    with pytest.raises(
        Refused,
        match=r'^Git status evidence is uncertain: \["unterminated-status-record"\]$',
    ):
        index.markdown_admission()
    with pytest.raises(
        Refused,
        match=r'^prepush tree is not provably clean: \["unterminated-status-record"\]$',
    ):
        index.clean_admission()


def test_a_conflicted_docs_input_is_refused(cwd_repo: Repo) -> None:
    blob = cwd_repo.git("rev-parse", "HEAD:guide.md").decode().strip()
    subprocess.run(
        ["git", "update-index", "--index-info"],
        cwd=cwd_repo.root,
        input=f"0 {'0' * 40}\tguide.md\n100644 {blob} 1\tguide.md\n".encode(),
        check=True,
    )
    with pytest.raises(Refused, match="^non-stage-0 index entry: guide.md$"):
        index._worktree_matches_index()


def test_only_regular_files_are_hashed(cwd_repo: Repo) -> None:
    # A dangling symlink and a gitlink with no checkout would each fail
    # `hash-object`; the flake reads neither as file contents.
    (cwd_repo.root / "link").symlink_to("missing")
    cwd_repo.git("add", "link")
    cwd_repo.git("update-index", "--add", "--cacheinfo", f"160000,{'1' * 40},sub")
    index._worktree_matches_index()


@pytest.mark.parametrize(
    ("listing", "hashes", "refusal"),
    [
        (b"no tab\0", b"", "malformed NUL-delimited stage record"),
        (b"100644 x 0\t\xff.md\0", b"", "non-UTF-8 tracked path"),
        (b"\xff\tguide.md\0", b"", "non-UTF-8 index metadata"),
        (b"100644 x 0\tguide.txt", b"", "unterminated NUL-delimited index record"),
        (b"100644 x 0\tguide.md\0", b"\xff\n", "non-UTF-8 hash-object output"),
        (
            b"100644 x 0\tguide.md\0",
            b"",
            "hash-object returned an incomplete input set",
        ),
    ],
)
def test_index_listings_that_cannot_be_trusted_are_refused(
    monkeypatch: pytest.MonkeyPatch, listing: bytes, hashes: bytes, refusal: str
) -> None:
    def git(*args: str) -> subprocess.CompletedProcess[bytes]:
        output = listing if args[0] == "ls-files" else hashes
        return subprocess.CompletedProcess(["git", *args], 0, output, b"")

    monkeypatch.setattr(index, "_git", git)
    with pytest.raises(Refused, match=f"^{refusal}$"):
        index._worktree_matches_index()


def test_a_failed_hash_refuses(monkeypatch: pytest.MonkeyPatch) -> None:
    def git(*args: str) -> subprocess.CompletedProcess[bytes]:
        if args[0] == "hash-object":
            return subprocess.CompletedProcess([], 128, b"", b"fatal: gone\n")
        listing = b"100644 x 0\tguide.md\0" if args[0] == "ls-files" else b""
        return subprocess.CompletedProcess([], 0, listing, b"")

    monkeypatch.setattr(index, "_git", git)
    with pytest.raises(
        Refused, match="^git hash-object --no-filters failed: fatal: gone$"
    ):
        index._worktree_matches_index()
    monkeypatch.setattr(index, "parse", lambda *_: Snapshot({"guide.md": staged()}, []))
    with pytest.raises(
        Refused, match=r'^git \["hash-object", "--", "guide.md"\] failed: fatal: gone$'
    ):
        index.snapshot()


def test_a_failed_cached_diff_refuses(monkeypatch: pytest.MonkeyPatch) -> None:
    def git(*args: str) -> subprocess.CompletedProcess[bytes]:
        code = 128 if args[0] == "diff" else 0
        return subprocess.CompletedProcess([], code, b"", b"fatal: no index\n")

    monkeypatch.setattr(index, "_git", git)
    with pytest.raises(
        Refused, match="^git diff --cached --raw -z failed: fatal: no index$"
    ):
        index.snapshot()


def test_a_missing_git_refuses(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("PATH", str(tmp_path))
    with pytest.raises(Refused, match=r'^running git \["status"\]: '):
        index._git("status")
