"""The e2e verdicts over lifted lane evidence: lane population, the duration
budget, boot decomposition coverage, ownership and pair reconciliation."""

import json
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest
from conftest import FIXTURES, lift_lanes, write_capture, write_lane_capture

from dev import e2e
from dev.e2e_evidence import (
    LaneEvidence,
    reconcile,
    reconcile_ownership,
    reconciled_lanes,
    validate_boot_files,
    validate_boot_json,
    validate_duration_files,
    validate_duration_json,
    validate_population,
    verify_lane,
)
from dev.evidence import EvidenceError, ExpectedLane, read_catalog
from dev.traces import TRACE_MEMBER

CATALOG = FIXTURES / "e2e-lanes.json"
OWNERSHIP = FIXTURES / "e2e-ownership"
EVIDENCE = FIXTURES / "e2e-evidence"
IDENTITIES = [f"sqlite-firefox-{index}-of-5" for index in range(1, 6)]
CENSUS = (OWNERSHIP / "expected-census.json").read_text()


def shard(directory: Path, index: int, kind: str) -> str:
    suffix = "jsonl" if kind == "trace" else "json"
    return (directory / f"shard-{index}-{kind}.{suffix}").read_text()


def fixture(root: Path) -> list[LaneEvidence]:
    """The five SQLite/Firefox lanes' passing evidence, lifted under `root`."""
    return lift_lanes(root)


def errors(evidence: list[LaneEvidence]) -> str:
    with pytest.raises(EvidenceError) as refused:
        reconcile("sqlite", "firefox", read_catalog(CATALOG), evidence)
    return str(refused.value)


def edit(path: Path, change: Any) -> None:
    document = json.loads(path.read_text())
    change(document)
    path.write_text(json.dumps(document))


# Reconciliation of a pair.


def test_the_complete_five_lane_fixture_reconciles(tmp_path: Path) -> None:
    detail = reconcile("sqlite", "firefox", read_catalog(CATALOG), fixture(tmp_path))
    assert detail == "sqlite/firefox: reconciled 5 lane-qualified evidence sets"


def slow(lane: LaneEvidence) -> None:
    """Makes `lane`'s one passing attempt take 90% of its timeout."""
    edit(
        lane.report,
        lambda report: report["suites"][0]["specs"][0]["tests"][0]["results"][0].update(
            duration=90
        ),
    )


def test_reconciliation_names_pressure_its_producers_reported(tmp_path: Path) -> None:
    evidence = fixture(tmp_path)
    slow(evidence[4])
    detail = reconcile("sqlite", "firefox", read_catalog(CATALOG), evidence)
    assert detail == (
        "sqlite/firefox: reconciled 5 lane-qualified evidence sets; 1 attempt(s) "
        "under duration pressure, reported by their lane producers"
    )


def test_lifted_lanes_reconcile_only_for_their_own_browser(tmp_path: Path) -> None:
    lifted = tmp_path / "lifted"
    fixture(lifted)
    verdict, lanes = reconciled_lanes("sqlite", "firefox", CATALOG, lifted)
    assert "5 lane-qualified" in verdict
    assert [lane.identity for lane in lanes] == IDENTITIES
    assert all(lane.capture.exists() for lane in lanes)
    # The same root holds no Chromium lanes, so that pair must not reconcile.
    with pytest.raises(EvidenceError, match="reading expected census"):
        reconciled_lanes("sqlite", "chromium", CATALOG, lifted)


def test_missing_duplicate_and_malformed_lane_evidence_fail_closed(
    tmp_path: Path,
) -> None:
    evidence = fixture(tmp_path / "missing")
    assert "exactly match" in errors(evidence[:-1])
    evidence = fixture(tmp_path / "duplicate")
    assert "duplicate lane" in errors([*evidence, evidence[0]])
    evidence = fixture(tmp_path / "malformed")
    evidence[0].report.write_text("{")
    assert "ownership census: malformed report" in errors(evidence)


def test_every_failure_is_collected_in_order(tmp_path: Path) -> None:
    evidence = fixture(tmp_path)
    evidence[1].report.unlink()
    evidence[2].phase.unlink()
    reported = errors([*evidence, replace(evidence[0], identity="unknown-lane")])
    lines = reported.splitlines()
    assert lines[0].startswith("reading Playwright report ")
    assert lines[1] == "lane evidence does not exactly match the catalog"
    assert lines[2].startswith(
        "sqlite-firefox-2-of-5 duration evidence: reading report"
    )
    assert lines[3].startswith(
        "sqlite-firefox-2-of-5 trace evidence: reading Playwright"
    )
    assert lines[4].startswith("reading phase sidecar ")
    assert len(lines) == 5, lines


def test_a_lane_of_another_pair_is_unexpected(tmp_path: Path) -> None:
    evidence = fixture(tmp_path)
    foreign = replace(evidence[0], identity="sqlite-chromium-1-of-3")
    assert "unexpected lane `sqlite-chromium-1-of-3`" in errors([*evidence, foreign])


