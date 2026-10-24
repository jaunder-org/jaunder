//! Real command-lifetime PostgreSQL proof with checkout-local, long temporary paths.

use std::fs;
use std::path::Path;
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_devtool");
const CHILD_RECORD: &str = "JAUNDER_PG_LONG_PATH_RECORD";

fn sql(url: &str, query: &str) -> String {
    let output = Command::new("psql")
        .args([
            "-X",
            "-A",
            "-t",
            "-v",
            "ON_ERROR_STOP=1",
            "-d",
            url,
            "-c",
            query,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn pg_run_bootstraps_tcp_without_sockets_under_long_tmpdir() {
    if let Some(record) = std::env::var_os(CHILD_RECORD) {
        let app = std::env::var("JAUNDER_PG_TEST_URL").unwrap();
        let bootstrap = std::env::var("JAUNDER_PG_BOOTSTRAP_TEST_URL").unwrap();
        assert!(app.starts_with("postgres://jaunder@127.0.0.1:"));
        assert!(bootstrap.starts_with("postgres://postgres@127.0.0.1:"));
        assert_eq!(
            sql(
                &app,
                "SELECT current_user || '|' || current_database() || '|' || host(inet_server_addr())"
            ),
            "jaunder|jaunder|127.0.0.1"
        );
        assert_eq!(
            sql(
                &bootstrap,
                "SELECT current_user || '|' || current_database() || '|' || host(inet_server_addr())"
            ),
            "postgres|postgres|127.0.0.1"
        );
        assert_eq!(sql(&bootstrap, "SHOW unix_socket_directories"), "");
        let pgdata = sql(&bootstrap, "SHOW data_directory");
        let tmpdir = std::env::var_os("TMPDIR").unwrap();
        assert!(Path::new(&pgdata).starts_with(&tmpdir));
        assert!(
            pgdata.len() > 108,
            "PGDATA must exceed Unix socket pathname limits"
        );
        assert!(Path::new(&pgdata).join("postmaster.pid").exists());
        for entry in fs::read_dir(&pgdata).unwrap() {
            assert!(
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".s.PGSQL.")
            );
        }
        println!("TCP/bootstrap connected; unix_socket_directories empty; PGDATA={pgdata}");
        fs::write(record, pgdata).unwrap();
        return;
    }

    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let artifacts = checkout.join(".xtask/issue-1811");
    fs::create_dir_all(&artifacts).unwrap();
    let owned = tempfile::tempdir_in(artifacts).unwrap();
    let nested = owned
        .path()
        .join("deeply-nested-project-local-temporary-directory")
        .join("long-enough-to-exceed-the-unix-domain-socket-pathname-bound");
    fs::create_dir_all(&nested).unwrap();
    let record = owned.path().join("pgdata.txt");
    // Only the child inherits the override. Cluster owns PGDATA cleanup; this
    // test owns the routing parent and must retain it until the command returns.
    let output = Command::new(EXE)
        .args(["pg", "run", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "pg_run_bootstraps_tcp_without_sockets_under_long_tmpdir",
            "--nocapture",
        ])
        .env("TMPDIR", &nested)
        .env(CHILD_RECORD, &record)
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pgdata = fs::read_to_string(record).unwrap();
    assert!(
        !Path::new(&pgdata).exists(),
        "devtool must remove its PGDATA after stopping PostgreSQL"
    );
    assert!(
        nested.is_dir(),
        "cluster cleanup must not remove the caller-owned parent"
    );
    println!("Cluster removed PGDATA; caller-owned TMPDIR remains until TempDir drop");
}
