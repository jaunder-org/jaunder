"""A test whose prerequisite is missing fails, wherever it runs."""

import pytest
from conftest import require


def test_a_present_prerequisite_neither_skips_nor_fails() -> None:
    require(True, "anything")


def test_a_missing_prerequisite_fails() -> None:
    with pytest.raises(pytest.fail.Exception, match="missing prerequisite: jq"):
        require(False, "jq")