def test_a_duplicate_attempt_and_a_dropped_navigation_fail_closed(
    tmp_path: Path,
) -> None:
    evidence = fixture(tmp_path / "attempt")

    def duplicate(document: Any) -> None:
        attempts = document["tests"][0]["attempts"]
        attempts.append(attempts[0])

    edit(evidence[0].duration_manifest, duplicate)
    assert "duplicate retry" in errors(evidence)

    evidence = fixture(tmp_path / "dropped")
    dropped = shard(EVIDENCE, 1, "trace").replace(
        'e2e.navigation_top_dropped","value":{"stringValue":"0"',
        'e2e.navigation_top_dropped","value":{"stringValue":"1"',
    )
    write_lane_capture(evidence[0].capture, dropped)
    assert "sqlite-firefox-1-of-5 trace evidence: " in errors(evidence)
    assert "dropped" in errors(evidence)


def test_a_wrong_lane_and_a_failed_phase_fail_closed(tmp_path: Path) -> None:
    evidence = fixture(tmp_path / "lane")
    edit(evidence[0].phase, lambda phase: phase.update(lane="wrong-lane"))
    assert "sqlite-firefox-1-of-5 phase sidecar: identity mismatch" in errors(evidence)

    evidence = fixture(tmp_path / "failed")

    def fail(phase: Any) -> None:
        phase["phases"][2].update(outcome="failed", detail="panic")

    edit(evidence[2].phase, fail)
    assert "sqlite-firefox-3-of-5 phase sidecar: " in errors(evidence)
    assert "phase is incomplete or failed" in errors(evidence)


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (lambda phases: phases.pop(1), "requires exactly one gate-execution phase"),
        (lambda phases: phases.append(phases[1]), "exactly one gate-execution"),
        (lambda phases: phases[1].update(duration_ms=None), "incomplete or failed"),
        (lambda phases: phases[1].update(detail=""), "incomplete or failed"),
        (lambda phases: phases[0].update(outcome="failed"), "records a failed phase"),
        (lambda phases: phases[0].update(name="nix-dreaming"), "unknown phase"),
        (lambda phases: phases[0].update(outcome="maybe"), "unknown phase outcome"),
        (lambda phases: phases[1].update(duration_ms=-1), "parsing phase sidecar"),
    ],
)
def test_a_phase_sidecar_needs_each_required_phase_to_succeed(
    tmp_path: Path, change: Any, message: str
) -> None:
    evidence = fixture(tmp_path)
    edit(evidence[0].phase, lambda sidecar: change(sidecar["phases"]))
    assert message in errors(evidence)


# Ownership.


def lane(identity: str) -> ExpectedLane:
    return next(lane for lane in read_catalog(CATALOG) if lane.identity == identity)


def ownership() -> list[tuple[str, str, str, str]]:
    return [
        (
            identity,
            CENSUS,
            shard(OWNERSHIP, index, "report"),
            shard(OWNERSHIP, index, "manifest"),
        )
        for index, identity in enumerate(IDENTITIES, 1)
    ]


def test_a_producer_population_rejects_unowned_projects_mixed_reports_and_wrong_lane() -> (
    None
):
    first = lane("sqlite-firefox-1-of-5")
    report, manifest = shard(OWNERSHIP, 1, "report"), shard(OWNERSHIP, 1, "manifest")
    validate_population(first, report, manifest)
    for wrong_report, wrong_manifest, message in [
        (shard(OWNERSHIP, 2, "report"), manifest, "population mismatch"),
        (report, shard(OWNERSHIP, 2, "manifest"), "mismatched or incomplete"),
        ("{", manifest, "malformed report"),
        (report, "{", "malformed lane manifest"),
        (
            report.replace('"projectName": "firefox"', '"projectName": "chromium"'),
            manifest.replace('"project_name": "firefox"', '"project_name": "chromium"'),
            "differs from the lane's browser project",
        ),
    ]:
        with pytest.raises(EvidenceError, match=message):
            validate_population(first, wrong_report, wrong_manifest)


