"""The span record format shared with host-proof's Rust writer."""

from pathlib import Path

import pytest
from opentelemetry.sdk.trace.id_generator import IdGenerator

from dev import trace

GOLDEN = Path(__file__).parent / "golden" / "span-records.jsonl"


class FixedIds(IdGenerator):
    def __init__(self) -> None:
        self._spans = iter([0x00F067AA0BA902B7, 0x1111111111111111])

    def generate_trace_id(self) -> int:
        return 0x0AF7651916CD43DD8448EB211C80319C

    def generate_span_id(self) -> int:
        return next(self._spans)


def test_writer_emits_the_golden_records(tmp_path: Path) -> None:
    writer = trace.RecordWriter(tmp_path, trace.Warner())
    provider = trace.recording_provider(writer, id_generator=FixedIds())
    tracer = provider.get_tracer("test")
    with tracer.start_as_current_span(
        "dev static tsc", start_time=1000, end_on_exit=False
    ) as root:
        child = tracer.start_span("tsc", start_time=2000)
        child.set_attribute(
            "process.command_line", "tsc --noEmit -p end2end/tsconfig.json"
        )
        child.set_attribute("process.exit.code", 2)
        child.set_attribute("jaunder.status", "failed")
        child.end(end_time=8000)
        root.set_attribute("process.exit.code", 2)
        root.set_attribute("jaunder.status", "failed")
        root.end(end_time=9000)
    provider.shutdown()

    (written,) = tmp_path.glob("*.jsonl")
    assert written.read_text(encoding="utf-8") == GOLDEN.read_text(encoding="utf-8")


def test_only_the_first_tracing_failure_is_reported(
    capsys: pytest.CaptureFixture[str],
) -> None:
    warn = trace.Warner()
    warn("first")
    warn("second")
    assert capsys.readouterr().err == "dev: tracing disabled: first\n"
