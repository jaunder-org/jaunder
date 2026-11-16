"""pg-run <command> [args...]

Run a command against a throwaway PostgreSQL cluster reachable only over a
Unix socket, and remove the cluster however the command ends: normal exit,
failure, SIGINT or SIGTERM.

The command receives JAUNDER_PG_TEST_URL (role and database `jaunder`) and
JAUNDER_PG_BOOTSTRAP_TEST_URL (superuser `postgres`). Each cluster owns its
socket directory, so there is no TCP port to choose or race for. Durability is
off because the cluster is discarded after the run.
"""

import os
import shutil
import signal
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from dataclasses import dataclass

SERVER_SETTINGS = """
listen_addresses = ''
unix_socket_directories = '{sockdir}'
max_connections = 200
fsync = off
full_page_writes = off
synchronous_commit = off
"""

BOOTSTRAP_SQL = """
CREATE ROLE jaunder LOGIN CREATEDB;
CREATE DATABASE jaunder OWNER jaunder;
"""

QUIET = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}


class Interrupted(Exception):
    """A signal arrived during setup."""


class StartFailed(Exception):
    """The server did not come up; its log has been shown."""


@dataclass
class Signals:
    """Record SIGINT and SIGTERM, and pass them on to the command.

    The handler never interrupts the step in progress: setup checks between
    steps, so a half-started server is never deleted under itself. While the
    command runs, the signal is passed on and pg-run waits for the command to
    exit. Either way the cluster is torn down, then pg-run dies by the signal
    it received.
    """

    received: int | None = None
    child: subprocess.Popen | None = None

    def __enter__(self):
        signal.signal(signal.SIGINT, self.handle)
        signal.signal(signal.SIGTERM, self.handle)
        return self

    def __exit__(self, *_exc):
        # Runs after the cluster's teardown, which is nested inside.
        if self.received is not None:
            signal.signal(self.received, signal.SIG_DFL)
            os.kill(os.getpid(), self.received)

    def handle(self, signum, _frame):
        self.received = signum
        if self.child is not None:
            if not (signum == signal.SIGINT and terminal_sent_it()):
                self.child.send_signal(signum)

    def check(self):
        if self.received is not None:
            raise Interrupted

    def attach(self, child):
        self.child = child
        # The command did not exist yet when this signal arrived.
        if self.received is not None:
            child.send_signal(self.received)


def terminal_sent_it():
    """Whether a terminal Ctrl-C also reached the command directly.

    The command shares pg-run's process group. Forwarding a second SIGINT
    would turn a graceful cancel into a forced one (nextest kills its tests
    on a second signal).
    """
    try:
        tty = os.open("/dev/tty", os.O_RDONLY)
    except OSError:
        return False
    try:
        return os.tcgetpgrp(tty) == os.getpgrp()
    finally:
        os.close(tty)


def warn(action):
    # Cleanup problems never replace the command's result and never print
    # the data directory path.
    print(f"pg-run: warning: ignored failure to {action} the ephemeral "
          "PostgreSQL cluster", file=sys.stderr)


@dataclass
class Cluster:
    """The throwaway cluster: what exists so far, so teardown removes it."""

    pgdata: str | None = None
    sockdir: str | None = None
    started: bool = False

    def start(self, signals):
        """Create, start and bootstrap the cluster, checking for signals."""
        tmp = os.environ.get("TMPDIR", "/tmp")
        self.pgdata = tempfile.mkdtemp(prefix="jaunder-pg.", dir=tmp)
        # A socket path is limited to 107 bytes, which a checkout-local
        # TMPDIR inside a branch-named worktree can exceed, so the socket
        # gets its own short directory.
        runtime = os.environ.get("XDG_RUNTIME_DIR", tmp)
        self.sockdir = tempfile.mkdtemp(prefix="jaunder-pg.", dir=runtime)
        signals.check()

        subprocess.run(
            ["initdb", "-D", self.pgdata, "-U", "postgres", "-A", "trust",
             "--no-sync"],
            check=True, stdout=subprocess.DEVNULL)
        conf_path = os.path.join(self.pgdata, "postgresql.conf")
        with open(conf_path, "a") as conf:
            conf.write(SERVER_SETTINGS.format(sockdir=self.sockdir))
        signals.check()

        log = os.path.join(self.pgdata, "server.log")
        start = ["pg_ctl", "-D", self.pgdata, "-l", log, "-w", "start"]
        if subprocess.run(start, stdout=subprocess.DEVNULL).returncode != 0:
            # A timed-out start can leave a postmaster behind; it is stopped
            # without a warning. Teardown deletes the log, so show it now,
            # unless the start failed because of Ctrl-C.
            self.stop()
            signals.check()
            with open(log, errors="replace") as server_log:
                sys.stderr.write(server_log.read())
            raise StartFailed
        self.started = True
        signals.check()

        subprocess.run(
            ["psql", "-X", "-q", "-h", self.sockdir, "-U", "postgres",
             "-d", "postgres", "-v", "ON_ERROR_STOP=1"],
            input=BOOTSTRAP_SQL, text=True, check=True,
            stdout=subprocess.DEVNULL)
        signals.check()

    def urls(self):
        host = f"host={self.sockdir}"
        return {
            "JAUNDER_PG_TEST_URL":
                f"postgres://jaunder@localhost/jaunder?{host}",
            "JAUNDER_PG_BOOTSTRAP_TEST_URL":
                f"postgres://postgres@localhost/postgres?{host}",
        }

    def stop(self):
        command = ["pg_ctl", "-D", self.pgdata, "-m", "immediate", "stop"]
        return subprocess.run(command, **QUIET).returncode == 0

    def teardown(self):
        if self.started and not self.stop():
            warn("stop")
        removed = True
        for directory in filter(None, (self.pgdata, self.sockdir)):
            try:
                shutil.rmtree(directory)
            except FileNotFoundError:
                pass
            except OSError:
                removed = False
        if not removed:
            warn("remove")


@contextmanager
def throwaway_cluster(signals):
    """Yield the cluster's URLs, and remove whatever setup created."""
    cluster = Cluster()
    try:
        cluster.start(signals)
        yield cluster.urls()
    finally:
        cluster.teardown()


def run_command(command, signals):
    """Run the command, returning its exit status the way a shell would."""
    try:
        child = subprocess.Popen(command)
    except FileNotFoundError:
        print(f"pg-run: {command[0]}: command not found", file=sys.stderr)
        return 127
    except PermissionError:
        print(f"pg-run: {command[0]}: permission denied", file=sys.stderr)
        return 126
    signals.attach(child)
    status = child.wait()
    return 128 - status if status < 0 else status


def main(args):
    if not args:
        print("usage: pg-run <command> [args...]", file=sys.stderr)
        return 2

    try:
        with Signals() as signals, throwaway_cluster(signals) as urls:
            os.environ.update(urls)
            return run_command(args, signals)
    except (Interrupted, StartFailed, subprocess.CalledProcessError):
        # Setup failed or was interrupted. A failing tool has already
        # explained itself on stderr.
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