def census_with(change: Any) -> str:
    census = json.loads(CENSUS)
    change(census)
    return json.dumps(census)


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (lambda evidence: evidence.pop(), "does not exactly match the catalog"),
        (lambda evidence: evidence.append(evidence[0]), "does not exactly match"),
        (
            lambda evidence: evidence.__setitem__(
                0, (*evidence[0][:3], evidence[1][3])
            ),
            "mismatched lane manifest for `sqlite-firefox-1-of-5`",
        ),
        (
            lambda evidence: evidence.__setitem__(
                0, (*evidence[0][:2], evidence[1][2], evidence[0][3])
            ),
            "population mismatch for `sqlite-firefox-1-of-5`",
        ),
        (
            lambda evidence: evidence.__setitem__(
                0, (evidence[0][0], "{", *evidence[0][2:])
            ),
            "malformed expected census for `sqlite-firefox-1-of-5`",
        ),
        (
            lambda evidence: evidence.__setitem__(0, (*evidence[0][:3], "{")),
            "malformed lane manifest for `sqlite-firefox-1-of-5`",
        ),
        (
            lambda evidence: evidence.__setitem__(
                0,
                (
                    evidence[0][0],
                    census_with(lambda census: census.update(topology="other")),
                    *evidence[0][2:],
                ),
            ),
            "incomplete or mismatched expected census",
        ),
        (
            lambda evidence: evidence.__setitem__(
                0,
                (
                    evidence[0][0],
                    census_with(lambda census: census.update(tests=[])),
                    *evidence[0][2:],
                ),
            ),
            "empty expected census",
        ),
        (
            lambda evidence: evidence.__setitem__(
                0,
                (
                    evidence[0][0],
                    census_with(
                        lambda census: census["tests"][0].update(
                            project_name="chromium"
                        )
                    ),
                    *evidence[0][2:],
                ),
            ),
            "expected census has unowned project",
        ),
        (
            lambda evidence: evidence.__setitem__(
                1,
                (
                    evidence[1][0],
                    census_with(lambda census: census["tests"].pop()),
                    *evidence[1][2:],
                ),
            ),
            "lifted expected censuses are not identical",
        ),
        (
            lambda evidence: [
                evidence.__setitem__(
                    index,
                    (
                        evidence[index][0],
                        census_with(lambda census: census["tests"].pop()),
                        *evidence[index][2:],
                    ),
                )
                for index in range(5)
            ],
            "global expected census/report union mismatch",
        ),
    ],
)
def test_ownership_evidence_fails_closed(change: Any, message: str) -> None:
    catalog = read_catalog(CATALOG)
    assert reconcile_ownership("sqlite", "firefox", catalog, ownership()) == (
        "sqlite/firefox: 5 candidate lanes, 5 expected tests"
    )
    evidence = ownership()
    change(evidence)
    with pytest.raises(EvidenceError, match=message):
        reconcile_ownership("sqlite", "firefox", catalog, evidence)


def manifest_with(index: int, change: Any) -> str:
    manifest = json.loads(shard(OWNERSHIP, index, "manifest"))
    change(manifest)
    return json.dumps(manifest)


def report_with(index: int, change: Any) -> str:
    report = json.loads(shard(OWNERSHIP, index, "report"))
    change(report["suites"][0]["specs"][0])
    return json.dumps(report)


@pytest.mark.parametrize(
    ("report", "manifest", "message"),
    [
        (
            None,
            lambda manifest: manifest["tests"][0].update(project_name="chromium"),
            "lane manifest has unowned project",
        ),
        (None, lambda manifest: manifest.update(tests=[]), "unowned project"),
        (
            lambda spec: spec["tests"].append(spec["tests"][0]),
            None,
            "duplicate report identity for `sqlite-firefox-1-of-5`",
        ),
        (
            lambda spec: spec.update(id=""),
            None,
            "malformed report test id",
        ),
        (
            lambda spec: spec["tests"][0].pop("projectId"),
            None,
            "malformed report project id",
        ),
        (
            lambda spec: spec["tests"][0].update(projectName=""),
            None,
            "malformed report project name",
        ),
        (
            None,
            lambda manifest: manifest["tests"][0].update(test_id=""),
            "malformed lane manifest identity",
        ),
        (
            None,
            lambda manifest: manifest["tests"].append(manifest["tests"][0]),
            "duplicate lane manifest identity",
        ),
    ],
)
def test_a_lane_population_fails_closed(
    report: Any, manifest: Any, message: str
) -> None:
    evidence = ownership()
    identity, census, report_raw, manifest_raw = evidence[0]
    if report is not None:
        report_raw = report_with(1, report)
    if manifest is not None:
        manifest_raw = manifest_with(1, manifest)
    evidence[0] = (identity, census, report_raw, manifest_raw)
    with pytest.raises(EvidenceError, match=message):
        reconcile_ownership("sqlite", "firefox", read_catalog(CATALOG), evidence)


def test_a_report_of_two_lanes_is_duplicated_across_them() -> None:
    evidence = ownership()
    second = evidence[1]
    evidence[1] = (
        second[0],
        second[1],
        evidence[0][2],
        second[3].replace("two", "one"),
    )
    with pytest.raises(EvidenceError, match="duplicate report identity across lanes"):
        reconcile_ownership("sqlite", "firefox", read_catalog(CATALOG), evidence)


def test_a_report_without_specs_has_no_population() -> None:
    evidence = ownership()
    evidence[0] = (*evidence[0][:2], '{"suites": []}', evidence[0][3])
    with pytest.raises(EvidenceError, match="has no specs"):
        reconcile_ownership("sqlite", "firefox", read_catalog(CATALOG), evidence)


