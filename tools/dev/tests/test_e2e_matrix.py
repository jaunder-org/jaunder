"""CI runs every enabled e2e lane, and only a well-formed lane catalog.

`scripts/e2e-matrix.jq` projects the lane catalog into the CI matrix. A wrong
projection or a malformed catalog would silently run fewer lanes, or merge
lanes' traces, while CI stays green. The projection runs under the real `jq`
over the repository's catalog and mutated copies of it. It needs the
repository's files and `jq`, so only the filtered `checks.dev-python` source
may skip it.
"""

import json
import shutil
import subprocess
from typing import Any

import pytest
from conftest import REPOSITORY, require

PROJECTION = "scripts/e2e-matrix.jq"


def lane_catalog() -> Any:
    """The repository's lane catalog, freshly parsed."""
    path = REPOSITORY / "end2end" / "e2e-lanes.json"
    require(path.is_file(), "the repository's end2end/e2e-lanes.json")
    return json.loads(path.read_text(encoding="utf-8"))


def project(lanes: Any) -> subprocess.CompletedProcess[str]:
    require(shutil.which("jq") is not None, "jq")
    require((REPOSITORY / PROJECTION).is_file(), f"the repository's {PROJECTION}")
    return subprocess.run(
        ["jq", "-r", "-f", PROJECTION],
        cwd=REPOSITORY,
        input=json.dumps(lanes),
        capture_output=True,
        text=True,
        check=False,
    )


def test_the_matrix_runs_every_enabled_lane_and_reconciles_every_pair() -> None:
    lanes = lane_catalog()
    result = project(lanes)
    assert result.returncode == 0, result.stderr
    # `key=json` lines, as GitHub reads `$GITHUB_OUTPUT`.
    values = {
        key: json.loads(value)
        for key, value in (line.split("=", 1) for line in result.stdout.splitlines())
    }
    enabled = [lane for lane in lanes["lanes"] if lane["enabled"] is True]
    assert {item["lane"] for item in values["lanes"]} == {
        lane["identity"] for lane in enabled
    }
    assert {(pair["backend"], pair["browser"]) for pair in values["reconciles"]} == {
        (lane["backend"], lane["browser"]) for lane in enabled
    }


def index(lanes: Any, identity: str) -> int:
    return next(
        at for at, lane in enumerate(lanes["lanes"]) if lane["identity"] == identity
    )


def duplicate(lanes: Any) -> None:
    lanes["lanes"][2]["identity"] = lanes["lanes"][0]["identity"]


def repeated_shard(lanes: Any) -> None:
    # A pair whose shards repeat one index.
    shard = lanes["lanes"][index(lanes, "sqlite-chromium-3-of-3")]
    shard["shardIndex"] = 1
    shard["identity"] = "sqlite-chromium-1-of-3"


def missing_shard(lanes: Any) -> None:
    lanes["lanes"][index(lanes, "sqlite-chromium-3-of-3")]["enabled"] = False


def ungated(lanes: Any) -> None:
    # A lane's browser selects its one project, so it must be a gated browser.
    shard = lanes["lanes"][index(lanes, "sqlite-chromium-3-of-3")]
    shard["browser"] = "webkit"
    shard["identity"] = "sqlite-webkit-3-of-3"


def unsplit(lanes: Any) -> None:
    lanes["lanes"][index(lanes, "sqlite-chromium-unsplit")]["enabled"] = True


def malformed_weights(lanes: Any) -> None:
    # Shard weights must be a well-formed string, one weight per shard, for
    # exactly the gated browsers.
    lanes["shardWeights"]["firefox"] = "34/22/22/22"


def miscounted_weights(lanes: Any) -> None:
    # A browser's weight count must match its lanes' shard count.
    lanes["shardWeights"]["firefox"] = "34:33:33"


def foreign_count(lanes: Any) -> None:
    # A lane's shard count must be its own browser's, not another browser's.
    shard = lanes["lanes"][index(lanes, "sqlite-chromium-3-of-3")]
    shard["shardCount"] = 5
    shard["identity"] = "sqlite-chromium-3-of-5"


def missing_weights(lanes: Any) -> None:
    del lanes["shardWeights"]["chromium"]


def colliding_seed(lanes: Any) -> None:
    # Seeds that expand to the same trace id as another lane's.
    lanes["lanes"][1]["traceSeed"] = "11"


def zero_seed(lanes: Any) -> None:
    # A seed that expands to the invalid all-zero trace id.
    lanes["lanes"][0]["traceSeed"] = "00"


@pytest.mark.parametrize(
    "breakage",
    [
        duplicate,
        repeated_shard,
        missing_shard,
        ungated,
        unsplit,
        malformed_weights,
        miscounted_weights,
        foreign_count,
        missing_weights,
        colliding_seed,
        zero_seed,
    ],
)
def test_the_projection_rejects_a_malformed_catalog(breakage: Any) -> None:
    broken = lane_catalog()
    breakage(broken)
    assert project(broken).returncode != 0, "malformed catalog accepted"
