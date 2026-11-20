"""The tools, flags and environment changes the workflows share.

Each is spelled once here, so every workflow runs a tool the same way.
"""

from dataclasses import dataclass

# Where tools-workspace binaries are built, under the checkout root.
TOOLS_TARGET = "tools/target"


@dataclass(frozen=True)
class Tool:
    """A tools-workspace binary built from this checkout's source.

    `steps.tool` builds it as its own step, then the caller runs it directly,
    so the tool's step times the tool and never the compiler.
    """

    package: str
    binary: str
    features: tuple[str, ...] = ()

    def build(self) -> list[str]:
        return [
            "cargo",
            "build",
            "--locked",
            "--quiet",
            "--manifest-path",
            "tools/Cargo.toml",
            "--target-dir",
            TOOLS_TARGET,
            "-p",
            self.package,
            *(["--features", ",".join(self.features)] if self.features else []),
            "--bin",
            self.binary,
        ]

    @property
    def path(self) -> str:
        """The built binary, relative to the checkout root."""
        return f"{TOOLS_TARGET}/debug/{self.binary}"


def repository_policy(binary: str) -> Tool:
    return Tool("repository-policy", binary)


def host_proof(binary: str) -> Tool:
    return Tool("host-proof", binary)


SOURCE_CONTRACT = repository_policy("source-contracts")
SCOPED_LEPTOSFMT = repository_policy("scoped-leptosfmt")
COVERAGE_CHECK = Tool("coverage", "coverage-check", ("host",))
PERF_DATA = Tool("performance", "perf-data")
PR_OBSERVER = Tool("pr-observer", "pr-observer")
SYSTEM_ARTIFACTS = Tool("system-artifacts", "system-artifacts")
