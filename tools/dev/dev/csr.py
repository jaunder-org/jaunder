"""The CSR bundle, built into target/site with the checkout's locked wasm
toolchain and Vite."""

import shutil
import tempfile
from pathlib import Path

from dev import steps
from dev.commands import SYSTEM_ARTIFACTS


def build(profile: str) -> None:
    checkout = Path.cwd()
    release = profile == "release"
    vite_tools = steps.nix_build(
        "csr-vite-node-modules", ".#csr-vite-node-modules", keep=False
    )
    steps.run(
        "csr-wasm",
        [
            "cargo",
            "build",
            "--locked",
            "-p",
            "csr",
            "--target",
            "wasm32-unknown-unknown",
            *(["--release"] if release else []),
        ],
        compiles=True,
    )
    shutil.rmtree(checkout / "target" / "site", ignore_errors=True)
    with tempfile.TemporaryDirectory() as temporary:
        work = Path(temporary)
        project = work / "project"
        system_artifacts = work / "system-artifacts"
        (project / "csr").mkdir(parents=True)
        (work / "input").mkdir()
        system_artifacts.mkdir()
        for name in ["package.json", "package-lock.json", "vite.config.js"]:
            shutil.copy(checkout / name, project)
        for name in ["index.html", "entry.js"]:
            shutil.copy(checkout / "csr" / name, project / "csr")
        (project / "node_modules").symlink_to(f"{vite_tools}/node_modules")
        generate = steps.tool(SYSTEM_ARTIFACTS)
        steps.run("system-artifacts", [*generate, "--out", str(system_artifacts)])
        steps.run(
            "csr-assets",
            [
                "bash",
                f"{checkout}/scripts/build-csr-assets.sh",
                f"target/wasm32-unknown-unknown/{profile}/csr.wasm",
                str(project),
                str(work / "input"),
                str(system_artifacts),
                str(work / "dist"),
            ],
            environment={"CSR_VITE_PUBLIC_DIR": f"{checkout}/public"},
        )
        (checkout / "target" / "site").mkdir(parents=True, exist_ok=True)
        steps.run(
            "csr-site", ["cp", "-a", f"{work}/dist/.", f"{checkout}/target/site/"]
        )
