//! Throwaway PostgreSQL 16 cluster for the instrumented test suite: an `initdb`
//! cluster on a private TCP port
//! with durability disabled (it is discarded after the run, so crash-safety is
//! traded for speed), the `jaunder` app role/database created, and the cluster
//! torn down on every exit path — normal return, panic, or SIGINT/SIGTERM.

use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

pub(crate) const HOST: &str = "127.0.0.1";
const START_ATTEMPTS: usize = 8;

const BOOTSTRAP_SQL: &str =
    "CREATE ROLE jaunder LOGIN CREATEDB;\nCREATE DATABASE jaunder OWNER jaunder;\n";

/// Connection endpoints handed to the wrapped command.
pub struct PgEnv {
    pub test_url: String,
    pub bootstrap_url: String,
}

fn app_url(host: &str, port: u16) -> String {
    format!("postgres://jaunder@{host}:{port}/jaunder")
}

fn bootstrap_url(host: &str, port: u16) -> String {
    format!("postgres://postgres@{host}:{port}/postgres")
}

fn initdb_args(pgdata: &Path) -> Vec<String> {
    vec![
        "-D".into(),
        pgdata.display().to_string(),
        "-U".into(),
        "postgres".into(),
        "-A".into(),
        "trust".into(),
        "--no-sync".into(),
    ]
}

/// `-c k=v` pairs for `pg_ctl -o`; durability disabled (the cluster is discarded).
fn server_settings(host: &str, port: u16, pgdata: &Path) -> Vec<String> {
    [
        format!("listen_addresses={host}"),
        format!("port={port}"),
        format!("unix_socket_directories={}", pgdata.display()),
        "max_connections=200".to_string(),
        "fsync=off".to_string(),
        "full_page_writes=off".to_string(),
        "synchronous_commit=off".to_string(),
    ]
    .into_iter()
    .flat_map(|kv| ["-c".to_string(), kv])
    .collect()
}

fn psql_args(host: &str, port: u16) -> Vec<String> {
    vec![
        "-h".into(),
        host.into(),
        "-p".into(),
        port.to_string(),
        "-U".into(),
        "postgres".into(),
        "-d".into(),
        "postgres".into(),
        "-v".into(),
        "ON_ERROR_STOP=1".into(),
    ]
}

fn choose_free_port(host: &str) -> Result<u16> {
    choose_free_port_with(host, |host| {
        TcpListener::bind((host, 0)).with_context(|| {
            format!("binding temporary loopback listener for ephemeral PostgreSQL on {host}")
        })
    })
}

fn choose_free_port_with(
    host: &str,
    bind: impl FnOnce(&str) -> Result<TcpListener>,
) -> Result<u16> {
    let listener = bind(host)?;
    let port = listener
        .local_addr()
        .context("reading temporary PostgreSQL listener address")?
        .port();
    if port == 0 {
        bail!("temporary PostgreSQL listener returned port 0");
    }
    Ok(port)
}

/// Owns a running ephemeral cluster's data dir; tears it down exactly once.
struct Cluster {
    pgdata: PathBuf,
    torn_down: AtomicBool,
}

impl Cluster {
    /// Stop the server and delete the data dir. Idempotent so the Drop path and the
    /// signal-handler path cannot double-fire.
    fn teardown(&self) {
        self.teardown_with(
            |pgdata| {
                matches!(
                    Command::new("pg_ctl")
                        .args([
                            "-D",
                            &pgdata.display().to_string(),
                            "-m",
                            "immediate",
                            "stop",
                        ])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status(),
                    Ok(status) if status.success()
                )
            },
            |pgdata| std::fs::remove_dir_all(pgdata).is_ok(),
            &mut std::io::stderr(),
        );
    }

    fn teardown_with(
        &self,
        stop: impl FnOnce(&Path) -> bool,
        remove: impl FnOnce(&Path) -> bool,
        stderr: &mut impl Write,
    ) {
        if self.torn_down.swap(true, Ordering::SeqCst) {
            return;
        }
        report_cleanup_failure(!stop(&self.pgdata), "devtool.pg.stop", stderr);
        report_cleanup_failure(!remove(&self.pgdata), "devtool.pg.remove", stderr);
    }
}

fn report_cleanup_failure(failed: bool, key: &str, stderr: &mut impl Write) {
    if failed {
        let _ = writeln!(
            stderr,
            "devtool: warning: {key}: ignored failure during ephemeral PostgreSQL cleanup"
        );
    }
}
fn emulate_signal_with(sig: i32, emulate: impl FnOnce(i32) -> bool, stderr: &mut impl Write) {
    report_cleanup_failure(!emulate(sig), "devtool.pg.emulate_signal", stderr);
}

