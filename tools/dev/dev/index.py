"""Single-process custody of the Git index for `dev precommit` and `dev prepush`.

ADR-0029: the precommit snapshots the dirty tree, runs the host checks its
class selects, and then re-stages only provably safe formatter/check fixes,
also after a failed check. A Markdown-only change requests the shared
`static-docs` verdict, and prepush runs all of its checks, some of them shared
Nix verdicts, between an admission and a mandatory post-check. A signal is the one exception: a check or request killed by signal
n, or a termination request to `dev`, skips the reconciliation and the
post-check, and `dev` exits 128+n.

Every Git query here runs on the `git` CLI with the hook's environment, so a
hook's GIT_DIR or temporary GIT_INDEX_FILE (as `git commit <path>` sets) is
the index the evidence describes. Steps never inherit them (`steps`). Evidence
that cannot be represented exactly is uncertain, and uncertainty refuses.
"""

import json
import os
import platform
import stat
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import NoReturn

from dev import steps
from dev.rawdiff import (
    REGULAR,
    Change,
    Reason,
    markdown_only,
    nul_records,
    raw_changes,
    utf8,
)


class Refused(Exception):
    """Git evidence, staging or custody refused; the message is shown."""


def shares_verdicts() -> bool:
    """Whether this host takes the shared Nix static verdicts: x86_64 Linux,
    the only system `dev`'s flake checks name and Cachix holds."""
    return (platform.system(), platform.machine()) == ("Linux", "x86_64")


@dataclass
class PathStatus:
    staged: bool = False
    unstaged: bool = False
    untracked: bool = False
    delete_or_rename: bool = False
    unsupported_change: bool = False
    index_mode: int | None = None
    staged_change: str | None = None
    raw_change: str | None = None
    worktree_fingerprint: str | None = None


@dataclass
class Snapshot:
    """The complete dirty tree, its staged changes and any uncertain evidence.

    Uncertain evidence leaves the snapshot deliberately incomplete, so routing
    and reconciliation both fail closed.
    """

    paths: dict[str, PathStatus] = field(default_factory=dict)
    uncertainty: list[str] = field(default_factory=list)
    staged_changes: list[Change] = field(default_factory=list)


@dataclass
class StagePlan:
    stage: list[str]
    failures: list[str]


@dataclass(frozen=True)
class _IndexBlob:
    path: str
    object_id: str


def _debug(values: list[str]) -> str:
    """`values` as the messages have always shown a list: `["a", "b"]`."""
    return (
        "[" + ", ".join(json.dumps(value, ensure_ascii=False) for value in values) + "]"
    )


def _git(*args: str) -> subprocess.CompletedProcess[bytes]:
    try:
        return subprocess.run(["git", *args], capture_output=True, check=False)
    except OSError as error:
        raise Refused(f"running git {_debug(list(args))}: {error}") from error


def _stderr(done: subprocess.CompletedProcess[bytes]) -> str:
    return done.stderr.decode(errors="replace").strip()


# Parsing.


def _valid_status_pair(index: str, worktree: str) -> bool:
    if (index, worktree) == ("?", "?"):
        return True
    return index in " MADRCUT" and worktree in " MADUT"


def parse(status: bytes, raw: bytes) -> Snapshot:
    """A snapshot from NUL-delimited porcelain v1 status and cached raw diff.

    Status gives each path's index and worktree state; the raw diff gives each
    staged path's index mode and must describe the same staged population.
    """
    paths: dict[str, PathStatus] = {}
    uncertainty: set[str] = set()
    staged_from_status: set[str] = set()
    seen_status: set[tuple[str, str, str]] = set()

    def add(path: str, index: str, worktree: str) -> None:
        if (path, index, worktree) in seen_status:
            uncertainty.add("duplicate-status-path")
        seen_status.add((path, index, worktree))
        entry = paths.setdefault(path, PathStatus())
        if (index, worktree) == ("?", "?"):
            entry.untracked = True
            return
        entry.staged |= index != " "
        entry.unstaged |= worktree != " "
        entry.delete_or_rename |= index in "DR" or worktree in "DR"
        entry.unsupported_change |= index in "TUC" or worktree in "TUC"
        if index != " ":
            entry.staged_change = index
        if entry.staged:
            staged_from_status.add(path)

    records = iter(nul_records(status, uncertainty, "status"))
    for record in records:
        if len(record) < 4 or record[2:3] != b" ":
            uncertainty.add("malformed-status-record")
            continue
        index, worktree = chr(record[0]), chr(record[1])
        if not _valid_status_pair(index, worktree):
            uncertainty.add("unknown-status-pair")
            continue
        path = utf8(record[3:])
        if path is None:
            uncertainty.add("non-utf8-or-empty-status-path")
            continue
        add(path, index, worktree)
        if index in ("R", "C"):
            old = next(records, None)
            old_path = None if old is None else utf8(old)
            if old is None:
                uncertainty.add("missing-rename-path")
            elif old_path is None:
                uncertainty.add("non-utf8-or-empty-rename-path")
            else:
                add(old_path, index, worktree)

    changes = raw_changes(raw, uncertainty)
    for change in changes:
        entry = paths.setdefault(change.path, PathStatus())
        entry.delete_or_rename |= change.status == "D"
        entry.unsupported_change |= change.status not in ("A", "M", "D")
        entry.index_mode = change.new_mode
        entry.raw_change = change.status
    staged_from_raw = {change.path for change in changes}

    for entry in paths.values():
        if None not in (entry.staged_change, entry.raw_change) and (
            entry.staged_change != entry.raw_change
        ):
            uncertainty.add("status-raw-change-mismatch")
    if staged_from_status != staged_from_raw:
        uncertainty.add("status-raw-population-mismatch")
    return Snapshot(dict(sorted(paths.items())), sorted(uncertainty), changes)