# Strict decoding.


@pytest.mark.parametrize(
    ("manifest", "message"),
    [
        ({"complete": "yes"}, "`complete`: expected a boolean"),
        ({"lane": 5}, "`lane`: expected a string"),
        ({"shard_index": -1}, "`shard_index`: expected a non-negative integer"),
        ({"shard_count": True}, "`shard_count`: expected a non-negative integer"),
        ({"schema_version": 1.0}, "`schema_version`: expected a non-negative"),
    ],
)
def test_a_lane_manifest_is_decoded_strictly(manifest: Any, message: str) -> None:
    raw = manifest_with(1, lambda document: document.update(manifest))
    with pytest.raises(EvidenceError, match=message):
        validate_population(
            lane("sqlite-firefox-1-of-5"), shard(OWNERSHIP, 1, "report"), raw
        )


# Lane verification.


def test_a_lifted_lane_verifies_its_population_duration_and_boot_evidence(
    tmp_path: Path,
) -> None:
    fixture(tmp_path)
    root = tmp_path / "e2e-sqlite-firefox-1-of-5"
    detail, duration = verify_lane("sqlite-firefox-1-of-5", CATALOG, root)
    assert detail == (
        "sqlite-firefox-1-of-5: population, retry-duration and boot evidence verified"
    )
    assert duration.pressured == []
    for identity in ["sqlite-firefox-unsplit", "unknown"]:
        with pytest.raises(EvidenceError, match="unknown or disabled lane"):
            verify_lane(identity, CATALOG, root)
    (root / "playwright-report-sqlite-firefox-1-of-5.json").unlink()
    with pytest.raises(EvidenceError, match="reading report"):
        verify_lane("sqlite-firefox-1-of-5", CATALOG, root)


def test_a_verified_lane_under_pressure_lists_it(tmp_path: Path) -> None:
    evidence = fixture(tmp_path)
    slow(evidence[4])
    root = tmp_path / "e2e-sqlite-firefox-5-of-5"
    detail, duration = verify_lane("sqlite-firefox-5-of-5", CATALOG, root)
    assert detail == (
        "sqlite-firefox-5-of-5: population, retry-duration and boot evidence "
        f"verified\n{duration.detail()}"
    )
    assert len(duration.pressured) == 1


# The duration budget.

REPORT = json.dumps(
    {
        "suites": [
            {
                "title": "tests",
                "file": "",
                "line": 0,
                "specs": [
                    {
                        "id": "id-1",
                        "title": "works",
                        "file": "tests/works.spec.ts",
                        "line": 7,
                        "tests": [
                            {
                                "projectId": "chromium",
                                "projectName": "chromium",
                                "results": [
                                    {"retry": 0, "duration": 100, "status": "passed"}
                                ],
                            }
                        ],
                    }
                ],
            }
        ]
    },
    separators=(",", ":"),
)
MANIFEST = json.dumps(
    {
        "schema_version": 1,
        "complete": True,
        "tests": [
            {
                "test_id": "id-1",
                "project_id": "chromium",
                "project_name": "chromium",
                "title": "works",
                "file": "tests/works.spec.ts",
                "line": 7,
                "attempts": [{"retry": 0, "effective_timeout_ms": 1000}],
            }
        ],
    },
    separators=(",", ":"),
)


ATTEMPT = '{"retry":0,"duration":100,"status":"passed"}'
TWO_ATTEMPTS = MANIFEST.replace(
    '[{"retry":0,"effective_timeout_ms":1000}]',
    '[{"retry":0,"effective_timeout_ms":1000},{"retry":1,"effective_timeout_ms":1000}]',
)
WORKS = (
    "test_id=id-1 project_id=chromium project_name=chromium "
    "tests/works.spec.ts:7 title=works"
)


def attempts(*results: tuple[int, str]) -> str:
    """REPORT with one attempt per (duration, status), retries in order."""
    return REPORT.replace(
        ATTEMPT,
        ",".join(
            json.dumps(
                {"retry": retry, "duration": duration, "status": status},
                separators=(",", ":"),
            )
            for retry, (duration, status) in enumerate(results)
        ),
    )


def test_an_attempt_below_the_pressure_threshold_passes_unremarked() -> None:
    duration = validate_duration_json(REPORT, MANIFEST)
    assert duration.pressured == []
    assert duration.detail() == (
        f"maximum duration pressure 10.00%: {WORKS}; retry 0; "
        "duration 100.000 ms / effective timeout 1000.000 ms"
    )


