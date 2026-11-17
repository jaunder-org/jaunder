"""The argv prefixes, flags and environment changes the workflows share.

Each is spelled once here, so every workflow runs a tool the same way.
"""

from jaunder_dev.steps import Environment

# A tools-workspace binary built from this checkout's source on every launch.
CARGO_RUN = (
    "cargo",
    "run",
    "--locked",
    "--quiet",
    "--manifest-path",
    "tools/Cargo.toml",
)
REPOSITORY_POLICY = (*CARGO_RUN, "-p", "repository-policy", "--bin")
SOURCE_CONTRACT = (*REPOSITORY_POLICY, "source-contracts", "--")
HOST_PROOF = (*CARGO_RUN, "-p", "host-proof", "--bin")
DOCTEST_CHECK = (
    *CARGO_RUN,
    "-p",
    "doctests",
    "--features",
    "host",
    "--bin",
    "doctest-check",
    "--",
)
COVERAGE_CHECK = (
    *CARGO_RUN,
    "-p",
    "coverage",
    "--features",
    "host",
    "--bin",
    "coverage-check",
    "--",
)
PERF_DATA = (*CARGO_RUN, "-p", "performance", "--bin", "perf-data", "--")

# Flake commands that must leave flake.lock exactly as committed.
NIX_LOCK_FLAGS = ("--no-update-lock-file", "--no-write-lock-file")

# Inherited Git overrides (as set inside hooks) would redirect discovery.
GIT_OVERRIDES = (
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
)
WITHOUT_GIT_OVERRIDES: Environment = dict.fromkeys(GIT_OVERRIDES)
