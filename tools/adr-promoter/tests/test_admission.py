"""Offline adapter/error proof; real Nix predicate coverage lives in host tests."""

import json
import subprocess

import pytest

from adr_promoter.admission import admit_checkout
from test_promotion import tracked_repo, write


@pytest.mark.parametrize(("status", "response", "error"), [
    (0, b'["tools/adr-promoter/tests/new.py"]', ValueError),
    (1, b"", RuntimeError),
    (0, b"{}", ValueError),
])
def test_admission_preserves_candidates_and_fails_closed(tmp_path, monkeypatch, status, response, error):
    tracked_repo(tmp_path, {".gitignore": b"ignored/\n"})
    write(tmp_path, "tools/adr-promoter/tests/new.py", b"source\n")
    write(tmp_path, "notes.md", b"unrelated\n")
    write(tmp_path, "ignored/test.py", b"ignored\n")
    original = subprocess.run
    observed = []

    def nix_response(arguments, **kwargs):
        if arguments[0] != "nix":
            return original(arguments, **kwargs)
        assert ".#adr-promoter.sourceAdmits" in arguments
        observed.extend(json.loads(kwargs["env"]["JAUNDER_XTASK_UNTRACKED_INPUTS"]))
        assert "GIT_DIR" not in kwargs["env"]
        return subprocess.CompletedProcess(arguments, status, response, b"fixture eval failure")

    monkeypatch.setattr(subprocess, "run", nix_response)
    monkeypatch.setenv("GIT_DIR", "/must-not-redirect")
    with pytest.raises(error):
        admit_checkout(tmp_path)
    assert observed == [
        {"path": "notes.md", "kind": "regular"},
        {"path": "tools/adr-promoter/tests/new.py", "kind": "regular"},
    ]
