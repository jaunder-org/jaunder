"""The sole ADR mutation owner: tracked promotion and README projection."""

import re
import stat
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from marko.ext.gfm import gfm
from marko.inline import Image, Link

from . import git


def _byte_boundaries(text: str) -> list[int]:
    """Translate Marko 2.2.4 normalization boundaries, not Markdown syntax."""
    boundaries = [0]
    offset = 0
    previous = ""
    for character in text:
        offset += len(character.encode("utf-8"))
        if previous == "\r" and character == "\n":
            boundaries[-1] = offset
        else:
            boundaries.append(offset)
        previous = character
    return boundaries


def rewrite_relative_links(source: bytes) -> bytes:
    """Remove one parent prefix only from parsed inline link/image destinations.

    Patching backwards preserves all original bytes outside destination spans,
    including the line endings and characters normalized by Marko for parsing.
    Reference definitions have no inline destination span and remain untouched.
    """
    edits = []
    for start, end, target in _destinations(source):
        if target.startswith(b"../") and len(target) > 3:
            edits.append((start, end, target[3:]))
    for start, end, replacement in sorted(edits, reverse=True):
        source = source[:start] + replacement + source[end:]
    return source


def _destinations(source):
    text = source.decode("utf-8")
    boundaries = _byte_boundaries(text)
    found = []

    def visit(node):
        if type(node) in (Link, Image) and node.dest_span is not None:
            start, end = (boundaries[index] for index in node.dest_span)
            target = source[start:end]
            found.append((start, end, target))
        for child in getattr(node, "children", ()):
            if not isinstance(child, str):
                visit(child)

    visit(gfm.parse(text))
    return sorted(found)


ADR_DIR = "docs/adr"
DRAFTS = "docs/adr/drafts"
README = "docs/README.md"
BEGIN = b"<!-- adr-table:begin -->"
END = b"<!-- adr-table:end -->"


def _regular_names(directory):
    # Like DirEntry::file_type, don't follow symlinks into the draft population.
    # lstat propagates metadata errors instead of silently shrinking the population.
    return [path.name for path in directory.iterdir() if stat.S_ISREG(path.lstat().st_mode)]


def _leading_number(name):
    match = re.match(r"[0-9]+", name)
    if match and int(match[0]) <= 0xFFFFFFFF:
        return int(match[0])
    return None


def _status(body):
    offset = 0
    # Rust's status locator splits only LF, not all Unicode line separators.
    for line in body.split(b"\n"):
        bare = line.removesuffix(b"\r")
        trimmed = bare.decode("utf-8").lstrip()
        for prefix in ("- Status:", "Status:"):
            if trimmed.startswith(prefix):
                return offset, len(bare), trimmed[len(prefix):].strip()
        offset += len(line) + 1
    return None


def _graduate(body, number, path):
    required = b"# ADR-DRAFT: "
    title = body[len(required):].split(b"\n", 1)[0].decode("utf-8").strip()
    if not body.startswith(required) or not title:
        raise ValueError(f"{path} must start with `# ADR-DRAFT: ` and a non-empty title")
    return f"# ADR-{number:04}: ".encode() + body[len(required):]


@dataclass(frozen=True)
class Row:
    number: int
    target: str
    title: str
    status: str


def _bounds(readme):
    begin, end = readme.find(BEGIN), readme.find(END)
    for index, marker in ((begin, BEGIN), (end, END)):
        if index < 0:
            raise ValueError(f"{README} is missing the `{marker.decode()}` marker")
    if begin >= end:
        raise ValueError(f"{README} adr-table markers are out of order")
    return begin + len(BEGIN), end


def _table_rows(block):
    rows = []
    for line in block.decode("utf-8").split("\n"):
        cells = [cell.strip() for cell in line.split("|")]
        if len(cells) != 5 or cells[0] or cells[-1]:
            continue
        match = re.fullmatch(r"\[([+]?[0-9]+)\]\((.*)\)", cells[1])
        if match and int(match[1]) <= 0xFFFFFFFF:
            rows.append(Row(int(match[1]), match[2], cells[2], cells[3]))
    return rows


def _heading_title(body):
    line = next((line for line in body.decode("utf-8").split("\n") if line.startswith("# ")), "")
    after = line
    while after.startswith("# "):
        after = after[2:]
    after = after.strip()
    for prefix, separator in (("ADR-", ": "), ("", ". ")):
        lhs, found, title = after.partition(separator)
        if found and lhs.startswith(prefix) and lhs and all(c in "0123456789" for c in lhs[len(prefix):]):
            return title.strip()
    return after