@pytest.mark.parametrize(
    ("used", "shown"), [(800, "80.00%"), (850, "85.00%"), (1020, "102.00%")]
)
def test_a_passing_attempt_at_or_above_the_threshold_passes_with_a_warning(
    used: int, shown: str
) -> None:
    duration = validate_duration_json(attempts((used, "passed")), MANIFEST)
    assert [pressure.retry for pressure in duration.pressured] == [0]
    assert duration.detail() == (
        "duration pressure: 1 attempt(s) at or above 80% of their timeout\n"
        f"  {shown}: {WORKS}; retry 0; duration {used}.000 ms / effective "
        "timeout 1000.000 ms"
    )


def test_every_pressured_attempt_is_listed_worst_first() -> None:
    duration = validate_duration_json(
        attempts((850, "failed"), (900, "passed")), TWO_ATTEMPTS
    )
    assert [pressure.retry for pressure in duration.pressured] == [1, 0]
    assert duration.maximum == duration.pressured[0]


@pytest.mark.parametrize("timeout", [1_000, 3_000, 12_345])
def test_every_effective_timeout_mode_is_accepted(timeout: int) -> None:
    validate_duration_json(REPORT, MANIFEST.replace("1000", str(timeout)))


def test_a_timed_out_attempt_fails_even_when_its_retry_passes() -> None:
    report = attempts((1000, "timedOut"), (100, "passed"))
    with pytest.raises(EvidenceError, match="attempt timed out at 100.00%: .*retry 0"):
        validate_duration_json(report, TWO_ATTEMPTS)


@pytest.mark.parametrize("status", ["failed", "skipped", "interrupted"])
def test_a_failed_or_skipped_attempt_is_playwrights_verdict(status: str) -> None:
    duration = validate_duration_json(
        attempts((100, status), (100, "passed")), TWO_ATTEMPTS
    )
    assert duration.pressured == []


def with_manifest_test(change: Any) -> str:
    manifest = json.loads(MANIFEST)
    change(manifest)
    return json.dumps(manifest)


def with_report_test(change: Any) -> str:
    report = json.loads(REPORT)
    change(report["suites"][0]["specs"][0])
    return json.dumps(report)


@pytest.mark.parametrize(
    ("report", "manifest", "message"),
    [
        ('{"suites":[]}', MANIFEST, "report contains no selected tests"),
        ("not json", MANIFEST, "parsing Playwright report"),
        (REPORT, "{}", "parsing duration-budget manifest"),
        (
            REPORT,
            '{"schema_version":1,"complete":false,"tests":[]}',
            "not complete",
        ),
        (
            REPORT,
            '{"schema_version":2,"complete":true,"tests":[]}',
            "unsupported manifest schema_version 2; expected 1",
        ),
        (
            REPORT,
            '{"schema_version":1,"complete":true,"tests":[]}',
            "manifest selects no tests",
        ),
        (REPORT.replace('"id-1"', '"other-id"'), MANIFEST, "identity mismatch"),
        (
            REPORT,
            with_manifest_test(
                lambda manifest: manifest["tests"].append(
                    {**manifest["tests"][0], "test_id": "id-2"}
                )
            ),
            "report is missing selected test",
        ),
        (
            with_report_test(
                lambda spec: spec["tests"].append(
                    {**spec["tests"][0], "projectId": "firefox"}
                )
            ),
            MANIFEST,
            "report has unexpected selected test",
        ),
        (
            REPORT.replace('"retry":0', '"retry":1'),
            MANIFEST,
            "report retries must start at zero and be contiguous",
        ),
        (
            attempts((100, "failed"), (100, "passed")),
            MANIFEST,
            "retry reconciliation mismatch",
        ),
        (
            REPORT,
            MANIFEST.replace('"retry":0', '"retry":1'),
            "manifest retries must start at zero and be contiguous",
        ),
        (
            REPORT,
            with_manifest_test(lambda manifest: manifest["tests"][0].update(title="")),
            "manifest test has an empty title",
        ),
        (
            REPORT,
            with_manifest_test(lambda manifest: manifest["tests"][0].update(line=0)),
            "manifest test has line 0",
        ),
        (
            REPORT,
            with_manifest_test(
                lambda manifest: manifest["tests"][0].update(attempts=[])
            ),
            "manifest test has no attempts",
        ),
        (
            REPORT,
            MANIFEST.replace('"effective_timeout_ms":1000', '"effective_timeout_ms":0'),
            "non-positive or non-finite effective timeout",
        ),
        (
            REPORT,
            with_manifest_test(
                lambda manifest: manifest["tests"].append(manifest["tests"][0])
            ),
            "manifest has a duplicate selected test",
        ),
        (
            with_report_test(lambda spec: spec.update(tests=[])),
            MANIFEST,
            "report spec id-1 has no project results",
        ),
        (
            with_report_test(lambda spec: spec.update(id=None, tests=[])),
            MANIFEST,
            "report spec <missing> has no project results",
        ),
        (
            with_report_test(lambda spec: spec.pop("file")),
            MANIFEST,
            "report test has an empty file",
        ),
        (
            with_report_test(lambda spec: spec["tests"][0].update(results=[])),
            MANIFEST,
            "report test has no results",
        ),
        (
            REPORT.replace('"retry":0,', ""),
            MANIFEST,
            "report attempt has no retry",
        ),
        (
            REPORT.replace(',"duration":100', ""),
            MANIFEST,
            "report retry 0 has no duration",
        ),
        (
            REPORT.replace(',"status":"passed"', ""),
            MANIFEST,
            "report retry 0 has a missing or unknown status null",
        ),
        (
            REPORT.replace('"passed"', '"flaky"'),
            MANIFEST,
            'report retry 0 has a missing or unknown status "flaky"',
        ),
        (
            REPORT.replace('"duration":100', '"duration":-1'),
            MANIFEST,
            "non-finite or negative duration",
        ),
        (
            REPORT.replace(ATTEMPT, f"{ATTEMPT},{ATTEMPT}"),
            MANIFEST,
            "report has duplicate retry 0",
        ),
        (
            with_report_test(lambda spec: spec["tests"].append(spec["tests"][0])),
            MANIFEST,
            "report has a duplicate selected test",
        ),
    ],
)
def test_the_duration_budget_fails_closed(
    report: str, manifest: str, message: str
) -> None:
    with pytest.raises(EvidenceError, match=message):
        validate_duration_json(report, manifest)


