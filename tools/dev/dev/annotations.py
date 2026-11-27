"""GitHub Actions annotations that `dev` writes and `dev pr-watch` reads back.

A step writes one as a workflow command on its stdout; GitHub attaches it to the
job's check run, where the observer finds it by its title.
"""

import os

# Marks a lane's duration pressure
# (docs/adr/0272-e2e-duration-pressure-is-a-signal.md).
DURATION_PRESSURE = "Duration pressure"


def in_github_actions() -> bool:
    return os.environ.get("GITHUB_ACTIONS") == "true"


def warning(title: str, message: str) -> None:
    """Writes a warning annotation; `title` must need no escaping."""
    escaped = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::warning title={title}::{escaped}", flush=True)
