"""ADR-0225 admission: Git supplies candidates, the selected Nix filter judges."""

import json
from pathlib import Path
import stat
import subprocess

from . import git


# Match the existing host_tooling adapter; Nix, not Python, defines membership.
ADMIT_INPUTS = """sourceAdmits:
let candidates = builtins.fromJSON (builtins.getEnv "JAUNDER_XTASK_UNTRACKED_INPUTS");
in map (candidate: candidate.path)
  (builtins.filter (candidate: sourceAdmits candidate.path candidate.kind) candidates)
"""


def admit_checkout(repo: Path) -> None:
    candidates = []
    for path in git.paths(repo, "ls-files", "--others", "--exclude-standard", "-z"):
        mode = (repo / path).lstat().st_mode
        if stat.S_ISLNK(mode):
            kind = "symlink"
        elif stat.S_ISDIR(mode):
            kind = "directory"
        elif stat.S_ISREG(mode):
            kind = "regular"
        else:
            raise ValueError(f"unsupported untracked input type: {path}")
        candidates.append({"path": path, "kind": kind})
    if not candidates:
        return
    environment = git.environment()
    environment["JAUNDER_XTASK_UNTRACKED_INPUTS"] = json.dumps(candidates)
    result = subprocess.run(
        ["nix", "eval", "--impure", "--json", "--accept-flake-config",
         "--no-update-lock-file", "--no-write-lock-file",
         ".#adr-promoter.sourceAdmits", "--apply", ADMIT_INPUTS],
        cwd=repo, env=environment, capture_output=True, check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "cannot verify Nix tooling inputs; ensure new tooling files are tracked "
            "with git add (or git add -N): "
            + result.stderr.decode("utf-8", errors="replace").strip()
        )
    omitted = json.loads(result.stdout)
    if not isinstance(omitted, list) or not all(isinstance(path, str) for path in omitted):
        raise ValueError("invalid Nix tooling input admission response")
    if omitted:
        raise ValueError(
            "Git flakes omit untracked declared inputs: " + ", ".join(omitted)
            + ". Track them with git add (or git add -N) before invoking adr-promoter"
        )
