"""Keeps a checkout's run evidence after its worktree is removed.

Worktrunk's `pre-remove` hook (`.config/wt.toml`) runs `dev keep-runs` inside
the worktree about to be deleted. It copies the span records, merged traces and
`traced-run` summaries from `.xtask/run/` into a per-user store, one directory
per checkout, and leaves the parked stdout/stderr bodies behind. The store is
capped by deleting the least recently kept checkouts' directories first.
"""

import os
import shutil
from collections.abc import Mapping
from pathlib import Path

CAP_BYTES = 20 * 1024**3
# A run's span records (what `dev trace summary` reads), its merged trace, and
# a `traced-run` summary.
KEPT = ("*.trace.d", "*.trace.json", "*.run.json")


def store(environment: Mapping[str, str]) -> Path:
    state = environment.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(state) / "jaunder" / "runs"


def _size(path: Path) -> int:
    return sum(entry.stat().st_size for entry in path.rglob("*") if entry.is_file())


def _prune(runs: Path, cap: int, spare: Path) -> None:
    """Deletes kept checkouts, least recently kept first, until `runs` fits `cap`.

    `spare`, the checkout just kept, is never deleted, so it alone can leave
    the store above the cap.
    """
    kept = sorted(
        (entry for entry in runs.iterdir() if entry.is_dir()),
        key=lambda entry: entry.stat().st_mtime_ns,
    )
    sizes = {entry: _size(entry) for entry in kept}
    total = sum(sizes.values())
    for entry in kept:
        if total <= cap:
            break
        if entry != spare:
            shutil.rmtree(entry)
            total -= sizes[entry]


def keep(checkout: Path, runs: Path, cap: int = CAP_BYTES) -> Path | None:
    """Copies `checkout`'s runs into `runs`; returns where, or None if it had none.

    The store is keyed by the checkout's directory name, since its branch may
    already be deleted. Copying again replaces the same names. An OSError is
    left to the caller.
    """
    sources = sorted(
        path for pattern in KEPT for path in (checkout / ".xtask" / "run").glob(pattern)
    )
    if not sources:
        return None
    target = runs / checkout.name
    target.mkdir(parents=True, exist_ok=True)
    for source in sources:
        if source.is_dir():
            shutil.copytree(source, target / source.name, dirs_exist_ok=True)
        else:
            shutil.copy2(source, target / source.name)
    # Kept now, whenever its directory was created: the cap deletes it last.
    os.utime(target)
    _prune(runs, cap, target)
    return target