fn join_signal_thread_with(join: impl FnOnce() -> bool, stderr: &mut impl Write) {
    report_cleanup_failure(!join(), "devtool.pg.join_signal_thread", stderr);
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.teardown();
    }
}

/// Spawn `cmd`, suppressing its stdout, and fail on a non-zero exit.
fn run_checked(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("spawning {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed with {status}");
    }
    Ok(())
}

/// Create the `jaunder` role + database by piping the bootstrap SQL to `psql`.
fn bootstrap(host: &str, port: u16) -> Result<()> {
    use std::io::Write;

    let mut child = Command::new("psql")
        .args(psql_args(host, port))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .context("spawning psql for bootstrap")?;
    child
        .stdin
        .take()
        .context("psql stdin unavailable")?
        .write_all(BOOTSTRAP_SQL.as_bytes())
        .context("writing bootstrap SQL")?;
    let status = child.wait().context("waiting on psql")?;
    if !status.success() {
        bail!("psql bootstrap failed with {status}");
    }
    Ok(())
}

fn create_cluster_guard() -> Result<Arc<Cluster>> {
    // `keep()` hands ownership of the dir to us so `TempDir`'s own Drop won't delete
    // it while the server is still running; the `Cluster` guard removes it after the
    // server is stopped.
    let pgdata = tempfile::Builder::new()
        .prefix("jaunder-pg.")
        .tempdir()
        .context("creating PGDATA temp dir")?
        .keep();
    Ok(Arc::new(Cluster {
        pgdata,
        torn_down: AtomicBool::new(false),
    }))
}

fn start_cluster_on_port(port: u16) -> Result<Arc<Cluster>> {
    let cluster = create_cluster_guard()?;
    let startup = (|| {
        run_checked(Command::new("initdb").args(initdb_args(&cluster.pgdata)))?;
        let settings = server_settings(HOST, port, &cluster.pgdata).join(" ");
        run_checked(Command::new("pg_ctl").args([
            "-D",
            &cluster.pgdata.display().to_string(),
            "-w",
            "start",
            "-o",
            &settings,
        ]))?;
        bootstrap(HOST, port)?;
        Ok(())
    })();
    if let Err(error) = startup {
        cluster.teardown();
        return Err(error);
    }
    Ok(cluster)
}

fn retry_start<T>(
    attempts: usize,
    mut choose_port: impl FnMut() -> Result<u16>,
    mut start: impl FnMut(u16) -> Result<T>,
) -> Result<(T, u16)> {
    let mut last_error = None;
    for _ in 0..attempts {
        let port = choose_port()?;
        match start(port) {
            Ok(cluster) => return Ok((cluster, port)),
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) => Err(error).context("starting ephemeral PostgreSQL after retrying ports"),
        None => bail!("starting ephemeral PostgreSQL requires at least one attempt"),
    }
}

/// Boot a throwaway PostgreSQL 16 cluster, run `body` with its endpoints, and tear
/// down on every exit path (normal return, panic, or SIGINT/SIGTERM).
pub fn with_ephemeral<T>(body: impl FnOnce(&PgEnv) -> Result<T>) -> Result<T> {
    let (cluster, port) = retry_start(
        START_ATTEMPTS,
        || choose_free_port(HOST),
        start_cluster_on_port,
    )?;

    // Parity with the bash `trap cleanup INT TERM`: a dedicated thread tears the
    // cluster down on signal, then emulates the default disposition so the process
    // still dies with the right status. The thread holds only a `Weak` ref so
    // `cluster` stays the *sole* strong owner — that way an unwinding panic in
    // `body` drops the last strong Arc and fires `Cluster::drop`, rather than the
    // thread's clone pinning the refcount and leaking a running server. The Drop
    // guard thus covers normal return AND panic; the signal path covers INT/TERM.
    let mut signals = Signals::new([SIGINT, SIGTERM]).context("installing signal handler")?;
    let sig_cluster = Arc::downgrade(&cluster);
    let handle = signals.handle();
    let joiner = std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            if let Some(c) = sig_cluster.upgrade() {
                c.teardown();
            }
            emulate_signal_with(
                sig,
                |sig| signal_hook::low_level::emulate_default_handler(sig).is_ok(),
                &mut std::io::stderr(),
            );
        }
    });

    let env = PgEnv {
        test_url: app_url(HOST, port),
        bootstrap_url: bootstrap_url(HOST, port),
    };
    let result = body(&env);

    handle.close(); // unblock the signal thread on the normal path
    join_signal_thread_with(|| joiner.join().is_ok(), &mut std::io::stderr());
    cluster.teardown();
    result
}

