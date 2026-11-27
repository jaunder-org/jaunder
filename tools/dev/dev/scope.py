"""CI's documentation-only and dependency scopes.

A pull request or merge-queue entry that changes Markdown alone, by the rule
the precommit's narrow route uses (`rawdiff.markdown_only`), runs only the
documentation checks. Anything this cannot establish runs everything.

The host lane checks RustSec advisories only for an entry that changes the
product workspace's dependencies, so a newly published advisory against an
unchanged dependency fails no unrelated work; the daily scan of `main`
(`.github/workflows/advisories.yml`) reports it instead.

CI's merge-queue verdict job runs it with the slim image's own Python, from a
sparse checkout and before any Nix shell exists, so it imports only the
standard library and `rawdiff`:

    PYTHONPATH=tools/dev python3 -m dev.scope --base <sha> >> "$GITHUB_OUTPUT"
"""

import argparse
import subprocess
import sys

from dev.rawdiff import Change, Reason, markdown_only, raw_changes

# The product workspace's dependency inputs that cargo-deny reads. `tools/` is a
# separate workspace cargo-deny does not check, so its changes do not count.
_PRODUCT_DEPENDENCY_FILES = ("Cargo.lock", "deny.toml")


def _git(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["git", *args], capture_output=True, check=False)


def _changes(base: str) -> list[Change] | str:
    """The changes from `base` to HEAD, or why they cannot be trusted."""
    # Depth 1 is enough: comparing two trees needs no history between them. A
    # failed fetch needs no check of its own: the diff then fails too, unless
    # the base is already here.
    _git("fetch", "--quiet", "--no-tags", "--depth=1", "origin", base)
    diff = _git("diff", "--raw", "-z", "--no-abbrev", "--no-renames", base, "HEAD")
    if diff.returncode:
        return "base-unavailable"
    uncertainty: set[str] = set()
    changes = raw_changes(diff.stdout, uncertainty)
    if uncertainty:
        return Reason.UNCERTAIN_STATUS
    return changes


def _product_dependency(path: str) -> bool:
    return path in _PRODUCT_DEPENDENCY_FILES or (
        path.rsplit("/", 1)[-1] == "Cargo.toml" and not path.startswith("tools/")
    )


def ci_scope(base: str) -> None:
    """Prints `docs_only=` and `dependencies=true|false` for `$GITHUB_OUTPUT`,
    and why on stderr.

    An empty `base` (a push) runs everything except advisories, which the daily
    scan owns for `main`. An unreadable diff counts as a dependency change.
    """
    if not base:
        reason, dependencies = "no-base", False
    else:
        changes = _changes(base)
        if isinstance(changes, str):
            reason, dependencies = changes, True
        else:
            reason = markdown_only(changes)
            dependencies = any(_product_dependency(c.path) for c in changes)
    docs_only = reason == Reason.ISOLATED_STAGED_MARKDOWN
    print(f"docs_only={'true' if docs_only else 'false'}", flush=True)
    print(f"dependencies={'true' if dependencies else 'false'}", flush=True)
    print(
        f"ci-scope: docs_only={docs_only} reason={reason} dependencies={dependencies}",
        file=sys.stderr,
    )


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        prog="python3 -m dev.scope", description=__doc__.splitlines()[0]
    )
    parser.add_argument(
        "--base", default="", help="The commit the change is compared with."
    )
    ci_scope(parser.parse_args(argv).base)


if __name__ == "__main__":
    main()
