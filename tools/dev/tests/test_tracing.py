"""Run directories, the merged Chrome trace, and tracing failures."""

import json
import subprocess
from pathlib import Path
from typing import Any

from conftest import Checkout

from jaunder_dev import chrome

KILL_PARENT = "os.kill(os.getppid(), signal.SIGKILL); sys.exit()"


def slices(trace_json: Path) -> list[dict[str, Any]]:
    events = json.loads(trace_json.read_text())["traceEvents"]
    return [event for event in events if event["ph"] == "X"]


def records(run: Path) -> list[dict[str, Any]]:
    return [
        json.loads(line)
        for path in sorted(run.glob("*.jsonl"))
        for line in path.read_text().splitlines()
    ]


def test_records_survive_a_sigkilled_writer(checkout: Checkout) -> None:
    checkout.tool("tsc", KILL_PARENT)
    result = checkout.dev("static", "tsc", "--sandbox")
    assert result.returncode == -9

    (run,) = checkout.runs()
    written = records(run)
    assert [(record["kind"], record["name"]) for record in written] == [
        ("start", "dev static tsc"),
        ("start", "tsc"),
    ]
    assert written[1]["parent_id"] == written[0]["span_id"]
    assert not run.with_name(run.name.replace(".d", ".json")).exists()

    summary = checkout.dev("trace", "summary", str(run))
    assert summary.returncode == 0, summary
    lines = summary.stdout.decode().splitlines()
    assert [line.endswith(("dev static tsc", "  tsc")) for line in lines[1:]] == [
        True,
        True,
    ]
    assert all("incomplete" in line for line in lines[1:])


def test_child_dev_joins_the_run_under_its_step_and_a_killed_child_is_incomplete(
    checkout: Checkout,
) -> None:
    # The outer `tsc` launches a nested `dev static tsc`, whose own `tsc` kills it.
    checkout.tool(
        "tsc",
        'if os.environ.get("INNER"):\n'
        f"    {KILL_PARENT}\n"
        'os.environ["INNER"] = "1"\n'
        'nested = subprocess.run(eval(os.environ["DEV"]) + ["static", "tsc", "--sandbox"])\n'
        "sys.exit(128 - nested.returncode)",
    )
    result = checkout.dev("static", "tsc", "--sandbox", DEV=repr(checkout.command()))
    assert result.returncode == 137, result

    (run,) = checkout.runs()
    events = slices(run.with_name(run.name.replace(".d", ".json")))
    outer_gate, outer_step, inner_gate, inner_step = events
    assert [event["name"] for event in events] == [
        "dev static tsc",
        "tsc",
        "dev static tsc",
        "tsc",
    ]
    assert {event["args"]["trace_id"] for event in events} == {
        outer_gate["args"]["trace_id"]
    }
    assert outer_step["args"]["parent_id"] == outer_gate["args"]["span_id"]
    assert inner_gate["args"]["parent_id"] == outer_step["args"]["span_id"]
    assert inner_step["args"]["parent_id"] == inner_gate["args"]["span_id"]
    assert outer_gate["args"]["jaunder.status"] == "failed"
    assert outer_step["args"]["process.exit.code"] == 137
    assert inner_gate["args"]["jaunder.status"] == "incomplete"
    assert inner_step["args"]["jaunder.status"] == "incomplete"
    # The unfinished spans still end inside the run, so they nest on one track.
    assert {event["tid"] for event in events} == {0}


def write_records(run: Path, *lines: dict[str, Any]) -> None:
    run.mkdir(parents=True)
    text = "".join(json.dumps(line) + "\n" for line in lines)
    (run / "1-a.jsonl").write_text(text + '{"kind":"start","trunc')


def start(span: str, parent: str | None, at: int, name: str = "") -> dict[str, Any]:
    return {
        "kind": "start",
        "trace_id": "t",
        "span_id": span,
        "parent_id": parent,
        "name": name or span,
        "time_ns": at,
    }


