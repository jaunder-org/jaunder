"""What a run's root span records about its machine, its CI job and its commit.

Each attribute is read where the machine exposes it and left out silently
where it does not: tracing never changes a run.
"""

import os
import subprocess
from collections.abc import Mapping
from pathlib import Path

BRANCH = "vcs.ref.head.name"
REVISION = "vcs.ref.head.revision"
BRANCH_PREFIX = "refs/heads/"
CPU_MODEL = "host.cpu.model.name"
NPROC = "jaunder.host.nproc"
CLOUD = "cloud.provider"
JOB_START = "jaunder.ci.job_start_ns"
# Set by setup-ci's first step: the wall clock, in nanoseconds, as the job began.
JOB_START_VARIABLE = "JAUNDER_CI_JOB_START_NS"

CPUINFO = Path("/proc/cpuinfo")
SYS_VENDOR = Path("/sys/class/dmi/id/sys_vendor")
# DMI system vendors of the clouds GitHub-hosted runners run on. A local
# Hyper-V machine also reports Microsoft's.
_CLOUDS = {"Amazon EC2": "aws", "Microsoft Corporation": "azure"}


def _read(path: Path) -> str | None:
    try:
        return path.read_text(errors="replace")
    except OSError:
        return None


def _cpu_model(cpuinfo: str) -> str | None:
    for line in cpuinfo.splitlines():
        key, _, value = line.partition(":")
        if key.strip() == "model name" and value.strip():
            return value.strip()
    return None


def attributes(
    environment: Mapping[str, str],
    cpuinfo: Path = CPUINFO,
    sys_vendor: Path = SYS_VENDOR,
) -> dict[str, str | int]:
    """The root span's runner and CI job attributes that can be read here."""
    found: dict[str, str | int] = {}
    model = _cpu_model(_read(cpuinfo) or "")
    if model is not None:
        found[CPU_MODEL] = model
    # The CPUs this process may run on, as `nproc` counts them.
    count = os.process_cpu_count()
    if count is not None:
        found[NPROC] = count
    cloud = _CLOUDS.get((_read(sys_vendor) or "").strip())
    if cloud is not None:
        found[CLOUD] = cloud
    start = environment.get(JOB_START_VARIABLE, "")
    if start.isascii() and start.isdigit():
        found[JOB_START] = int(start)
    return found


def head(root: Path) -> dict[str, str]:
    """The commit `root` has checked out, and its branch unless HEAD is detached.

    In a pre-commit hook this is the parent of the commit being made.
    """
    try:
        found = subprocess.run(
            ["git", "rev-parse", "HEAD", "--symbolic-full-name", "HEAD"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return {}
    lines = found.stdout.split()
    if found.returncode or len(lines) != 2:
        return {}
    revision, ref = lines
    checked_out = {REVISION: revision}
    if ref.startswith(BRANCH_PREFIX):
        checked_out[BRANCH] = ref.removeprefix(BRANCH_PREFIX)
    return checked_out