# Snapshots.


def _fingerprintable(path: str, entry: PathStatus) -> bool:
    """Whether the worktree bytes of `path` decide if a check changed it."""
    if entry.untracked or entry.delete_or_rename or entry.unsupported_change:
        return False
    if entry.index_mode in REGULAR:
        return True
    if entry.staged or not entry.unstaged:
        return False
    try:
        return stat.S_ISREG(os.lstat(path).st_mode)
    except OSError:
        return False


def snapshot() -> Snapshot:
    """The complete dirty tree with its staged index evidence.

    NUL-delimited porcelain keeps whitespace and cannot mistake a path for
    status syntax; the cached raw diff supplies index modes.
    """
    status = _git(
        "status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames"
    )
    if status.returncode:
        raise Refused(f"git status --porcelain=v1 -z failed: {_stderr(status)}")
    raw = _git("diff", "--cached", "--raw", "-z", "--no-abbrev", "--no-renames")
    if raw.returncode:
        raise Refused(f"git diff --cached --raw -z failed: {_stderr(raw)}")
    taken = parse(status.stdout, raw.stdout)
    if not taken.uncertainty:
        for path, entry in taken.paths.items():
            if _fingerprintable(path, entry):
                hashed = _git("hash-object", "--", path)
                if hashed.returncode:
                    args = _debug(["hash-object", "--", path])
                    raise Refused(f"git {args} failed: {_stderr(hashed)}")
                entry.worktree_fingerprint = hashed.stdout.decode(
                    errors="replace"
                ).strip()
    return taken


def classify(taken: Snapshot) -> Reason:
    """The precommit route for `taken`; uncertainty takes the broad route.

    Beyond the shared rule, the commit must be the whole dirty tree: nothing
    untracked or unstaged that the checks would see but the commit would not.
    """
    statuses = taken.paths.values()
    if taken.uncertainty:
        return Reason.UNCERTAIN_STATUS
    if not taken.paths:
        return Reason.EMPTY_STATE
    if any(entry.untracked for entry in statuses):
        return Reason.UNTRACKED_PATH
    if any(entry.unstaged or not entry.staged for entry in statuses):
        return Reason.UNSTAGED_PATH
    return markdown_only(taken.staged_changes)


def routing(reason: Reason) -> str:
    if reason is Reason.ISOLATED_STAGED_MARKDOWN:
        return f"class=staged-markdown-only reason={reason}"
    return f"class=broad reason={reason}"


# Staging reconciliation.


def _changed(before: PathStatus | None, after: PathStatus | None) -> bool:
    def comparable(
        entry: PathStatus | None,
    ) -> tuple[bool, bool, bool, bool, str | None] | None:
        if entry is None:
            return None
        return (
            entry.staged,
            entry.unstaged,
            entry.untracked,
            entry.delete_or_rename,
            entry.worktree_fingerprint,
        )

    return comparable(before) != comparable(after)