def end(span: str, parent: str | None, at: int, **attributes: Any) -> dict[str, Any]:
    return {**start(span, parent, at), "kind": "end", "attributes": attributes}


def test_merge_marks_a_span_whose_parent_left_no_record_incomplete(
    tmp_path: Path,
) -> None:
    run = tmp_path / "run.trace.d"
    write_records(
        run,
        start("root", None, 1_000),
        start("orphan", "lost", 2_000),
        end("orphan", "lost", 3_000, **{"jaunder.status": "ok"}),
        end("root", None, 9_000, **{"jaunder.status": "ok"}),
    )
    chrome.write(run, tmp_path / "run.trace.json")
    root, orphan = slices(tmp_path / "run.trace.json")
    assert root["args"]["jaunder.status"] == "ok"
    assert orphan["args"]["jaunder.status"] == "incomplete"
    assert orphan["args"]["parent_id"] == "lost"
    assert (orphan["ts"], orphan["dur"]) == (2, 1)


def test_merge_gives_overlapping_siblings_separate_tracks(tmp_path: Path) -> None:
    run = tmp_path / "run.trace.d"
    write_records(
        run,
        start("root", None, 0),
        start("a", "root", 1_000),
        start("b", "root", 3_000),
        start("a1", "a", 1_500),
        end("a1", "a", 2_000),
        end("a", "root", 5_000),
        end("b", "root", 7_000),
        start("c", "root", 8_000),
        end("c", "root", 9_000),
        end("root", None, 10_000),
    )
    chrome.write(run, tmp_path / "run.trace.json")
    track = {
        event["name"]: event["tid"] for event in slices(tmp_path / "run.trace.json")
    }
    assert track == {"root": 0, "a": 0, "a1": 0, "b": 1, "c": 0}
    names = [
        event["args"]["name"]
        for event in json.loads((tmp_path / "run.trace.json").read_text())[
            "traceEvents"
        ]
        if event["ph"] == "M"
    ]
    assert names == ["root", "b"]


def test_summary_does_not_sum_overlapping_children(
    checkout: Checkout, tmp_path: Path
) -> None:
    run = tmp_path / "run.trace.d"
    second = 1_000_000_000
    write_records(
        run,
        start("gate", None, 0, name="dev check"),
        start("a", "gate", 1 * second, name="nix build"),
        start("b", "gate", 3 * second, name="cargo test"),
        end(
            "a",
            "gate",
            5 * second,
            **{"jaunder.status": "ok", "jaunder.cache": "reused"},
        ),
        end("b", "gate", 7 * second, **{"jaunder.status": "failed"}),
        end("gate", None, 10 * second, **{"jaunder.status": "failed"}),
    )
    text = checkout.dev("trace", "summary", str(run))
    assert text.returncode == 0, text
    assert text.stdout.decode() == (
        "Duration  Untraced  Depth  Status  Cache   Span\n"
        " 10.000s    4.000s      0  failed          dev check\n"
        "  4.000s                1  ok      reused  nix build\n"
        "  4.000s                1  failed          cargo test\n"
    )
    markdown = checkout.dev("trace", "summary", "--markdown", str(run))
    assert markdown.stdout.decode().splitlines()[2] == (
        "| 10.000s | 4.000s | 0 | failed |  | dev check |"
    )


