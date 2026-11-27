"""What every `dev` launch imports before it runs a command."""

import subprocess

from conftest import launch

# Third-party modules only some commands use; each launch would otherwise pay
# for them at import.
DEFERRED = ("yaml", "tabulate")


def test_starting_dev_leaves_command_specific_libraries_unloaded() -> None:
    probe = (
        "import dev.__main__; "
        f"print(' '.join(name for name in {DEFERRED!r} if name in sys.modules))"
    )
    done = subprocess.run(launch(probe), capture_output=True, check=True)
    assert done.stdout.decode().split() == []