def stage_plan(before: Snapshot, after: Snapshot) -> StagePlan:
    """Which changed paths to re-stage, and why each other change refuses.

    Only a path the user staged whole (no unstaged change of their own) is
    re-staged; anything else a check changed is never added.
    """
    if before.uncertainty or after.uncertainty:
        evidence = sorted({*before.uncertainty, *after.uncertainty})
        return StagePlan(
            [],
            [
                "precommit staging requires complete git status evidence: "
                + ", ".join(evidence)
            ],
        )
    entries = [*before.paths.values(), *after.paths.values()]
    delete_or_rename = any(entry.delete_or_rename for entry in entries)
    unsupported = any(
        entry.unsupported_change
        or (entry.index_mode is not None and entry.index_mode not in REGULAR)
        for entry in entries
    )
    stage: list[str] = []
    failures: list[str] = []
    for path in sorted({*before.paths, *after.paths}):
        was, now = before.paths.get(path), after.paths.get(path)
        if not _changed(was, now):
            continue
        if delete_or_rename:
            failures.append(f"{path}: delete/rename status is unsafe for auto-staging")
        elif unsupported:
            failures.append(
                f"{path}: unsupported change or index mode is unsafe for auto-staging"
            )
        elif now is not None and now.untracked:
            if was is None:
                failures.append(f"{path}: new untracked file created during precommit")
        elif was is not None and was.untracked:
            pass
        elif was is not None and was.staged and not was.unstaged:
            stage.append(path)
        elif was is not None and was.staged:
            failures.append(
                f"{path}: pre-existing mixed staged/unstaged state made "
                "auto-staging unsafe"
            )
        else:
            failures.append(f"{path}: will not add work the user did not stage")
    return StagePlan(stage, sorted(set(failures)))


def apply(plan: StagePlan) -> str:
    """Stages the plan's safe paths, even beside refusals; returns the detail."""
    failures = list(plan.failures)
    for path in plan.stage:
        added = _git("add", "--", path)
        if added.returncode:
            failures.append(
                f"git {_debug(['add', '--', path])} failed: {_stderr(added)}"
            )
    if failures:
        raise Refused("\n".join(sorted(set(failures))))
    if not plan.stage:
        return "no staged fixes"
    return "staged: " + ", ".join(plan.stage)


# Custody of the shared Nix verdict requests.

# Index modes whose worktree bytes the Git flake reads as file contents. A
# symlink is read as a link and a gitlink not at all, so neither is hashed.
_REGULAR_MODES = ("100644", "100755")


def _worktree_matches_index() -> None:
    """Refuses unless every tracked regular file's raw worktree bytes are its
    index blob.

    The Git flake reads worktree bytes, so a clean but converting checkout
    (an EOL attribute, say) would otherwise check bytes nobody staged. The
    whole tracked tree is compared, so no list of what each check reads has
    to mirror its Nix source filter.
    """
    listed = _git("ls-files", "--stage", "-z")
    if listed.returncode:
        raise Refused(f"git ls-files --stage failed: {_stderr(listed)}")
    selected: list[_IndexBlob] = []
    for record in filter(None, listed.stdout.split(b"\0")):
        metadata, tab, path_bytes = record.partition(b"\t")
        if not tab:
            raise Refused("malformed NUL-delimited stage record")
        try:
            path = path_bytes.decode()
        except UnicodeDecodeError as error:
            raise Refused("non-UTF-8 tracked path") from error
        try:
            fields = metadata.decode().split()
        except UnicodeDecodeError as error:
            raise Refused("non-UTF-8 index metadata") from error
        if len(fields) != 3 or fields[2] != "0":
            raise Refused(f"non-stage-0 index entry: {path}")
        if fields[0] in _REGULAR_MODES:
            selected.append(_IndexBlob(path, fields[1]))
    if listed.stdout and not listed.stdout.endswith(b"\0"):
        raise Refused("unterminated NUL-delimited index record")
    if not selected:
        return
    hashed = _git(
        "hash-object", "--no-filters", "--", *(blob.path for blob in selected)
    )
    if hashed.returncode:
        raise Refused(f"git hash-object --no-filters failed: {_stderr(hashed)}")
    try:
        actual = hashed.stdout.decode().splitlines()
    except UnicodeDecodeError as error:
        raise Refused("non-UTF-8 hash-object output") from error
    if len(actual) != len(selected):
        raise Refused("hash-object returned an incomplete input set")
    for blob, object_id in zip(selected, actual, strict=True):
        if object_id != blob.object_id:
            raise Refused(
                f"raw worktree bytes differ from staged index blob: {blob.path}"
            )


def markdown_admission() -> Snapshot:
    """Admits only a reconciled tree that is clean or still Markdown-only, with
    byte-identical tracked files."""
    taken = snapshot()
    if taken.uncertainty:
        raise Refused(f"Git status evidence is uncertain: {_debug(taken.uncertainty)}")
    if any(entry.unstaged or entry.untracked for entry in taken.paths.values()):
        raise Refused(
            "unstaged or nonignored untracked changes remain after reconciliation"
        )
    reason = markdown_only(taken.staged_changes)
    if taken.staged_changes and reason is not Reason.ISOLATED_STAGED_MARKDOWN:
        raise Refused(f"post-reconciliation index is no longer Markdown-only: {reason}")
    _worktree_matches_index()
    return taken