def test_summary_lists_every_unsuccessful_span_beyond_the_slowest(
    checkout: Checkout, tmp_path: Path
) -> None:
    # Twenty-five slow `ok` steps, and short steps that ended badly or carry no
    # status, all shorter than every slow one.
    slow = [f"slow{index}" for index in range(25)]
    short = {"failed": "failed", "signalled": "signalled", "unknown": None}
    records: list[dict[str, Any]] = [start("gate", None, 0)]
    for index, span in enumerate(slow):
        records.append(start(span, "gate", 0))
        records.append(end(span, "gate", 1_000 + index, **{"jaunder.status": "ok"}))
    for duration, (span, status) in zip([10, 9, 8], short.items()):
        records.append(start(span, "gate", 0))
        attributes = {} if status is None else {"jaunder.status": status}
        records.append(end(span, "gate", duration, **attributes))
    # No end record: it is incomplete, ending with its parent.
    records.append(start("killed", "gate", 9_995))
    records.append(end("gate", None, 10_000, **{"jaunder.status": "failed"}))
    write_records(tmp_path / "run.trace.d", *records)

    result = checkout.dev("trace", "summary", str(tmp_path / "run.trace.d"))
    assert result.returncode == 0, result
    names = [line.rsplit(" ", 1)[-1] for line in result.stdout.decode().splitlines()]
    # The slowest twenty, then the unsuccessful spans, slowest first.
    assert names == [
        "Span",
        "gate",
        *reversed(slow[-19:]),
        "failed",
        "signalled",
        "killed",
    ]


def test_summary_defaults_to_the_latest_run(checkout: Checkout) -> None:
    missing = checkout.dev("trace", "summary")
    assert missing.returncode == 1
    assert b"no traced runs" in missing.stderr

    runs = checkout.root / ".xtask" / "run"
    write_records(runs / "1-1.trace.d", start("old", None, 0), end("old", None, 1))
    write_records(runs / "2-1.trace.d", start("new", None, 0), end("new", None, 1))
    latest = checkout.dev("trace", "summary", cwd=checkout.root / "sub")
    assert latest.returncode == 0, latest
    assert latest.stdout.decode().splitlines()[1].endswith("new")


def outcome(result: subprocess.CompletedProcess[bytes]) -> tuple[int, bytes, bytes]:
    return result.returncode, result.stdout, result.stderr


def test_unwritable_trace_directory_leaves_output_and_status_unchanged(
    checkout: Checkout,
) -> None:
    checkout.tool("tsc", 'print("out"); print("err", file=sys.stderr); sys.exit(5)')
    (checkout.root / ".xtask").write_text("not a directory")
    unwritable = outcome(checkout.dev("static", "tsc", "--sandbox"))
    missing = str(checkout.root / "missing")
    child = outcome(
        checkout.dev("static", "tsc", "--sandbox", JAUNDER_TRACE_DIR=missing)
    )
    (checkout.root / ".xtask").unlink()
    traced = outcome(checkout.dev("static", "tsc", "--sandbox"))
    assert traced == (5, b"out\n", b"err\n")

    for code, stdout, stderr in (unwritable, child):
        assert (code, stdout) == traced[:2]
        warning, rest = stderr.split(b"\n", 1)
        assert warning.startswith(b"dev: tracing disabled: ")
        assert rest == traced[2]


# Lines a run directory can hold that are not span records: other JSON, a record
# missing fields or with a wrongly typed one, bytes that are not UTF-8, and a
# multibyte character cut by a killed writer, each between valid neighbours.
MALFORMED = (
    b"1\n"
    b'"text"\n'
    b'{"kind":"start"}\n'
    b'{"kind":"end","trace_id":"t","span_id":"x","parent_id":null,"name":"x",'
    b'"time_ns":"late","attributes":{}}\n'
    b"\xff\xfe\n"
    b'{"kind":"start","trace_id":"t","span_id":"y","parent_id":null,"name":"\xc3\n'
    # Two spans, finished and not, each naming the other as its parent.
    b'{"kind":"start","trace_id":"t","span_id":"a","parent_id":"b","name":"a",'
    b'"time_ns":1}\n'
    b'{"kind":"start","trace_id":"t","span_id":"b","parent_id":"a","name":"b",'
    b'"time_ns":2}\n'
    b'{"kind":"end","trace_id":"t","span_id":"b","parent_id":"a","name":"b",'
    b'"time_ns":3,"attributes":{}}\n'
)
# A step body that fails visibly after running `action` on the run directory.
FAIL_AFTER = (
    'run = pathlib.Path(os.environ["JAUNDER_TRACE_DIR"])\n'
    "{action}\n"
    'print("out"); print("err", file=sys.stderr); sys.exit(3)'
)


