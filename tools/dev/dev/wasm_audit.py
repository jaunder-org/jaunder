"""Frontend bundle weight: the CSR bundle's sizes, its raw wasm budget, and a
manual breakdown of where the bytes come from (#836, ADR-0106).

The budget is on **raw** bytes, not brotli: raw is what the browser's wasm
compiler reads, and compile time dominates the boot gap (#818). A budget on the
compressed figure would pass a change that compresses better while compiling
slower.

`--breakdown` reads the unstripped `.#csrWasm`, the only artifact that still has
a name section: the shipped bundle has none once `wasm-opt` has run, so it
cannot be attributed. twiggy reads the names, sizes and demangling; this module
walks only the section headers and rolls twiggy's items up by crate.
"""

import json
import re
from collections import Counter
from pathlib import Path
from typing import Any, NamedTuple

from dev import steps
from dev.report import Align, table

# The Vite manifest key of wasm-bindgen's wasm. Spelled in Rust too
# (`host::vite_manifest`); ADR-0109's literal table keeps the two equal.
WASM_SOURCE = "csr_bg.wasm"

# Raw bytes of the selected wasm after #1625's Manage Posts workspace, under
# `wasm-opt -Oz`. A passing budget reports its drift from this. A few bytes is
# build noise (the artifact is not bit-reproducible); kilobytes mean change.
WASM_RAW_ACHIEVED_BYTES = 4_009_656
# The inclusive ceiling: decimal 5 MB, not 5 MiB, accepted for #1750's feature
# headroom (ADR-0106's amendment). The achieved baseline stays historical.
WASM_RAW_CEILING_BYTES = 5_000_000

# The `\0asm` magic and version that precede the first section.
HEADER_BYTES = 8
SECTIONS = {
    1: "type",
    2: "import",
    3: "function",
    4: "table",
    5: "memory",
    6: "global",
    7: "export",
    8: "start",
    9: "element",
    10: "code",
    11: "data",
    12: "data-count",
    13: "tag",
}
# twiggy's names for the items that are not functions: they are charged to
# their sections in the section table, never to a crate.
NOT_A_FUNCTION = re.compile(
    r'"[^"]*" subsection$|data segment |custom section |export |import '
    r"|(type|elem|global|table|memory)\[\d+\]|wasm magic bytes$|.* section headers$"
)
UNATTRIBUTED = "<unattributed>"


class Artifact(NamedTuple):
    """A selected bundle output and the sizes of its precompressed copies."""

    path: str
    raw: int
    gzip: int
    brotli: int


def _listed_path(root: Path, value: object, owner: str) -> str:
    """A manifest-listed output path, refused unless it is safe and present.

    Safe is `host::vite_manifest`'s rule: every segment is a plain name, so
    the path stays below `root` and has one spelling.
    """
    if not isinstance(value, str):
        steps.refuse(f"Vite manifest entry {owner!r} must list output paths")
    if {"", ".", ".."} & set(value.split("/")):
        steps.refuse(f"unsafe Vite output path: {value!r}")
    if not (root / value).is_file():
        steps.refuse(f"Vite output file is missing: {value}")
    return value


def outputs(root: Path) -> tuple[str, list[str]]:
    """The selected wasm and every output the bundle's Vite manifest lists.

    The checks are `host::vite_manifest`'s, less its `index.html` entry, which
    nothing here reads and `server/build.rs` still validates.
    """
    path = root / ".vite/manifest.json"
    try:
        manifest: Any = json.loads(path.read_bytes())
    except (OSError, ValueError) as error:
        steps.refuse(f"reading Vite manifest {path}: {error}")
    if not isinstance(manifest, dict):
        steps.refuse("Vite manifest root must be an object")
    files = set()
    for key, entry in manifest.items():
        if not isinstance(entry, dict) or "file" not in entry:
            steps.refuse(f"Vite manifest has no output file for {key!r}")
        files.add(_listed_path(root, entry["file"], key))
        for field in ("css", "assets"):
            listed = entry.get(field, [])
            if not isinstance(listed, list):
                steps.refuse(
                    f"Vite manifest entry {key!r} field {field!r} must be a list"
                )
            files.update(_listed_path(root, value, key) for value in listed)
    if WASM_SOURCE not in manifest:
        steps.refuse(f"Vite manifest has no output file for {WASM_SOURCE!r}")
    wasm: str = manifest[WASM_SOURCE]["file"]
    if not wasm.endswith(".wasm"):
        steps.refuse(f"Vite {WASM_SOURCE} mapping must resolve to a WASM output")
    return wasm, sorted(files)