/// CLI entry: run `cmd` with the ephemeral cluster's env, propagating its exit code.
pub fn run_command(cmd: &[String]) -> Result<()> {
    let code = with_ephemeral(|env| {
        let status = Command::new(&cmd[0])
            .args(&cmd[1..])
            .env("JAUNDER_PG_TEST_URL", &env.test_url)
            .env("JAUNDER_PG_BOOTSTRAP_TEST_URL", &env.bootstrap_url)
            .status()
            .with_context(|| format!("spawning {cmd:?}"))?;
        Ok(status.code().unwrap_or(1))
    })?;
    // `with_ephemeral` has already torn the cluster down by here, so exiting (which
    // skips destructors) leaks nothing.
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn urls_thread_selected_port() {
        assert_eq!(
            app_url(HOST, 15432),
            "postgres://jaunder@127.0.0.1:15432/jaunder"
        );
        assert_eq!(
            bootstrap_url(HOST, 25432),
            "postgres://postgres@127.0.0.1:25432/postgres"
        );
    }

    #[test]
    fn initdb_args_trust_no_sync() {
        let a = initdb_args(&PathBuf::from("/tmp/pg"));
        assert_eq!(
            a,
            [
                "-D",
                "/tmp/pg",
                "-U",
                "postgres",
                "-A",
                "trust",
                "--no-sync"
            ]
        );
    }

    #[test]
    fn server_settings_disable_durability() {
        let s = server_settings(HOST, 54329, &PathBuf::from("/tmp/pg")).join(" ");
        assert!(s.contains("-c listen_addresses=127.0.0.1"));
        assert!(s.contains("-c port=54329"));
        assert!(s.contains("-c unix_socket_directories=/tmp/pg"));
        assert!(s.contains("-c max_connections=200"));
        assert!(s.contains("-c fsync=off"));
        assert!(s.contains("-c full_page_writes=off"));
        assert!(s.contains("-c synchronous_commit=off"));
    }

    #[test]
    fn psql_args_stop_on_error() {
        let a = psql_args(HOST, 54329);
        assert_eq!(
            a,
            [
                "-h",
                "127.0.0.1",
                "-p",
                "54329",
                "-U",
                "postgres",
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1"
            ]
        );
    }

    #[test]
    fn retry_start_uses_a_new_port_after_failure() {
        let mut ports = vec![15432, 25432].into_iter();
        let mut attempted = Vec::new();

        let (cluster, port) = retry_start(
            2,
            || Ok(ports.next().unwrap()),
            |port| {
                attempted.push(port);
                if port == 15432 {
                    anyhow::bail!("simulated bind race");
                }
                Ok("cluster")
            },
        )
        .unwrap();

        assert_eq!(cluster, "cluster");
        assert_eq!(port, 25432);
        assert_eq!(attempted, [15432, 25432]);
    }

    #[test]
    fn bootstrap_sql_creates_role_and_db() {
        assert!(BOOTSTRAP_SQL.contains("CREATE ROLE jaunder LOGIN CREATEDB;"));
        assert!(BOOTSTRAP_SQL.contains("CREATE DATABASE jaunder OWNER jaunder;"));
    }

    #[test]
    fn ancillary_warning_pg_cleanup_preserves_primary_result() {
        let primary: anyhow::Result<i32> = Ok(17);
        let cluster = Cluster {
            pgdata: PathBuf::from("sensitive"),
            torn_down: AtomicBool::new(false),
        };
        let mut stderr = Vec::new();
        cluster.teardown_with(|_| false, |_| false, &mut stderr);
        emulate_signal_with(SIGTERM, |_| false, &mut stderr);
        join_signal_thread_with(|| false, &mut stderr);
        assert!(matches!(primary, Ok(17)));
        let warning = String::from_utf8(stderr).unwrap();
        for key in [
            "devtool.pg.stop",
            "devtool.pg.remove",
            "devtool.pg.join_signal_thread",
            "devtool.pg.emulate_signal",
        ] {
            assert_eq!(warning.matches(key).count(), 1);
        }
        assert_eq!(warning.lines().count(), 4);
        assert!(!warning.contains("sensitive"));
    }
}
