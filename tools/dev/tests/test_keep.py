"""`dev keep-runs`: a checkout's run evidence kept in the per-user store."""

import os
from pathlib import Path

import pytest
from conftest import Checkout

from dev import keep


def stored(checkout: Checkout) -> Path:
    return Path(os.environ["XDG_STATE_HOME"]) / "jaunder" / "runs" / checkout.root.name


def test_a_removal_keeps_traces_and_summaries_but_not_output(
    checkout: Checkout,
) -> None:
    checkout.tool("tsc", "")
    assert checkout.dev("static", "tsc", "--sandbox").returncode == 0
    runs = checkout.root / ".xtask" / "run"
    for name in ["1-2-3.run.json", "1-2-3.out", "1-2-3.err"]:
        (runs / name).write_text("{}\n")

    result = checkout.dev("keep-runs")
    assert result.returncode == 0, result
    target = stored(checkout)
    assert result.stdout.decode() == f"kept runs in {target}\n"
    (run,) = checkout.runs()
    assert sorted(path.name for path in target.iterdir()) == sorted(
        [run.name, run.name.replace(".d", ".json"), "1-2-3.run.json"]
    )
    assert sorted(path.name for path in (target / run.name).iterdir()) == sorted(
        path.name for path in run.iterdir()
    )
    # The kept records read as the checkout's did; keeping again replaces them.
    summary = checkout.dev("trace", "summary", str(target / run.name))
    assert summary.returncode == 0, summary
    assert "dev static tsc" in summary.stdout.decode()
    assert checkout.dev("keep-runs").returncode == 0


def test_a_checkout_without_runs_keeps_nothing(checkout: Checkout) -> None:
    result = checkout.dev("keep-runs")
    assert (result.returncode, result.stdout) == (0, b"no runs to keep\n")
    assert not stored(checkout).parent.exists()


def test_a_failed_copy_warns_and_fails(checkout: Checkout) -> None:
    runs = checkout.root / ".xtask" / "run"
    runs.mkdir(parents=True)
    (runs / "1.run.json").write_text("{}\n")
    stored(checkout).parent.parent.mkdir(parents=True)
    # The store's directory is a file, so nothing can be kept in it.
    stored(checkout).parent.write_text("")
    result = checkout.dev("keep-runs")
    assert result.returncode == 1
    assert result.stderr.decode().startswith("dev: keep-runs: warning: ")


def kept(runs: Path, name: str, size: int, mtime: int) -> Path:
    directory = runs / name
    directory.mkdir(parents=True)
    (directory / "1.trace.json").write_bytes(b"x" * size)
    os.utime(directory, (mtime, mtime))
    return directory


@pytest.mark.parametrize(
    ("cap", "left"),
    [(30, ["new", "newer", "old"]), (25, ["new", "newer"]), (0, ["new"])],
)
def test_the_store_drops_the_least_recently_kept_first(
    tmp_path: Path, cap: int, left: list[str]
) -> None:
    runs = tmp_path / "store"
    kept(runs, "old", 10, 1)
    kept(runs, "newer", 10, 2)
    checkout = tmp_path / "new"
    (checkout / ".xtask" / "run").mkdir(parents=True)
    (checkout / ".xtask" / "run" / "1.run.json").write_bytes(b"x" * 10)
    # A kept directory is aged by when it was kept, not when it was created.
    kept(runs, "new", 0, 0)
    assert keep.keep(checkout, runs, cap) == runs / "new"
    assert sorted(path.name for path in runs.iterdir()) == left


def test_the_store_defaults_to_the_user_state_directory(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    assert keep.store({}) == tmp_path / ".local" / "state" / "jaunder" / "runs"
    assert keep.store({"XDG_STATE_HOME": "/s"}) == Path("/s/jaunder/runs")
