"""setup-ci's Nix configuration honours non-substitutable verdicts.

install-nix-action writes `always-allow-substitutes = true`, which would let CI
fetch the Rust coverage and e2e verdicts `nonSubstitutable` marks. It needs the
repository's `.github/`, so CI's host lane runs it, not `checks.dev-python`.
"""

import pytest
import yaml
from conftest import REPOSITORY, require

pytestmark = pytest.mark.repository

ACTION = REPOSITORY / ".github" / "actions" / "setup-ci" / "action.yml"


def test_setup_ci_turns_off_always_allow_substitutes() -> None:
    require(ACTION.is_file(), "the repository's setup-ci action")
    steps = yaml.safe_load(ACTION.read_text())["runs"]["steps"]
    (install,) = [
        step
        for step in steps
        if step.get("uses", "").startswith("cachix/install-nix-action@")
    ]
    settings = [
        [word.strip() for word in line.split("=", 1)]
        for line in install["with"]["extra_nix_config"].splitlines()
    ]
    assert ["always-allow-substitutes", "false"] in settings