def test_malformed_records_change_neither_the_merge_nor_the_outcome(
    checkout: Checkout,
) -> None:
    action = f'(run / "9-x.jsonl").write_bytes({MALFORMED!r})'
    checkout.tool("tsc", FAIL_AFTER.format(action=action))
    result = checkout.dev("static", "tsc", "--sandbox")
    assert outcome(result) == (3, b"out\n", b"err\n")
    slices = checkout.merged()
    assert set(slices) == {"dev static tsc", "tsc"}
    assert slices["tsc"]["args"]["jaunder.status"] == "failed"

    (run,) = checkout.runs()
    summary = checkout.dev("trace", "summary", str(run))
    assert summary.returncode == 0, summary
    assert summary.stdout.decode().splitlines()[1].endswith("dev static tsc")


def test_a_failed_merge_warns_once_and_changes_nothing_else(
    checkout: Checkout,
) -> None:
    # The merged file's path is taken by a directory.
    action = 'run.with_name(run.name.removesuffix(".d") + ".json").mkdir()'
    checkout.tool("tsc", FAIL_AFTER.format(action=action))
    code, stdout, stderr = outcome(checkout.dev("static", "tsc", "--sandbox"))
    assert (code, stdout) == (3, b"out\n")
    rest, warning = stderr.split(b"\n", 1)
    assert rest == b"err"
    assert warning.startswith(b"dev: tracing disabled: ")
    assert warning.count(b"\n") == 1


def test_summary_of_an_unreadable_run_reports_without_a_traceback(
    tmp_path: Path, checkout: Checkout
) -> None:
    run = tmp_path / "run.trace.d"
    write_records(run, start("root", None, 0))
    (run / "1-a.jsonl").chmod(0)
    summary = checkout.dev("trace", "summary", str(run))
    assert summary.returncode == 1
    assert summary.stderr.startswith(b"Error: cannot summarise ")
    assert b"Traceback" not in summary.stderr


def test_a_run_whose_root_disabled_tracing_stays_silent(checkout: Checkout) -> None:
    checkout.tool(
        "tsc",
        'print(repr(os.environ["JAUNDER_TRACE_DIR"]), "TRACEPARENT" in os.environ)',
    )
    result = checkout.dev(
        "static", "tsc", "--sandbox", JAUNDER_TRACE_DIR="", TRACEPARENT="00-x"
    )
    assert (result.returncode, result.stdout, result.stderr) == (0, b"'' False\n", b"")
    assert checkout.runs() == []


def test_command_text_is_capped_at_one_kibibyte(checkout: Checkout) -> None:
    checkout.tool("setup-e2e-node-modules", "")
    checkout.tool("tsc", "")
    long_pin = "é" * 600
    result = checkout.dev("static", "tsc", E2E_TYPES_NODE_MODULES=long_pin)
    assert result.returncode == 0, result
    text = checkout.merged()["setup-e2e-node-modules"]["args"]["process.command_line"]
    assert len(text.encode()) <= 1024
    assert text.startswith("setup-e2e-node-modules '" + "é" * 400)


def test_an_outside_traceparent_does_not_parent_the_root(checkout: Checkout) -> None:
    checkout.tool("tsc", "")
    outside = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
    result = checkout.dev("static", "tsc", "--sandbox", TRACEPARENT=outside)
    assert result.returncode == 0, result
    gate = checkout.merged()["dev static tsc"]["args"]
    assert gate["parent_id"] is None
    assert gate["trace_id"] != "0af7651916cd43dd8448eb211c80319c"
    assert gate["jaunder.status"] == "ok"
