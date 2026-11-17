"""The argv prefixes, flags and environment changes the workflows share.

Each is spelled once here, so every workflow runs a tool the same way.
"""

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