def sync_readme(repo: Path) -> str:
    """Regenerate mechanical rows while retaining curated titles and semantic no-op."""
    path = repo / README
    readme = path.read_bytes()
    readme.decode("utf-8")
    entries = []
    for name in sorted(_regular_names(repo / ADR_DIR)):
        number = _leading_number(name)
        if number is None or not name.endswith(".md"):
            continue
        body = (repo / ADR_DIR / name).read_bytes()
        status = _status(body)
        token = status[2].split()[0] if status and status[2].split() else ""
        entries.append(Row(number, f"adr/{name}", _heading_title(body), token))
    start, end = _bounds(readme)
    existing = _table_rows(readme[start:end])
    titles = {row.number: row.title for row in existing}
    desired = [
        Row(row.number, row.target, titles.get(row.number, row.title), row.status)
        for row in entries
    ]
    desired.sort(key=lambda row: row.number)
    if desired == existing:
        return f"{len(desired)} rows, already in sync"
    lines = ["| #   | Title | Status |", "| --- | ----- | ------ |"]
    lines.extend(f"| [{row.number:04}]({row.target}) | {row.title} | {row.status} |" for row in desired)
    block = "\n".join(lines).rstrip().encode()
    path.write_bytes(readme[:start] + b"\n\n" + block + b"\n\n" + readme[end:])
    old_numbers = {row.number for row in existing}
    new_numbers = {row.number for row in desired}
    return (
        f"{len(desired)} rows ({len(new_numbers - old_numbers)} added, "
        f"{len(old_numbers - new_numbers)} removed)"
    )


def _dead_links(repo, relative):
    tracked = set(git.paths(repo, "ls-files", "-z"))
    body = (repo / relative).read_bytes()
    dead = []
    for _, _, raw in _destinations(body):
        target = raw.decode("utf-8")
        if target.startswith(("#", "http://", "https://", "mailto:")):
            continue
        bare = target.split("#", 1)[0]
        if not bare:
            continue
        parts = list(PurePosixPath(relative).parent.parts)
        valid = not bare.startswith("/")
        for part in bare.split("/"):
            if part == "..":
                if parts:
                    parts.pop()
                else:
                    valid = False
            elif part and part != ".":
                parts.append(part)
        resolved = "/".join(parts)
        live_file = not bare.endswith("/") and resolved in tracked
        live_directory = any(path.startswith(resolved + "/") for path in tracked)
        if not valid or not (live_file or live_directory):
            dead.append(target)
    return dead


def promote(repo: Path) -> str:
    """Admit the complete set before moving; subsequent I/O is not transactional."""
    slugs = sorted(
        name[:-3] for name in _regular_names(repo / DRAFTS)
        if name.endswith(".md") and name != "README.md"
    )
    if not slugs:
        return "no ADR drafts to promote"
    numbers = [
        number for name in _regular_names(repo / ADR_DIR)
        if (number := _leading_number(name)) is not None
    ]
    next_number = max(numbers) + 1 if numbers else 0
    assigned = []
    for index, slug in enumerate(slugs):
        number = next_number + index
        old = f"{DRAFTS}/{slug}.md"
        new = f"{ADR_DIR}/{number:04}-{slug}.md"
        body = (repo / old).read_bytes()
        body.decode("utf-8")
        assigned.append((slug, number, old, new, _graduate(body, number, old)))
    tracked = set(git.paths(repo, "ls-files", "-z", "--", "*.md"))
    for _, _, old, _, _ in assigned:
        if old not in tracked:
            raise ValueError(f"{old} must be tracked before promotion")
    summaries = []
    for _, _, old, new, body in assigned:
        body = rewrite_relative_links(body)
        status = _status(body)
        accepted = status is not None and status[2] == "proposed"
        if accepted:
            offset, length, _ = status
            body = (
                body[:offset]
                + body[offset:offset + length].replace(b"proposed", b"accepted", 1)
                + body[offset + length:]
            )
        git.run(repo, "mv", "--", old, new)
        (repo / new).write_bytes(body)
        git.run(repo, "add", "--", new)
        summaries.append(f"{old} -> {new}" + (" (status: proposed -> accepted)" if accepted else ""))
    for slug, number, _, _, _ in assigned:
        old_stem, new_stem = f"drafts/{slug}", f"{number:04}-{slug}"
        matching = git.paths(
            repo, "grep", "-l", "-z", "--fixed-strings", "--", old_stem, allowed=(0, 1)
        )
        for relative in matching:
            path = repo / relative
            body = path.read_bytes()
            path.write_bytes(body.replace(old_stem.encode(), new_stem.encode()))
            git.run(repo, "add", "--", relative)
    try:
        readme = (repo / README).read_bytes()
        readme.decode("utf-8")
    except FileNotFoundError:
        readme = b""
    if BEGIN in readme and END in readme:
        note = f"README table synced ({sync_readme(repo)})"
        git.run(repo, "add", "--", README)
    else:
        note = "README table not synced (no adr-table markers)"
    warnings = []
    for _, _, _, new, _ in assigned:
        try:
            dead = _dead_links(repo, new)
            if dead:
                warnings.append(f"{new}: {', '.join(dead)}")
        except (OSError, UnicodeError, RuntimeError) as error:
            # Post-mutation inspection is advisory; the stable-tree gates enforce it.
            warnings.append(f"{new}: unreadable ({error})")
    warning = "; warning: unresolved link(s) — " + "; ".join(warnings) if warnings else ""
    return "; ".join(summaries) + f" — {note}; staged" + warning
