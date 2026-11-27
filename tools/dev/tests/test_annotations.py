"""GitHub Actions annotations as `dev` writes them."""

import pytest

from dev import annotations


@pytest.mark.parametrize(("value", "inside"), [("true", True), ("", False)])
def test_only_a_github_actions_job_takes_annotations(
    monkeypatch: pytest.MonkeyPatch, value: str, inside: bool
) -> None:
    monkeypatch.setenv("GITHUB_ACTIONS", value)
    assert annotations.in_github_actions() is inside


def test_a_warning_escapes_its_message(capsys: pytest.CaptureFixture[str]) -> None:
    annotations.warning("Duration pressure", "90%: one\r\ntwo")
    assert capsys.readouterr().out == (
        "::warning title=Duration pressure::90%25: one%0D%0Atwo\n"
    )