def measure(root: Path) -> list[Artifact]:
    """The selected wasm, then each listed JavaScript output, with the sizes of
    the `.gz` and `.br` copies Vite writes beside each."""
    wasm, files = outputs(root)
    artifacts = []
    for name in [wasm, *(file for file in files if file.endswith(".js"))]:
        sizes = []
        for suffix in ("", ".gz", ".br"):
            try:
                sizes.append((root / f"{name}{suffix}").stat().st_size)
            except OSError as error:
                steps.refuse(f"reading Vite output {name}{suffix}: {error}")
        artifacts.append(Artifact(name, *sizes))
    return artifacts


def format_bytes(count: int) -> str:
    """Bytes in the largest binary unit under 1024 of it: whole numbers for
    bytes and for values of 10 or more, one decimal otherwise."""
    value, unit = float(count), 0
    units = ("B", "KiB", "MiB", "GiB")
    while value >= 1024 and unit < len(units) - 1:
        value /= 1024
        unit += 1
    decimals = 0 if value >= 10 or unit == 0 else 1
    return f"{value:.{decimals}f} {units[unit]}"


def _percent(part: int, whole: int) -> str:
    return "n/a" if whole == 0 else f"{part / whole * 100:.1f}%"


def report(link: str) -> None:
    """Prints the size table of the bundle at `link`, named by its target."""
    root = Path(link).resolve()
    with steps.phase("wasm-audit"):
        artifacts = measure(root)
        lines = [
            "WASM bundle audit",
            f"bundle output: {root}",
            table(
                ["artifact", "raw", "gzip", "brotli"],
                [
                    [a.path, *map(format_bytes, (a.raw, a.gzip, a.brotli))]
                    for a in artifacts
                ],
                ["left", "right", "right", "right"],
            ),
        ]
        print("\n".join(lines))


def failure_message(actual: int) -> str:
    """The numbers, and the one edit that resolves an intended overrun, so the
    reader need not guess whether to shrink the bundle or move the line."""
    return (
        f"raw manifest-selected WASM identity artifact is {actual} bytes, over the "
        f"{WASM_RAW_CEILING_BYTES} byte ceiling by {actual - WASM_RAW_CEILING_BYTES}.\n"
        "This budget is on RAW bytes, not brotli: raw is what the wasm compiler "
        "reads, and compile time is what dominates the boot gap (#818).\n"
        "If the growth is intended, raise WASM_RAW_CEILING_BYTES in "
        "tools/dev/dev/wasm_audit.py deliberately, and say why in the commit. "
        "If it is not, `dev wasm-audit report --breakdown` attributes the bytes "
        "to crates."
    )


def over(raw: int) -> bool:
    """Whether `raw` bytes exceed the ceiling, which a build may land on."""
    return raw > WASM_RAW_CEILING_BYTES


def budget(root: str) -> None:
    """Fails unless the selected wasm under `root` fits the inclusive ceiling.

    The caller built the bundle; the budget never runs Nix.
    """
    with steps.phase("wasm-budget"):
        # Only the wasm's own size: the compressed copies are no budget input.
        raw = (Path(root) / outputs(Path(root))[0]).stat().st_size
        if over(raw):
            steps.refuse(failure_message(raw))
        # A headroom budget can erode quietly inside the headroom; the drift
        # from the achieved size is what makes that visible before the ceiling.
        drift = raw - WASM_RAW_ACHIEVED_BYTES
        print(
            f"wasm-budget: {raw} raw bytes "
            f"(ceiling {WASM_RAW_CEILING_BYTES}, {drift:+} vs #1549)"
        )


def _leb128(data: bytes, offset: int) -> tuple[int, int]:
    """An unsigned LEB128 at `offset`, and the offset after it."""
    value = shift = 0
    while True:
        if offset >= len(data):
            raise ValueError("truncated LEB128")
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if not byte & 0x80:
            return value, offset


