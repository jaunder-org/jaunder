"""Fixed mutation actions and explicit operational error exits."""

import argparse
from pathlib import Path
import sys

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
        # Direct Nix/Just selection must not bypass the Rust delegate's admission.
        # The predicate consumes repo-relative paths even when invoked below root.
        repo = Path(git.run(Path.cwd(), "rev-parse", "--show-toplevel").decode("utf-8").strip())
        admit_checkout(repo)
        if arguments.action == "controller":
            print(controller.execute(repo).detail)
        else:
            action = promote if arguments.action == "promote" else sync_readme
            print(action(repo))
    except (OSError, UnicodeError, ValueError, RuntimeError) as error:
        print(f"adr-promoter: {error}", file=sys.stderr)
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