def clean_admission() -> Snapshot:
    """Admits only a provably clean tree whose tracked files match the index."""
    taken = snapshot()
    if taken.uncertainty or taken.paths:
        raise Refused(
            f"prepush tree is not provably clean: {_debug(taken.uncertainty)}"
        )
    _worktree_matches_index()
    return taken


def _unchanged(before: Snapshot, name: str) -> None:
    if snapshot() != before:
        raise Refused(
            f"tracked, staged, or nonignored untracked Git state changed during {name}"
        )
    _worktree_matches_index()


# Shared requests leave flake.lock exactly as committed, as the clean-state
# admission and post-check assume.
SHARED_FLAGS = (*steps.NIX_FLAGS, *steps.NIX_LOCK_FLAGS)


def custody(
    name: str, admission: Callable[[], Snapshot], body: Callable[[], None]
) -> None:
    """Runs `body`, which requests shared Nix verdicts, between `admission` and
    the mandatory post-check, both phases named after `name`.

    The Git flake reads worktree bytes: the admission proves they are the
    index's, and the post-check that no Git state changed while `body` ran. The
    post-check runs even after a failed step in `body`. If it passes, that
    failure is raised unchanged; if it fails too, the refusal names both. A
    step killed by
    a signal, or a termination request to `dev`, skips the post-check: the gate
    stops at once and exits 128+n.
    """
    try:
        with steps.phase(f"{name}-admission"):
            before = admission()
    except Refused as refused:
        raise Refused(f"{name}-admission: {refused}") from refused
    failed: steps.StepFailed | None = None
    try:
        body()
    except steps.StepFailed as failure:
        if failure.signalled:
            raise
        failed = failure
    try:
        with steps.phase(f"{name}-post-check"):
            _unchanged(before, name)
    except Refused as refused:
        post = f"{name}-state: {refused}"
        if failed is not None:
            post = f"a step exited {failed.code}\n{post}"
        raise Refused(post) from refused
    print(f"{name}-state: unchanged", file=sys.stderr, flush=True)
    if failed is not None:
        raise failed


# The custody operations.


def _fail(operation: str, message: str) -> NoReturn:
    print(f"[FAIL] {operation} — {message}", file=sys.stderr, flush=True)
    raise steps.StepFailed(1, signalled=False)


def precommit(checks: Callable[[bool], None]) -> None:
    """The pre-commit gate around `checks`, called with whether the change is
    Markdown-only.

    The after-snapshot and one staging reconciliation run even when the checks
    fail. The exception is a signal: a check killed by signal n, or a
    termination request to `dev`, stops the gate at once, skipping the
    reconciliation and the docs post-check, and `dev` exits 128+n. Host, staging
    or admission failure skips the docs request; a failed request exits with
    its own status once the post-check passes.
    """
    try:
        with steps.phase("snapshot-before"):
            before = snapshot()
    except Refused as refused:
        _fail("precommit", f"precommit before-snapshot: {refused}")
    reason = classify(before)
    print(f"precommit-routing: {routing(reason)}", flush=True)
    markdown = reason is Reason.ISOLATED_STAGED_MARKDOWN
    host = "precommit-markdown-host" if markdown else "precommit-broad-host"
    failures: list[str] = []
    try:
        with steps.phase(host):
            checks(markdown)
    except steps.StepFailed as failure:
        if failure.signalled:
            raise
        failures.append(f"precommit-host: {host} exited {failure.code}")
    try:
        with steps.phase("snapshot-after"):
            try:
                after = snapshot()
            except Refused as refused:
                raise Refused(
                    f"after-snapshot failed; refusing staging: {refused}"
                ) from refused
        with steps.phase("staging"):
            detail = apply(stage_plan(before, after))
        print(f"precommit-staging: {detail}", file=sys.stderr, flush=True)
    except Refused as refused:
        print(f"precommit-staging: {refused}", file=sys.stderr, flush=True)
        failures.append(str(refused))
    if failures:
        _fail("precommit", "\n".join(failures))
    if markdown and shares_verdicts():
        try:
            custody(
                "static-docs",
                markdown_admission,
                lambda: steps.nix_check("static-docs", SHARED_FLAGS),
            )
        except Refused as refused:
            _fail("precommit", str(refused))
    print("[ ok ] precommit", flush=True)


def prepush(checks: Callable[[], None]) -> None:
    """The pre-push gate's `checks`, which request shared verdicts, in custody
    on a clean tree."""
    try:
        custody("prepush", clean_admission, checks)
    except Refused as refused:
        _fail("prepush", str(refused))
