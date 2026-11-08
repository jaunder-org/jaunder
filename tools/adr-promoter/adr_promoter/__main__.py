"""Fixed mutation actions and explicit operational error exits."""

import argparse
from pathlib import Path
import sys

from github import GithubException
from requests import RequestException

from . import controller, git
from .admission import admit_checkout
from .promotion import promote, sync_readme


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="adr-promoter",
        description="ADR promotion, README synchronization and serialized controller.",
    )
    parser.add_argument("action", choices=("controller", "promote", "sync-readme"))
    arguments = parser.parse_args()
    try:
        # One admission path consumes Nix's authoritative source predicate.
        repo = Path(git.run(Path.cwd(), "rev-parse", "--show-toplevel").decode("utf-8").strip())
        admit_checkout(repo)
        if arguments.action == "controller":
            print(controller.execute(repo).detail)
        else:
            action = promote if arguments.action == "promote" else sync_readme
            print(action(repo))
    except (OSError, UnicodeError, ValueError, RuntimeError, GithubException, RequestException) as error:
        print(f"adr-promoter: {error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
