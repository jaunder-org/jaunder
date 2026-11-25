"""CI's documentation-only scope.

A pull request or merge-queue entry that changes Markdown alone, by the rule
the precommit's narrow route uses (`rawdiff.markdown_only`), runs only the
documentation checks. Anything this cannot establish runs everything.

CI's merge-queue verdict job runs it with the slim image's own Python, from a
sparse checkout and before any Nix shell exists, so it imports only the
standard library and `rawdiff`:

    PYTHONPATH=tools/dev python3 -m dev.scope --base <sha> >> "$GITHUB_OUTPUT"
"""

import argparse
import subprocess
import sys

from dev.rawdiff import Reason, markdown_only, raw_changes


def _git(*args: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["git", *args], capture_output=True, check=False)


def _reason(base: str) -> str:
    """Why the change from `base` to HEAD is, or is not, Markdown-only."""
    if not base:
        return "no-base"
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
    return markdown_only(changes)


def ci_scope(base: str) -> None:
    """Prints `docs_only=true|false` for `$GITHUB_OUTPUT`, and why on stderr.

    An empty `base` (a push) runs everything.
    """
    reason = _reason(base)
    docs_only = reason == Reason.ISOLATED_STAGED_MARKDOWN
    print(f"docs_only={'true' if docs_only else 'false'}", flush=True)
    print(f"ci-scope: docs_only={docs_only} reason={reason}", file=sys.stderr)


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