def test_both_missing_and_unexpected_tests_are_named() -> None:
    report = with_report_test(lambda spec: spec.update(id="other"))
    with pytest.raises(EvidenceError) as refused:
        validate_duration_json(report, MANIFEST)
    assert "missing test_id=id-1" in str(refused.value)
    assert "unexpected test_id=other" in str(refused.value)


def test_pressure_without_a_step_summary_is_still_annotated(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.delenv("GITHUB_STEP_SUMMARY", raising=False)
    e2e._report_pressure(
        "lane", validate_duration_json(attempts((900, "passed")), MANIFEST)
    )
    assert capsys.readouterr().out.startswith(
        "::warning title=Duration pressure::lane: "
    )


def test_an_unwritable_step_summary_is_said_never_failed(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(tmp_path))
    e2e._report_pressure(
        "lane", validate_duration_json(attempts((900, "passed")), MANIFEST)
    )
    captured = capsys.readouterr()
    assert captured.out.startswith("::warning title=Duration pressure::")
    assert captured.err.startswith(f"dev: {tmp_path}: no pressure summary: ")


def test_duration_files_name_what_could_not_be_read(tmp_path: Path) -> None:
    report = tmp_path / "report.json"
    report.write_text(REPORT)
    with pytest.raises(EvidenceError, match="reading manifest"):
        validate_duration_files(report, tmp_path / "missing-manifest.json")


# Boot decomposition coverage.


def project_report(*projects: Any) -> str:
    return json.dumps(
        {
            "suites": [
                {
                    "specs": [
                        {"tests": [{"projectName": project} for project in projects]}
                    ]
                }
            ]
        }
    )


def string_attr(key: str, value: str) -> dict[str, Any]:
    return {"key": key, "value": {"stringValue": value}}


def boot_navigation(id: int = 1, complete: bool = True, **extra: Any) -> dict[str, Any]:
    phases = (
        {
            "jaunder.boot.entry->jaunder.boot.seed_parsed": 10.0,
            "jaunder.boot.seed_parsed->jaunder.boot.render_start": 15.0,
            "jaunder.boot.render_start->jaunder.boot.mount_done": 20.0,
        }
        if complete
        else {"jaunder.boot.entry->jaunder.boot.mount_done": 40.0}
    )
    return {
        "id": id,
        "wasmTimingSchema": "direct-init-v1",
        "commitToMountMs": 42.0,
        "wasmInitStartMs": 5.0,
        "wasmInitStartToBootEntryMs": 5.0,
        "bootPhases": phases,
        **extra,
    }


def boot_marks(id: int = 1) -> dict[str, Any]:
    return {
        "id": id,
        "marks": [
            {"name": "jaunder.boot.entry", "startTime": 10.0},
            {"name": "jaunder.boot.seed_parsed", "startTime": 20.0},
            {"name": "jaunder.boot.render_start", "startTime": 35.0},
            {"name": "jaunder.boot.mount_done", "startTime": 55.0},
        ],
    }


def boot_trace(
    project: str = "chromium",
    span: str = "e2e.test",
    navigations: Any = None,
    marks: Any = "default",
    dropped: str | None = "0",
) -> str:
    attributes = [string_attr("e2e.project", project)]
    if navigations is not None:
        attributes.append(string_attr("e2e.navigation_top_json", navigations))
    if marks is not None:
        if marks == "default":
            marks = json.dumps([boot_marks()])
        attributes.append(string_attr("e2e.boot_marks_json", marks))
    if dropped is not None:
        attributes.append(string_attr("e2e.navigation_top_dropped", dropped))
    return json.dumps(
        {
            "resourceSpans": [
                {"scopeSpans": [{"spans": [{"name": span, "attributes": attributes}]}]}
            ]
        }
    )


def navigations(*entries: Any) -> str:
    return json.dumps(list(entries))


VALID = boot_trace(navigations=navigations(boot_navigation()))


def test_complete_current_evidence_for_every_reported_project_passes() -> None:
    trace = "\n".join(
        [
            VALID,
            boot_trace(
                "firefox",
                "e2e.page",
                navigations(boot_navigation(2)),
                json.dumps([boot_marks(2)]),
            ),
        ]
    )
    assert validate_boot_json(project_report("chromium", "firefox"), trace) == (
        "boot-decomposition evidence complete for 2 project(s)"
    )


@pytest.mark.parametrize(
    ("report", "trace", "message"),
    [
        ("", VALID, "parsing Playwright report"),
        ('{"suites":[]}', VALID, "no complete project population"),
        (project_report("chromium"), "", "no navigation-bearing"),
        (project_report("chromium"), "{", "parsing trace capture"),
        (
            project_report("chromium"),
            boot_trace(navigations="{"),
            "project chromium: malformed navigation evidence: parsing JSON trace",
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=json.dumps({"id": 1})),
            "navigation evidence is not an array",
        ),
        (project_report("chromium"), boot_trace(), "missing navigation evidence"),
        (
            project_report("chromium"),
            boot_trace(span="e2e.page"),
            "missing navigation evidence",
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=navigations(boot_navigation()), dropped=None),
            "missing dropped-navigation count",
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=navigations(boot_navigation()), dropped="-1"),
            'malformed dropped-navigation count "-1"',
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=navigations(boot_navigation()), dropped=str(2**64)),
            "malformed dropped-navigation count",
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=navigations(boot_navigation()), dropped="1"),
            "project chromium: 1 dropped navigation record(s)",
        ),
        (
            project_report("chromium"),
            boot_trace(navigations=navigations(boot_navigation()), marks="{"),
            "malformed boot-mark evidence",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps({"id": 1}),
            ),
            "boot-mark evidence is not an array",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps([boot_marks(), boot_marks()]),
            ),
            "duplicate boot-mark navigation id 1",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps(
                    [
                        {
                            "id": 1,
                            "marks": [
                                {"name": "jaunder.boot.entry", "startTime": "early"}
                            ],
                        }
                    ]
                ),
            ),
            "boot-mark startTime is not a finite number",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps([{"id": 1, "marks": [{"startTime": 1.0}]}]),
            ),
            "boot-mark record has no string name",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps([{"id": 1, "marks": ["mark"]}]),
            ),
            "boot-mark record is not an object",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps([{"id": 1}]),
            ),
            "boot-mark entry has no marks array",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()),
                marks=json.dumps([{"marks": []}]),
            ),
            "boot-mark entry has no integer id",
        ),
        (
            project_report("chromium"),
            boot_trace(
                navigations=navigations(boot_navigation()), marks=json.dumps(["entry"])
            ),
            "boot-mark entry is not an object",
        ),
        (project_report("chromium", "firefox"), VALID, "project-set mismatch"),
        (
            project_report("chromium"),
            boot_trace(project="", navigations=navigations(boot_navigation())),
            "e2e.test span has no e2e.project",
        ),
    ],
)
def test_boot_evidence_fails_closed(report: str, trace: str, message: str) -> None:
    with pytest.raises(EvidenceError) as refused:
        validate_boot_json(report, trace)
    assert message in str(refused.value)


