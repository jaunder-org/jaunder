"""The tools, flags and environment changes the workflows share.

Each is spelled once here, so every workflow runs a tool the same way.
"""

from dataclasses import dataclass


@dataclass(frozen=True)
class Tool:
    """A host tool binary, built by this checkout's flake package `package`.

    `steps.tool` builds the package as its own step, then the caller runs the
    binary directly, so the tool's step times the tool and never the build.
    """

    package: str
    binary: str


def repository_policy(binary: str) -> Tool:
    return Tool("repository-policy", binary)


def host_proof(binary: str) -> Tool:
    return Tool("host-proof", binary)


SOURCE_CONTRACT = repository_policy("source-contracts")
SCOPED_LEPTOSFMT = repository_policy("scoped-leptosfmt")
E2E_EVIDENCE = host_proof("e2e-evidence")
COVERAGE_CHECK = Tool("coverage-check", "coverage-check")
SYSTEM_ARTIFACTS = Tool("system-artifacts", "system-artifacts")
