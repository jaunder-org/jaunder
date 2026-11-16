"""A test whose prerequisite is missing fails, except inside `checks.dev-python`."""

import pytest
from conftest import require


def test_a_present_prerequisite_neither_skips_nor_fails() -> None:
    require(True, "anything")


def test_a_missing_prerequisite_fails_outside_the_nix_check(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("JAUNDER_DEV_PYTHON_CHECK", raising=False)
    with pytest.raises(pytest.fail.Exception, match="missing prerequisite: jq"):
        require(False, "jq")


def test_a_missing_prerequisite_skips_inside_the_nix_check(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("JAUNDER_DEV_PYTHON_CHECK", "1")
    with pytest.raises(pytest.skip.Exception, match="needs jq"):
        require(False, "jq")
