"""Single-process custody of the Git index for `dev precommit` and `dev prepush`.

ADR-0029: the precommit snapshots the dirty tree, runs the host checks its
class selects, and then re-stages only provably safe formatter/check fixes,
also after a failed check. A Markdown-only change and prepush's Markdown slot
request the shared `static-docs` verdict between an admission and a mandatory
post-check. A signal is the one exception: a check or request killed by signal
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
from enum import StrEnum
from typing import NoReturn

from dev import steps

# Index modes of regular files, the only ones auto-staging and the narrow
# route accept.
REGULAR = (0o100644, 0o100755)
_KNOWN_MODES = (*REGULAR, 0o120000, 0o160000)


class Refused(Exception):
    """Git evidence, staging or the docs request refused; the message is shown."""


def shares_docs() -> bool:
    """Whether this host takes the shared `static-docs` verdict: x86_64 Linux."""
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
    """The complete dirty tree, its staged raw diff and any uncertain evidence.

    Uncertain evidence leaves the snapshot deliberately incomplete, so routing
    and reconciliation both fail closed.
    """

    paths: dict[str, PathStatus] = field(default_factory=dict)
    uncertainty: list[str] = field(default_factory=list)
    staged_raw_diff: bytes = b""


class Reason(StrEnum):
    """Why the precommit takes its route; only the first is narrow.

    The broad reasons are listed in their stable precedence.
    """

    ISOLATED_STAGED_MARKDOWN = "isolated-staged-markdown"
    UNCERTAIN_STATUS = "uncertain-status"
    EMPTY_STATE = "empty-state"
    UNTRACKED_PATH = "untracked-path"
    UNSTAGED_PATH = "unstaged-path"
    DELETE_OR_RENAME = "delete-or-rename"
    UNSUPPORTED_CHANGE = "unsupported-change"
    UNSUPPORTED_INDEX_MODE = "unsupported-index-mode"
    NON_MARKDOWN_PATH = "non-markdown-path"


@dataclass
class StagePlan:
    stage: list[str]
    failures: list[str]


@dataclass(frozen=True)
class _RawHeader:
    new_mode: int
    status: str


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


def _utf8(path: bytes) -> str | None:
    """`path` as text, or None when it is empty or not UTF-8."""
    try:
        return path.decode() or None
    except UnicodeDecodeError:
        return None


# Parsing.


def _nul_records(data: bytes, uncertainty: set[str], label: str) -> list[bytes]:
    if not data:
        return []
    records = data.split(b"\0")
    if data.endswith(b"\0"):
        records.pop()
    else:
        uncertainty.add(f"unterminated-{label}-record")
    if not all(records):
        uncertainty.add(f"interior-empty-{label}-record")
    return [record for record in records if record]


def _valid_status_pair(index: str, worktree: str) -> bool:
    if (index, worktree) == ("?", "?"):
        return True
    return index in " MADRCUT" and worktree in " MADUT"


def _mode(text: str) -> int | None:
    if len(text) == 6 and all(digit in "01234567" for digit in text):
        return int(text, 8)
    return None


def _object_id(text: str) -> bool:
    return len(text) in (40, 64) and all(
        digit in "0123456789abcdefABCDEF" for digit in text
    )


def _object_type(mode: int) -> int:
    return {0o100644: 1, 0o100755: 1, 0o120000: 2, 0o160000: 3}.get(mode, 0)


def _valid_transition(
    old_mode: int, new_mode: int, old_object: str, new_object: str, status: str
) -> bool:
    if len(old_object) != len(new_object):
        return False
    old_zero = set(old_object) == {"0"}
    new_zero = set(new_object) == {"0"}
    known = old_mode in _KNOWN_MODES and new_mode in _KNOWN_MODES
    match status:
        case "A":
            return (
                old_mode == 0 and new_mode in _KNOWN_MODES and old_zero and not new_zero
            )
        case "D":
            return (
                old_mode in _KNOWN_MODES and new_mode == 0 and not old_zero and new_zero
            )
        case "M":
            return (
                known
                and _object_type(old_mode) == _object_type(new_mode)
                and not old_zero
                and not new_zero
                and (old_mode != new_mode or old_object != new_object)
            )
        case "T":
            return (
                known
                and _object_type(old_mode) != _object_type(new_mode)
                and not old_zero
                and not new_zero
            )
        case _:
            # "U", the only other status a header admits.
            return known and not old_zero and not new_zero


def _raw_header(header: bytes) -> _RawHeader | None:
    """A cached raw diff header, or None for anything this custody cannot vouch for.

    A rename or copy (which `--no-renames` never reports) is not vouched for.
    """
    try:
        text = header.decode()
    except UnicodeDecodeError:
        return None
    fields = text.removeprefix(":").split(" ")
    if not text.startswith(":") or len(fields) != 5:
        return None
    old_mode, new_mode = _mode(fields[0]), _mode(fields[1])
    old_object, new_object, status = fields[2], fields[3], fields[4]
    if old_mode is None or new_mode is None or status not in ("A", "M", "D", "T", "U"):
        return None
    if not _object_id(old_object) or not _object_id(new_object):
        return None
    if not _valid_transition(old_mode, new_mode, old_object, new_object, status):
        return None
    return _RawHeader(new_mode, status)


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

    records = iter(_nul_records(status, uncertainty, "status"))
    for record in records:
        if len(record) < 4 or record[2:3] != b" ":
            uncertainty.add("malformed-status-record")
            continue
        index, worktree = chr(record[0]), chr(record[1])
        if not _valid_status_pair(index, worktree):
            uncertainty.add("unknown-status-pair")
            continue
        path = _utf8(record[3:])
        if path is None:
            uncertainty.add("non-utf8-or-empty-status-path")
            continue
        add(path, index, worktree)
        if index in ("R", "C"):
            old = next(records, None)
            old_path = None if old is None else _utf8(old)
            if old is None:
                uncertainty.add("missing-rename-path")
            elif old_path is None:
                uncertainty.add("non-utf8-or-empty-rename-path")
            else:
                add(old_path, index, worktree)

    staged_from_raw: set[str] = set()
    seen_raw: set[str] = set()
    records = iter(_nul_records(raw, uncertainty, "raw"))
    for header in records:
        parsed = _raw_header(header)
        if parsed is None:
            uncertainty.add("malformed-raw-record")
            continue
        path_bytes = next(records, None)
        if path_bytes is None:
            uncertainty.add("missing-raw-path")
            continue
        path = _utf8(path_bytes)
        if path is None:
            uncertainty.add("non-utf8-or-empty-raw-path")
            continue
        if path in seen_raw:
            uncertainty.add("duplicate-raw-path")
        seen_raw.add(path)
        entry = paths.setdefault(path, PathStatus())
        entry.delete_or_rename |= parsed.status == "D"
        entry.unsupported_change |= parsed.status not in ("A", "M", "D")
        entry.index_mode = parsed.new_mode
        entry.raw_change = parsed.status
        staged_from_raw.add(path)

    for entry in paths.values():
        if None not in (entry.staged_change, entry.raw_change) and (
            entry.staged_change != entry.raw_change
        ):
            uncertainty.add("status-raw-change-mismatch")
    if staged_from_status != staged_from_raw:
        uncertainty.add("status-raw-population-mismatch")
    return Snapshot(dict(sorted(paths.items())), sorted(uncertainty), raw)


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
    """The precommit route for `taken`; uncertainty takes the broad route."""
    statuses = taken.paths.values()
    if taken.uncertainty:
        return Reason.UNCERTAIN_STATUS
    if not taken.paths:
        return Reason.EMPTY_STATE
    if any(entry.untracked for entry in statuses):
        return Reason.UNTRACKED_PATH
    if any(entry.unstaged or not entry.staged for entry in statuses):
        return Reason.UNSTAGED_PATH
    if any(entry.delete_or_rename for entry in statuses):
        return Reason.DELETE_OR_RENAME
    if any(entry.unsupported_change for entry in statuses):
        return Reason.UNSUPPORTED_CHANGE
    if any(entry.index_mode not in REGULAR for entry in statuses):
        return Reason.UNSUPPORTED_INDEX_MODE
    if not all(path.endswith(".md") for path in taken.paths):
        return Reason.NON_MARKDOWN_PATH
    return Reason.ISOLATED_STAGED_MARKDOWN


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


# The shared static-docs request.


def _static_docs_input(path: bytes) -> bool:
    """A tracked file `static-docs` reads: Markdown, Prettier config, `dev`."""
    if path == b".claude" or path.startswith(b".claude/"):
        return False
    return (
        path.endswith((b".md", b"/.prettierrc.json", b"/.prettierignore"))
        or path in (b".prettierrc.json", b".prettierignore")
        # The `dev` build the check runs (`devSrc` in nix/packages.nix).
        or path == b"tools/dev/pyproject.toml"
        or (path.startswith(b"tools/dev/dev/") and path.endswith(b".py"))
    )


def _worktree_matches_index() -> None:
    """Refuses unless the raw worktree bytes of every docs input are its index blob.

    The Git flake reads worktree bytes, so a clean but converting checkout
    (an EOL attribute, say) would otherwise check bytes nobody staged.
    """
    listed = _git("ls-files", "--stage", "-z")
    if listed.returncode:
        raise Refused(f"git ls-files --stage failed: {_stderr(listed)}")
    selected: list[_IndexBlob] = []
    for record in filter(None, listed.stdout.split(b"\0")):
        metadata, tab, path_bytes = record.partition(b"\t")
        if not tab:
            raise Refused("malformed NUL-delimited stage record")
        if not _static_docs_input(path_bytes):
            continue
        try:
            path = path_bytes.decode()
        except UnicodeDecodeError as error:
            raise Refused("non-UTF-8 static-docs input path") from error
        try:
            fields = metadata.decode().split()
        except UnicodeDecodeError as error:
            raise Refused("non-UTF-8 index metadata") from error
        if len(fields) != 3 or fields[2] != "0":
            raise Refused(f"non-stage-0 index entry: {path}")
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
    """Admits only a reconciled tree of staged regular Markdown, byte-identical."""
    taken = snapshot()
    if taken.uncertainty:
        raise Refused(f"Git status evidence is uncertain: {_debug(taken.uncertainty)}")
    if any(entry.unstaged or entry.untracked for entry in taken.paths.values()):
        raise Refused(
            "unstaged or nonignored untracked changes remain after reconciliation"
        )
    for path, entry in taken.paths.items():
        if not (entry.staged and path.endswith(".md") and entry.index_mode in REGULAR):
            raise Refused(
                "post-reconciliation index is outside staged regular Markdown "
                f"admission: {path}"
            )
    _worktree_matches_index()
    return taken


def clean_admission() -> Snapshot:
    """Admits only a provably clean tree whose docs inputs match the index."""
    taken = snapshot()
    if taken.uncertainty or taken.paths:
        raise Refused(
            f"prepush tree is not provably clean: {_debug(taken.uncertainty)}"
        )
    _worktree_matches_index()
    return taken


def _unchanged(before: Snapshot) -> None:
    if snapshot() != before:
        raise Refused(
            "tracked, staged, or nonignored untracked Git state changed during "
            "static-docs request"
        )
    _worktree_matches_index()


# The request leaves flake.lock exactly as committed, as the clean-state
# admission and post-check assume.
STATIC_DOCS_FLAGS = (*steps.NIX_FLAGS, *steps.NIX_LOCK_FLAGS)


def static_docs(admission: Callable[[], Snapshot]) -> None:
    """Requests `static-docs` between `admission` and the mandatory post-check.

    The post-check runs even when the request fails, and its own failure never
    hides the request's. A request killed by a signal, or a termination request
    to `dev`, skips the post-check: the gate stops at once and exits 128+n.
    """
    try:
        with steps.phase("static-docs-admission"):
            before = admission()
    except Refused as refused:
        raise Refused(f"shared-static-docs-admission: {refused}") from refused
    request: str | None = None
    try:
        steps.nix_check("static-docs", STATIC_DOCS_FLAGS)
    except steps.StepFailed as failure:
        if failure.signalled:
            raise
        request = f"the static-docs request exited {failure.code}"
    try:
        with steps.phase("static-docs-post-check"):
            _unchanged(before)
    except Refused as refused:
        post = f"shared-static-docs-state: {refused}"
        raise Refused(post if request is None else f"{request}\n{post}") from refused
    print("shared-static-docs-state: unchanged", file=sys.stderr, flush=True)
    if request is not None:
        raise Refused(request)


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
    or admission failure skips the docs request.
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
    if markdown and shares_docs():
        try:
            static_docs(markdown_admission)
        except Refused as refused:
            _fail("precommit", str(refused))
    print("[ ok ] precommit", flush=True)


def prepush_docs() -> None:
    """Prepush's Markdown verdict: the shared request on a clean tree."""
    try:
        static_docs(clean_admission)
    except Refused as refused:
        _fail("prepush-docs", str(refused))
    print("[ ok ] prepush-docs", flush=True)
