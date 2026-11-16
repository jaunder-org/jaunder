"""`dev test-local`: host-proof as a traced step, its report rendered."""

import json
from pathlib import Path

import pytest
from conftest import Checkout

from jaunder_dev import report

# Reports and their rendering by `jq -r -f scripts/step-report.jq`, captured
# from the real jq once: the rendering must stay byte-identical.
REPORT = {
    "command": "test-local",
    "ok": False,
    "duration_ms": 2042,
    "steps": [
        {"name": "fast", "ok": True, "skipped": False, "duration_ms": 999},
        {
            "name": "slow",
            "ok": True,
            "skipped": False,
            "duration_ms": 1000,
            "detail": "verified CSR bundle /nix/store/x",
        },
        {"name": "blank", "ok": True, "skipped": False, "duration_ms": 3, "detail": ""},
        {"name": "skipped", "ok": False, "skipped": True, "duration_ms": 0},
        {
            "name": "broken",
            "ok": False,
            "skipped": False,
            "duration_ms": 12,
            "detail": "line one\nline two — ü",
        },
    ],
    "report": "table\n  row",
}
RENDERED = (
    "table\n"
    "  row\n"
    "[ ok ] fast\n"
    "[ ok ] slow (1000 ms) — verified CSR bundle /nix/store/x\n"
    "[ ok ] blank — \n"
    "[skip] skipped (0 ms)\n"
    "[FAIL] broken (12 ms) — line one\n"
    "line two — ü\n"
)
PLAIN = {
    "command": "test-local",
    "ok": True,
    "duration_ms": 5,
    "steps": [{"name": "test-local", "ok": True, "skipped": False, "duration_ms": 5}],
}

HOST_PROOF = [
    "run",
    "--locked",
    "--quiet",
    "--manifest-path",
    "tools/Cargo.toml",
    "-p",
    "host-proof",
    "--bin",
    "test-local",
    "--",
]

# A stand-in for `cargo run … test-local`: it joins the trace with one span,
# records what it was given, and prints a report.
FAKE_HOST_PROOF = """\
import json
pathlib.Path("argv.json").write_text(json.dumps(sys.argv[1:]))
pathlib.Path("env.json").write_text(json.dumps({
    key: os.environ.get(key)
    for key in ("RUSTC_WRAPPER", "CARGO_INCREMENTAL", "SCCACHE_BASEDIRS")
}))
_, trace_id, parent_id, _ = os.environ["TRACEPARENT"].split("-")
record = {"trace_id": trace_id, "span_id": "2222222222222222",
          "parent_id": parent_id, "name": "xtask.command"}
start = time.time_ns()
with open(pathlib.Path(os.environ["JAUNDER_TRACE_DIR"], f"{os.getpid()}-f.jsonl"), "a") as out:
    out.write(json.dumps({"kind": "start", **record, "time_ns": start}) + "\\n")
    out.write(json.dumps({"kind": "end", **record, "time_ns": start + 1000,
                          "attributes": {"jaunder.status": "ok"}}) + "\\n")
print(os.environ["REPORT"])
print("host-proof progress", file=sys.stderr)
sys.exit(int(os.environ["HOST_PROOF_CODE"]))
"""


@pytest.mark.parametrize(
    ("document", "rendered"),
    [(REPORT, RENDERED), (PLAIN, "[ ok ] test-local\n")],
)
def test_report_renders_as_step_report_jq_does(
    document: dict[str, object], rendered: str
) -> None:
    assert report.render(json.dumps(document).encode()) == rendered


@pytest.mark.parametrize(
    ("output", "shown"), [(b"", ""), (b"not a report\n", "not a report\n")]
)
def test_output_that_is_not_a_report_passes_through(output: bytes, shown: str) -> None:
    assert report.render(output) == shown


@pytest.mark.parametrize("code", [0, 1, 2])
def test_test_local_runs_host_proof_as_a_traced_step(
    checkout: Checkout, code: int
) -> None:
    checkout.tool("cargo", FAKE_HOST_PROOF)
    args = ["-p", "storage", "a b", "$(touch pwned)", "--", "--nocapture"]

    result = checkout.dev(
        "test-local",
        "--",
        *args,
        cwd=checkout.root / "sub",
        REPORT=json.dumps(REPORT),
        HOST_PROOF_CODE=str(code),
    )

    assert result.returncode == code, result
    assert result.stdout.decode() == RENDERED
    assert result.stderr == b"host-proof progress\n"
    assert json.loads((checkout.root / "argv.json").read_text()) == [
        *HOST_PROOF,
        *args,
    ]
    environment = json.loads((checkout.root / "env.json").read_text())
    assert environment["RUSTC_WRAPPER"] == "sccache"
    assert environment["CARGO_INCREMENTAL"] == "0"
    assert environment["SCCACHE_BASEDIRS"] == str(checkout.root.resolve())
    assert not (checkout.root / "pwned").exists()

    slices = checkout.merged()
    gate = slices["dev test-local"]
    step = slices["test-local"]
    host_proof = slices["xtask.command"]
    assert step["args"]["parent_id"] == gate["args"]["span_id"]
    assert host_proof["args"]["parent_id"] == step["args"]["span_id"]
    assert host_proof["args"]["trace_id"] == gate["args"]["trace_id"]
    assert step["args"]["process.exit.code"] == code
    assert gate["args"]["jaunder.status"] == ("ok" if code == 0 else "failed")


def test_test_local_without_selectors_passes_none(checkout: Checkout) -> None:
    checkout.tool("cargo", FAKE_HOST_PROOF)
    result = checkout.dev("test-local", REPORT=json.dumps(PLAIN), HOST_PROOF_CODE="0")
    assert result.returncode == 0, result
    assert result.stdout == b"[ ok ] test-local\n"
    assert json.loads((checkout.root / "argv.json").read_text()) == HOST_PROOF


def test_test_local_refuses_options_before_the_separator(checkout: Checkout) -> None:
    checkout.tool("cargo", FAKE_HOST_PROOF)
    result = checkout.dev("test-local", "-p", "storage", HOST_PROOF_CODE="0")
    assert result.returncode == 2
    assert not Path(checkout.root / "argv.json").exists()
    assert checkout.runs() == []