def test_a_project_set_mismatch_names_both_sides() -> None:
    with pytest.raises(EvidenceError) as refused:
        validate_boot_json(project_report("firefox", "webkit"), VALID)
    assert str(refused.value) == (
        "project-set mismatch: missing trace projects [firefox, webkit]; "
        "unexpected trace projects [chromium]"
    )


@pytest.mark.parametrize("malformed", [{}, {"projectName": None}, {"projectName": ""}])
def test_a_mixed_report_with_an_incomplete_project_name_is_rejected(
    malformed: Any,
) -> None:
    report = json.dumps(
        {"suites": [{"specs": [{"tests": [{"projectName": "chromium"}, malformed]}]}]}
    )
    with pytest.raises(EvidenceError, match="missing or empty projectName"):
        validate_boot_json(report, VALID)


def test_server_spans_beside_the_e2e_spans_are_not_evidence() -> None:
    record = json.loads(VALID)
    record["resourceSpans"][0]["scopeSpans"][0]["spans"].append({"name": "request"})
    validate_boot_json(project_report("chromium"), json.dumps(record))


def test_a_malformed_span_beside_valid_evidence_is_rejected() -> None:
    record = json.loads(VALID)
    record["resourceSpans"][0]["scopeSpans"][0]["spans"].append({})
    with pytest.raises(
        EvidenceError, match="span entry without a nonempty string name"
    ):
        validate_boot_json(project_report("chromium"), json.dumps(record))


