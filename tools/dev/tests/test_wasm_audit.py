"""`dev wasm-audit`: the bundle's sizes, the raw wasm budget, and the manual
section and crate breakdown (#836, ADR-0106)."""

import json
import shutil
from pathlib import Path
from typing import Any

import pytest
from conftest import Checkout, leb128, module, require, section, write_bundle

from dev import steps, wasm_audit
from dev.wasm_audit import UNATTRIBUTED, WASM_RAW_CEILING_BYTES

ORGIZE = "_ZN6orgize5parse17h0123456789abcdefE"


def refusal(capsys: pytest.CaptureFixture[str], body: Any) -> str:
    """What `body` printed when it refused, as a failed step does."""
    with pytest.raises(steps.StepFailed) as failure:
        body()
    assert (failure.value.code, failure.value.signalled) == (1, False)
    return capsys.readouterr().err


@pytest.mark.parametrize(
    ("count", "shown"),
    [
        (0, "0 B"),
        (512, "512 B"),
        (1024, "1.0 KiB"),
        (1536, "1.5 KiB"),
        (10 * 1024, "10 KiB"),
        (1024**2, "1.0 MiB"),
        (10 * 1024**2, "10 MiB"),
        (1024**3, "1.0 GiB"),
        (2048 * 1024**3, "2048 GiB"),
    ],
)
def test_byte_counts_read_in_their_largest_binary_unit(count: int, shown: str) -> None:
    assert wasm_audit.format_bytes(count) == shown


# Measurement


def test_the_selected_wasm_comes_first_then_each_listed_script(tmp_path: Path) -> None:
    manifest = write_bundle(tmp_path, b"\0asm\x01\0\0\0")
    (tmp_path / "assets/style.css").write_text("a{}")
    manifest["index.html"]["css"] = ["assets/style.css"]
    (tmp_path / ".vite/manifest.json").write_text(json.dumps(manifest))
    assert wasm_audit.measure(tmp_path) == [
        wasm_audit.Artifact("assets/csr_bg-e5f6.wasm", 8, 3, 2),
        wasm_audit.Artifact("assets/index-a1b2.js", 9, 3, 2),
    ]


def edit(manifest: dict[str, Any], change: str) -> object:
    """`manifest` changed one way it can be wrong."""
    wasm, entry = manifest["csr_bg.wasm"], manifest["index.html"]
    match change:
        case "root":
            return []
        case "no file":
            del entry["file"]
        case "entry":
            manifest["index.html"] = "assets/index-a1b2.js"
        case "file type":
            entry["file"] = 7
        case "absolute":
            entry["file"] = "/etc/passwd"
        case "parent":
            entry["assets"] = ["../outside.wasm"]
        case "empty":
            entry["file"] = ""
        case "dot":
            # Present and below the root, but a second spelling of its path.
            entry["file"] = "./assets/index-a1b2.js"
        case "missing":
            entry["assets"] = ["assets/gone.wasm"]
        case "assets":
            entry["assets"] = "assets/csr_bg-e5f6.wasm"
        case "no wasm":
            del manifest["csr_bg.wasm"]
        case "not wasm":
            wasm["file"] = "assets/index-a1b2.js"
    return manifest


@pytest.mark.parametrize(
    ("change", "reason"),
    [
        ("root", "Vite manifest root must be an object"),
        ("no file", "Vite manifest has no output file for 'index.html'"),
        ("entry", "Vite manifest has no output file for 'index.html'"),
        ("file type", "Vite manifest entry 'index.html' must list output paths"),
        ("absolute", "unsafe Vite output path: '/etc/passwd'"),
        ("parent", "unsafe Vite output path: '../outside.wasm'"),
        ("empty", "unsafe Vite output path: ''"),
        ("dot", "unsafe Vite output path: './assets/index-a1b2.js'"),
        ("missing", "Vite output file is missing: assets/gone.wasm"),
        ("assets", "Vite manifest entry 'index.html' field 'assets' must be a list"),
        ("no wasm", "Vite manifest has no output file for 'csr_bg.wasm'"),
        ("not wasm", "Vite csr_bg.wasm mapping must resolve to a WASM output"),
    ],
)
def test_a_manifest_that_does_not_select_safe_present_outputs_is_refused(
    tmp_path: Path, capsys: pytest.CaptureFixture[str], change: str, reason: str
) -> None:
    manifest = edit(write_bundle(tmp_path, b"wasm"), change)
    (tmp_path / ".vite/manifest.json").write_text(json.dumps(manifest))
    assert refusal(capsys, lambda: wasm_audit.measure(tmp_path)) == f"{reason}\n"