def sections(data: bytes) -> list[tuple[str, int]]:
    """Each section's name and on-disk span (id byte, length prefix and payload).

    Only the headers are read, and a custom section's name. The spans plus the
    8-byte header account for every byte of the file: a section that would run
    past its end is refused rather than under-reported.
    """
    if data[:4] != b"\0asm" or len(data) < HEADER_BYTES:
        raise ValueError("not a wasm module")
    spans = []
    offset = HEADER_BYTES
    while offset < len(data):
        start = offset
        identity = data[offset]
        length, payload = _leb128(data, offset + 1)
        offset = payload + length
        if offset > len(data):
            raise ValueError(f"section at byte {start} runs past the end of the file")
        if identity == 0:
            size, name = _leb128(data, payload)
            label = "custom:" + data[name : name + size].decode(errors="replace")
        else:
            label = SECTIONS.get(identity, f"section-{identity}")
        spans.append((label, offset - start))
    return spans


def _first_segment(text: str) -> str | None:
    """The crate leading a type or path, or None for a primitive or generic."""
    text = text.strip()
    while prefix := next(
        (
            p
            for p in ("&", "*const ", "*mut ", "mut ", "dyn ", "impl ", "(", "[")
            if text.startswith(p)
        ),
        None,
    ):
        text = text[len(prefix) :].lstrip()
    # A path that never had a `::` is a primitive or a generic parameter, not a
    # crate: `u32`, `T`.
    if "::" not in text:
        return None
    # The crate is cut before generic arguments and twiggy's `[hash]`
    # disambiguator, which would otherwise split one crate across buckets.
    segment = re.split(r"::|<|\[", text, maxsplit=1)[0].strip()
    return segment or None


def crate_of(name: str) -> str | None:
    """The crate a demangled Rust path came from, or None.

    `<Self as Trait>::method` has no crate in leading position. They are among
    the commonest symbols, so the self type's crate is used, falling back to the
    trait's when the self type is a primitive (`<u32 as Display>::fmt` is code
    `core` emitted).
    """
    name = name.strip()
    if name.startswith("<"):
        inner = name[1:].split(">::", 1)[0]
        self_type, _, trait = inner.partition(" as ")
        return _first_segment(self_type) or (_first_segment(trait) if trait else None)
    return _first_segment(name)


def rollup(items: list[dict[str, Any]]) -> list[tuple[str, int]]:
    """twiggy's function items' bytes by crate, largest first, then by name.

    Every function's bytes are kept: one whose name names no crate (an unnamed
    `code[N]`, a wasm-bindgen shim) is charged to UNATTRIBUTED.
    """
    totals: Counter[str] = Counter()
    for item in items:
        name = item["name"]
        if not NOT_A_FUNCTION.match(name):
            totals[crate_of(name) or UNATTRIBUTED] += item["shallow_size"]
    return sorted(totals.items(), key=lambda row: (-row[1], row[0]))


def csr_wasm() -> str:
    """Builds `.#csrWasm`, the unstripped pre-wasm-bindgen wasm, as its own step."""
    return steps.nix_build("csr-wasm", ".#csrWasm") + "/lib/csr.wasm"


# The breakdown tables: a name on the left, its bytes and share on the right.
_BREAKDOWN_ALIGN: tuple[Align, ...] = ("left", "right", "right")


def breakdown(wasm: str) -> None:
    """Prints `wasm`'s bytes by section, as shares of the file, then by crate,
    as shares of twiggy's function total."""
    with steps.phase("wasm-sections"):
        try:
            data = Path(wasm).read_bytes()
        except OSError as error:
            steps.refuse(f"reading wasm artifact {wasm}: {error}")
        try:
            spans = sections(data)
        except ValueError as error:
            steps.refuse(f"parsing sections of {wasm}: {error}")
    items = json.loads(steps.output("twiggy", ["twiggy", "top", "-f", "json", wasm]))
    with steps.phase("wasm-crates"):
        crates = rollup(items)
        functions = sum(size for _, size in crates)
        lines = [
            "WASM bundle breakdown",
            f"artifact: {wasm}",
            (
                f"total: {format_bytes(len(data))} — this is the unstripped "
                "pre-wasm-bindgen artifact, NOT the shipped bundle size "
                "(see `dev wasm-audit report` for that)"
            ),
            "",
            table(
                ["section", "bytes", "share of file"],
                [
                    [name, format_bytes(size), _percent(size, len(data))]
                    for name, size in spans
                ],
                _BREAKDOWN_ALIGN,
            ),
            "",
            (
                "crate attribution, share of the functions "
                f"(code + function sections, {format_bytes(functions)})"
            ),
            table(
                ["crate", "bytes", "share of functions"],
                [
                    [name, format_bytes(size), _percent(size, functions)]
                    for name, size in crates
                ],
                _BREAKDOWN_ALIGN,
            ),
        ]
        print("\n".join(lines))
