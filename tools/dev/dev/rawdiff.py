"""Git raw diff records and the Markdown-only rule.

The one rule that decides whether a change touches Markdown alone, shared by
the precommit's narrow route (`index.classify`) and CI's documentation-only
scope (`scope`). Standard library only: CI runs it with the slim runner
image's own Python, before any Nix shell exists.
"""

from dataclasses import dataclass
from enum import StrEnum

# Index modes of regular files, the only ones auto-staging and the narrow
# route accept.
REGULAR = (0o100644, 0o100755)
_KNOWN_MODES = (*REGULAR, 0o120000, 0o160000)


@dataclass(frozen=True)
class Change:
    """One record of a raw Git diff; a mode is 0 on a side that does not exist."""

    old_mode: int
    new_mode: int
    status: str
    path: str


class Reason(StrEnum):
    """Why a change is Markdown-only or not; only the first is narrow.

    The other reasons are listed in their stable precedence.
    """

    ISOLATED_STAGED_MARKDOWN = "isolated-staged-markdown"
    UNCERTAIN_STATUS = "uncertain-status"
    EMPTY_STATE = "empty-state"
    UNTRACKED_PATH = "untracked-path"
    UNSTAGED_PATH = "unstaged-path"
    UNSUPPORTED_CHANGE = "unsupported-change"
    UNSUPPORTED_INDEX_MODE = "unsupported-index-mode"
    NON_MARKDOWN_PATH = "non-markdown-path"


@dataclass(frozen=True)
class _RawHeader:
    old_mode: int
    new_mode: int
    status: str


def utf8(path: bytes) -> str | None:
    """`path` as text, or None when it is empty or not UTF-8."""
    try:
        return path.decode() or None
    except UnicodeDecodeError:
        return None


def nul_records(data: bytes, uncertainty: set[str], label: str) -> list[bytes]:
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
    return _RawHeader(old_mode, new_mode, status)


def raw_changes(raw: bytes, uncertainty: set[str]) -> list[Change]:
    """The records of a `git diff --raw -z --no-abbrev --no-renames`.

    A record this module cannot vouch for is left out and recorded in
    `uncertainty`, so a caller with uncertainty must not trust the list.
    """
    changes: list[Change] = []
    seen: set[str] = set()
    records = iter(nul_records(raw, uncertainty, "raw"))
    for header in records:
        parsed = _raw_header(header)
        if parsed is None:
            uncertainty.add("malformed-raw-record")
            continue
        path_bytes = next(records, None)
        if path_bytes is None:
            uncertainty.add("missing-raw-path")
            continue
        path = utf8(path_bytes)
        if path is None:
            uncertainty.add("non-utf8-or-empty-raw-path")
            continue
        if path in seen:
            uncertainty.add("duplicate-raw-path")
        seen.add(path)
        changes.append(Change(parsed.old_mode, parsed.new_mode, parsed.status, path))
    return changes


def markdown_only(changes: list[Change]) -> Reason:
    """Whether a diff changes Markdown alone: the one rule the precommit's
    narrow route and CI's documentation-only scope share.

    Every record must add, modify or delete a case-sensitive `.md` path that is
    a regular file on each side that exists. A deleted or (as `--no-renames`
    reports it) renamed Markdown file qualifies: the documentation checks see
    what it breaks. Only the documentation consumes Markdown, so nothing else
    needs to run.
    """
    if not changes:
        return Reason.EMPTY_STATE
    if any(change.status not in ("A", "M", "D") for change in changes):
        return Reason.UNSUPPORTED_CHANGE
    if any(
        mode not in REGULAR
        for change in changes
        for mode in (change.old_mode, change.new_mode)
        if mode
    ):
        return Reason.UNSUPPORTED_INDEX_MODE
    if not all(change.path.endswith(".md") for change in changes):
        return Reason.NON_MARKDOWN_PATH
    return Reason.ISOLATED_STAGED_MARKDOWN