@pytest.mark.parametrize(
    ("navigation", "span", "marks", "message"),
    [
        ({"commitToMountMs": None}, "e2e.test", "default", "no mounted navigation"),
        (
            {"commitToMountMs": "fast"},
            "e2e.test",
            "default",
            "not null or a finite number",
        ),
        ({"wasmTimingSchema": "legacy-v1"}, "e2e.test", "default", "legacy or missing"),
        ({"wasmInitStartMs": None}, "e2e.test", "default", "incomplete direct-init"),
        (
            {"bootPhases": {"jaunder.boot.entry->jaunder.boot.mount_done": 40.0}},
            "e2e.test",
            "default",
            "incomplete direct-init decomposition",
        ),
        (
            {
                "bootPhases": {
                    "jaunder.boot.entry->jaunder.boot.mount_done": 40.0,
                    "notBoot": 1.0,
                    "boot.not_an_interval": "not finite",
                    "diagnostic": 3.0,
                }
            },
            "e2e.test",
            "default",
            "incomplete direct-init decomposition",
        ),
        ({"id": None}, "e2e.test", "default", "incomplete direct-init"),
        (
            {
                "bootPhases": {
                    "jaunder.boot.entry->jaunder.boot.seed_parsed": 10.0,
                    "jaunder.boot.seed_parsed->jaunder.boot.render_start": 15.0,
                    "jaunder.boot.render_start->jaunder.boot.mount_done": 22.0,
                }
            },
            "e2e.test",
            "default",
            "does not close within 1 ms",
        ),
        (
            {
                "bootPhases": {
                    "jaunder.boot.entry->jaunder.boot.seed_parsed": 10.0,
                    "jaunder.boot.seed_parsed->jaunder.boot.render_start": 15.0,
                    "jaunder.boot.render_start->jaunder.boot.mount_done": 22.0,
                }
            },
            "e2e.page",
            "default",
            "does not close within 1 ms",
        ),
        ({"documentBootTotalMs": 55.0}, "e2e.test", None, "incomplete direct-init"),
        ({"documentBootTotalMs": 56.5}, "e2e.page", None, "does not close within 1 ms"),
    ],
)
def test_a_mounted_navigation_must_be_current_complete_and_closing(
    navigation: dict[str, Any], span: str, marks: Any, message: str
) -> None:
    trace = boot_trace(
        span=span, navigations=navigations(boot_navigation(**navigation)), marks=marks
    )
    with pytest.raises(EvidenceError, match=message):
        validate_boot_json(project_report("chromium"), trace)


def test_a_navigation_missing_its_mount_field_or_not_an_object_is_rejected() -> None:
    navigation = boot_navigation()
    del navigation["commitToMountMs"]
    for entry, message in [
        (navigation, "no commitToMountMs"),
        (False, "navigation entry is not an object"),
    ]:
        trace = boot_trace(navigations=navigations(entry))
        with pytest.raises(EvidenceError, match=message):
            validate_boot_json(project_report("chromium"), trace)


def test_a_page_navigation_closed_to_its_document_total_needs_no_marks() -> None:
    trace = boot_trace(
        span="e2e.page",
        navigations=navigations(boot_navigation(documentBootTotalMs=55.0)),
        marks=None,
    )
    validate_boot_json(project_report("chromium"), trace)


def test_boot_files_need_a_report_and_exactly_one_nonempty_trace(
    tmp_path: Path,
) -> None:
    report = tmp_path / "playwright-report-sqlite.json"
    lane_capture = tmp_path / "capture-sqlite.tar.gz"
    report.write_text(project_report("chromium"))
    write_capture(lane_capture, [(TRACE_MEMBER, VALID.encode())])
    assert "1 project(s)" in validate_boot_files(report, lane_capture)

    report.write_text("")
    with pytest.raises(EvidenceError, match="Playwright report .* is empty"):
        validate_boot_files(report, lane_capture)
    report.write_text(project_report("chromium"))
    for members, message in [
        (None, "opening capture"),
        ([], "missing"),
        ([(TRACE_MEMBER, b"")], "empty"),
        ([(TRACE_MEMBER, VALID.encode())] * 2, "ambiguous"),
    ]:
        lane_capture.unlink(missing_ok=True)
        if members is not None:
            write_capture(lane_capture, members)
        with pytest.raises(EvidenceError, match=f"reading capture .*{message}"):
            validate_boot_files(report, lane_capture)
    lane_capture.write_bytes(b"not a gzip archive")
    with pytest.raises(EvidenceError, match="reading capture"):
        validate_boot_files(report, lane_capture)
    report.unlink()
    with pytest.raises(EvidenceError, match="reading Playwright report"):
        validate_boot_files(report, lane_capture)