def test_an_unreadable_manifest_or_precompressed_copy_is_refused(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    write_bundle(tmp_path, b"wasm")
    (tmp_path / "assets/index-a1b2.js.br").unlink()
    error = refusal(capsys, lambda: wasm_audit.measure(tmp_path))
    assert error.startswith("reading Vite output assets/index-a1b2.js.br: ")
    (tmp_path / ".vite/manifest.json").write_text("{")
    error = refusal(capsys, lambda: wasm_audit.measure(tmp_path))
    assert error.startswith(f"reading Vite manifest {tmp_path}/.vite/manifest.json: ")


# The budget


def test_the_ceiling_is_inclusive_and_leaves_headroom_over_the_achieved_size() -> None:
    assert WASM_RAW_CEILING_BYTES == 5_000_000
    assert not wasm_audit.over(WASM_RAW_CEILING_BYTES)
    assert wasm_audit.over(WASM_RAW_CEILING_BYTES + 1)
    # A ceiling on the achieved size is a strict ratchet, which #836 rejected:
    # every innocent dependency bump would turn the gate red.
    assert wasm_audit.WASM_RAW_ACHIEVED_BYTES < WASM_RAW_CEILING_BYTES


def test_historical_sizes_fall_where_adr_0106_records_them() -> None:
    # #1750's amendment accepts that the weaker `-Os` and `-O2` builds of
    # #1625 fit; the bundle before #836's `wasm-opt` still fails.
    assert not wasm_audit.over(4_191_207)
    assert not wasm_audit.over(4_252_496)
    assert wasm_audit.over(5_350_591)


def test_a_passing_budget_reports_its_drift_from_the_achieved_size(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    for size, drift in [(4_009_600, "-56"), (WASM_RAW_CEILING_BYTES, "+990344")]:
        write_bundle(tmp_path, b"\0" * size)
        wasm_audit.budget(str(tmp_path))
        assert capsys.readouterr().out == (
            f"wasm-budget: {size} raw bytes (ceiling 5000000, {drift} vs #1549)\n"
        )


def test_the_budget_reads_only_the_wasm_not_its_compressed_copies(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    write_bundle(tmp_path, b"wasm")
    (tmp_path / "assets/csr_bg-e5f6.wasm.br").unlink()
    wasm_audit.budget(str(tmp_path))
    assert capsys.readouterr().out.startswith("wasm-budget: 4 raw bytes ")


def test_an_overrun_names_the_constant_to_raise_and_how_to_attribute_it(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    write_bundle(tmp_path, b"\0" * (WASM_RAW_CEILING_BYTES + 3))
    error = refusal(capsys, lambda: wasm_audit.budget(str(tmp_path)))
    assert error.startswith(
        "raw manifest-selected WASM identity artifact is 5000003 bytes, "
        "over the 5000000 byte ceiling by 3.\n"
        "This budget is on RAW bytes, not brotli"
    )
    assert "raise WASM_RAW_CEILING_BYTES in tools/dev/dev/wasm_audit.py" in error
    assert "`dev wasm-audit report --breakdown`" in error


# The section walk


def test_section_spans_and_the_header_account_for_the_whole_file() -> None:
    wasm = module([ORGIZE]) + section(0, leb128(9) + b"producers" + b"x")
    wasm += section(42, b"")
    spans = wasm_audit.sections(wasm)
    assert [name for name, _ in spans] == [
        "type",
        "function",
        "code",
        "custom:name",
        "custom:producers",
        "section-42",
    ]
    assert wasm_audit.HEADER_BYTES + sum(size for _, size in spans) == len(wasm)


@pytest.mark.parametrize(
    ("wasm", "reason"),
    [
        (b"not a wasm file at all", "not a wasm module"),
        (b"\0asm\x01\0", "not a wasm module"),
        (module()[:-3], "runs past the end of the file"),
        (b"\0asm\x01\0\0\0\x01\x80", "truncated LEB128"),
    ],
)
def test_a_file_the_walk_cannot_account_for_is_refused(
    wasm: bytes, reason: str
) -> None:
    with pytest.raises(ValueError, match=reason):
        wasm_audit.sections(wasm)


# The crate rollup


@pytest.mark.parametrize(
    ("name", "crate"),
    [
        ("orgize::parse::inner", "orgize"),
        ("croner[3c1c0]::parse", "croner"),
        ("<reactive_graph::Signal as core::fmt::Debug>::fmt", "reactive_graph"),
        ("<&orgize::Ast as core::clone::Clone>::clone", "orgize"),
        ("<&mut *const dyn impl ([web[ab]::X as core::Y>::z", "web"),
        ("<alloc::vec::Vec<T> as core::ops::Drop>::drop", "alloc"),
        # `<u32 as Display>::fmt` is code `core` emitted; `u32` is no crate.
        ("<u32 as core::fmt::Display>::fmt", "core"),
        ("<tachys::View>::build", "tachys"),
        ("<T>::build", None),
        ("T", None),
        ("u32", None),
        ("", None),
        ("::orphan", None),
        ("__wbindgen_malloc", None),
    ],
)
def test_a_function_belongs_to_its_paths_leading_crate(
    name: str, crate: str | None
) -> None:
    assert wasm_audit.crate_of(name) == crate


def item(name: str, size: int) -> dict[str, Any]:
    return {"name": name, "shallow_size": size, "shallow_size_percent": 0.0}


NOT_FUNCTIONS = [
    '"function names" subsection',
    'data segment ".rodata"',
    "custom section '__wasm_bindgen_unstable'",
    'export "main"',
    "import env::f",
    "type[0]: () -> i32",
    "elem[0]",
    "global[1]",
    "table[0]",
    "memory[0]",
    "wasm magic bytes",
    "code section headers",
]


def test_functions_roll_up_by_crate_conserving_bytes_and_skipping_the_rest() -> None:
    items = [
        *(item(name, 1000) for name in NOT_FUNCTIONS),
        item("orgize::parse::h0123", 100),
        item("<orgize::Ast as core::Clone>::clone", 50),
        item("core::fmt::write", 30),
        item("alloc::fmt", 30),
        item("code[3]", 60),
        item("__wbindgen_malloc", 40),
    ]
    assert wasm_audit.rollup(items) == [
        ("orgize", 150),
        (UNATTRIBUTED, 100),
        ("alloc", 30),
        ("core", 30),
    ]
    assert wasm_audit.rollup([]) == []


# The command line


def test_report_measures_a_given_bundle_root(checkout: Checkout) -> None:
    root = checkout.root.parent / "bundle"
    write_bundle(root, b"\0" * 2 * 1024**2)
    result = checkout.dev("wasm-audit", "report", "--site-path", str(root))
    assert result.returncode == 0, result
    assert result.stdout.decode() == (
        "WASM bundle audit\n"
        f"bundle output: {root}\n"
        "artifact          raw        gzip       brotli\n"
        "assets/csr_bg-e5f6.wasm    2.0 MiB        3 B        2 B\n"
        "assets/index-a1b2.js        9 B        3 B        2 B\n"
    )


# A `nix` whose builds link and print one fixture output.
LINKING_NIX = """\
output = os.environ["FIXTURE_OUTPUT"]
if "--derivation" in sys.argv:
    print("/nix/store/fake.drv")
elif sys.argv[1] == "path-info":
    sys.exit(1)
else:
    if "--out-link" in sys.argv:
        link = pathlib.Path(sys.argv[sys.argv.index("--out-link") + 1])
        link.parent.mkdir(parents=True, exist_ok=True)
        link.symlink_to(output)
    print(output)
"""


def test_report_builds_the_shipped_bundle_without_a_root(checkout: Checkout) -> None:
    root = checkout.root.parent / "bundle"
    write_bundle(root, b"wasm")
    checkout.tool("nix", LINKING_NIX)
    result = checkout.dev("wasm-audit", "report", FIXTURE_OUTPUT=str(root))
    assert result.returncode == 0, result
    assert f"bundle output: {root}\n".encode() in result.stdout
    assert "nix build -L --accept-flake-config --print-out-paths --out-link " in (
        checkout.order()
    )
    assert ".#csrBundle" in checkout.order()


def twiggy(checkout: Checkout) -> None:
    present = shutil.which("twiggy")
    require(present is not None, "twiggy")
    assert present is not None
    (checkout.bin / "twiggy").symlink_to(present)


def row(name: str, size: str, share: str) -> str:
    return f"{name:<18}  {size:>9}  {share:>12}\n"


def test_breakdown_attributes_a_named_module_with_twiggy(checkout: Checkout) -> None:
    twiggy(checkout)
    wasm = checkout.root.parent / "csr.wasm"
    wasm.write_bytes(module([ORGIZE, "__wbindgen_malloc"], functions=3))
    result = checkout.dev("wasm-audit", "report", "--breakdown", "--wasm", str(wasm))
    assert result.returncode == 0, result
    # twiggy charges each function its 5-byte body and 1-byte declaration.
    assert result.stdout.decode() == (
        "WASM bundle breakdown\n"
        f"artifact: {wasm}\n"
        "total: 106 B — this is the unstripped pre-wasm-bindgen artifact, "
        "NOT the shipped bundle size (see `dev wasm-audit report` for that)\n"
        "\n"
        "section               bytes     share of file\n"
        f"{row('type', '7 B', '6.6%')}"
        f"{row('function', '6 B', '5.7%')}"
        f"{row('code', '18 B', '17.0%')}"
        f"{row('custom:name', '67 B', '63.2%')}"
        "\n"
        "crate attribution, share of the functions "
        "(code + function sections, 18 B)\n"
        "crate                 bytes     share of functions\n"
        f"{row(UNATTRIBUTED, '12 B', '66.7%')}"
        f"{row('orgize', '6 B', '33.3%')}"
    )


def test_breakdown_builds_the_unstripped_wasm_without_a_file(
    checkout: Checkout,
) -> None:
    twiggy(checkout)
    output = checkout.root.parent / "csr-wasm"
    (output / "lib").mkdir(parents=True)
    (output / "lib/csr.wasm").write_bytes(module())
    checkout.tool("nix", LINKING_NIX)
    result = checkout.dev(
        "wasm-audit", "report", "--breakdown", FIXTURE_OUTPUT=str(output)
    )
    assert result.returncode == 0, result
    assert f"artifact: {output}/lib/csr.wasm\n".encode() in result.stdout
    assert ".#csrWasm" in checkout.order()


@pytest.mark.parametrize(
    ("content", "reason"),
    [(None, "reading wasm artifact "), (b"not wasm", "parsing sections of ")],
)
def test_breakdown_refuses_a_file_it_cannot_walk(
    checkout: Checkout, content: bytes | None, reason: str
) -> None:
    wasm = checkout.root.parent / "csr.wasm"
    if content is not None:
        wasm.write_bytes(content)
    result = checkout.dev("wasm-audit", "report", "--breakdown", "--wasm", str(wasm))
    assert result.returncode == 1, result
    assert result.stderr.decode().startswith(f"{reason}")
    assert str(wasm) in result.stderr.decode()


@pytest.mark.parametrize(
    ("args", "reason"),
    [
        (["--wasm", "x.wasm"], "--wasm requires --breakdown"),
        (
            ["--breakdown", "--site-path", "/nix/store/x"],
            "--site-path is the shipped bundle; --breakdown is not",
        ),
    ],
)
def test_contradictory_report_options_are_refused_before_any_work(
    checkout: Checkout, args: list[str], reason: str
) -> None:
    result = checkout.dev("wasm-audit", "report", *args)
    assert result.returncode == 2, result
    assert reason in result.stderr.decode()
    assert checkout.runs() == []
