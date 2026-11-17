"""`dev coverage` and `dev doctests`: real Git admission, then fail-fast steps.

Their full success populations are in test_gates.
"""

import subprocess

import pytest
from conftest import FAKE_NIX_STORE, Checkout, Workflow

CLEAN = ["git", "status", "--porcelain", "--untracked-files=all"]
NIX_BUILD = "nix build -L --no-update-lock-file --no-write-lock-file"
CARGO_RUN = "cargo run --locked --quiet --manifest-path tools/Cargo.toml"
STEPS = {
    "coverage": [
        f"{NIX_BUILD} --print-out-paths --out-link .xtask/gcroots/coverage .#checks.x86_64-linux.coverage",
        f"{NIX_BUILD} --print-out-paths --no-link .#checks.x86_64-linux.coverage-gate",
        (
            f"{CARGO_RUN} -p coverage --features host --bin coverage-check -- "
            "policy --out .xtask/gcroots/coverage"
        ),
    ],
    "doctests": [
        f"{NIX_BUILD} --print-out-paths --out-link .xtask/gcroots/doctests .#checks.x86_64-linux.doctests",
        f"{NIX_BUILD} --print-out-paths --no-link .#checks.x86_64-linux.doctests-gate",
    ],
}


@pytest.mark.parametrize(
    ("workflow_name", "failing"),
    [(name, index) for name, lines in STEPS.items() for index in range(len(lines))],
)
def test_workflows_stop_at_each_failing_boundary(
    workflow: Workflow, workflow_name: str, failing: int
) -> None:
    lines = STEPS[workflow_name]
    result = workflow.dev(workflow_name, failure=lines[failing])
    assert result.returncode == 17
    assert workflow.commands() == [
        CLEAN,
        *(line.split(" ") for line in lines[: failing + 1]),
    ]


@pytest.mark.parametrize("workflow_name", ["coverage", "doctests"])
@pytest.mark.parametrize("staged", [False, True])
def test_public_workflows_refuse_a_really_dirty_checkout(
    checkout: Checkout, workflow_name: str, staged: bool
) -> None:
    for tool in ["nix", "cargo"]:
        checkout.tool(tool, "")
    # `dev` writes its trace under .xtask, which Jaunder's checkout ignores.
    (checkout.root / ".git" / "info" / "exclude").write_text(".xtask/\n")
    (checkout.root / "dirty").write_text("changed")
    if staged:
        subprocess.run(["git", "-C", str(checkout.root), "add", "dirty"], check=True)
    result = checkout.dev(workflow_name)
    assert result.returncode == 1
    assert result.stderr.startswith(b"verify workflows require a clean checkout\n")
    assert checkout.order() == ""


@pytest.mark.parametrize(
    ("extra", "cache"), [({"NIX_VALID": "1"}, "reused"), ({}, "substituted")]
)
def test_producer_and_gate_record_where_their_outputs_came_from(
    checkout: Checkout, extra: dict[str, str], cache: str
) -> None:
    checkout.tool("nix", FAKE_NIX_STORE)
    (checkout.root / ".git" / "info" / "exclude").write_text(".xtask/\nbuilt\n")
    result = checkout.run(checkout.command("doctests"), checkout.environment(**extra))
    assert result.returncode == 0, result
    slices = checkout.merged()
    assert slices["doctests"]["args"]["jaunder.cache"] == cache
    # The fake store holds one output, which the producer step leaves valid.
    assert slices["doctests-gate"]["args"]["jaunder.cache"] == "reused"
    # The probes evaluate the flake as the build does, lock file untouched.
    assert checkout.order().splitlines()[0] == (
        "nix eval --raw --no-update-lock-file --no-write-lock-file "
        ".#checks.x86_64-linux.doctests.outPath"
    )
