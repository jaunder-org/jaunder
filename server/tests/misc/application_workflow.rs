//! Actual production-feature application workflows, separate from libtest.

use std::{path::Path, process::Command, rc::Rc, time::Duration};

use anyhow::Context as _;
use jaunder::cli::StorageArgs;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{
    Backend, backends, backends_matrix, recorded_postgres_url, sqlite_url,
};
use tempfile::TempDir;

use super::{
    application_workflow_owner::{ApplicationOwner, workflow_deadline},
    commands::storage_args,
};

fn storage_command<R>(owner: &ApplicationOwner<R>, name: &str, args: &StorageArgs) -> Command {
    let mut command = owner.command(name);
    command
        .arg("--storage-path")
        .arg(&args.storage_path)
        .arg("--db")
        .arg(args.db.expose_url());
    command
}

async fn observe_http(
    url: &str,
    deadline: std::time::Instant,
) -> anyhow::Result<reqwest::Response> {
    tokio::time::timeout_at(deadline.into(), reqwest::Client::new().get(url).send())
        .await
        .context("HTTP observation workflow deadline")?
        .context("HTTP request observation")
}

fn require_success<R>(owner: &mut ApplicationOwner<R>, command: Command) {
    let status = owner.run(command).expect("owned CLI completion");
    assert!(
        status.success(),
        "actual application command failed: {}",
        owner.stderr()
    );
}

async fn runtime_ready<R>(owner: &mut ApplicationOwner<R>, runtime: &Path) -> serde_json::Value {
    let pid = owner.pid();
    tokio::time::timeout_at(owner.deadline().into(), async {
        loop {
            let status = owner.observe_exit().expect("owned child observation");
            let diagnostics = owner.stderr();
            assert!(
                status.is_none(),
                "application exited before readiness: {diagnostics}"
            );
            if let Ok(bytes) = std::fs::read(runtime) {
                let info: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("atomic runtime identity schema");
                assert_eq!(info["pid"].as_u64(), Some(u64::from(pid)));
                assert!(info["start_time"].as_u64().is_some_and(|time| time > 0));
                if info["port"].as_u64().is_some_and(|port| port > 0) {
                    return info;
                }
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("bounded actual application readiness")
}

#[derive(Clone, Copy)]
enum FailureBoundary {
    InvalidStorageStartup,
    ImmediatePostSpawnAbandonment,
    StalledHttpObservation,
    ShutdownIdentityRefusal,
}

#[apply(backends_matrix)]
#[case::invalid_storage(FailureBoundary::InvalidStorageStartup)]
#[case::abandoned(FailureBoundary::ImmediatePostSpawnAbandonment)]
#[case::refused(FailureBoundary::ShutdownIdentityRefusal)]
#[case::stalled_http(FailureBoundary::StalledHttpObservation)]
#[tokio::test]
async fn actual_application_failure_cleanup_reaps_before_backend_resources_drop(
    backend: Backend,
    #[case] boundary: FailureBoundary,
) {
    // Shared-harness provisioning precedes application ownership. Do not cancel
    // backend creation before the harness has returned its teardown guard.
    let env = Rc::new(backend.setup().await);
    let deadline = workflow_deadline();
    let db = match backend {
        Backend::Sqlite => sqlite_url(&env.base),
        Backend::Postgres => recorded_postgres_url(&env.base)
            .parse()
            .expect("owned PostgreSQL URL"),
    };
    let args = StorageArgs {
        storage_path: env.base.path().join("storage"),
        db,
    };
    let base = env.base.path().to_owned();
    let mut owner = ApplicationOwner::new(env.clone(), &base, deadline);
    if matches!(boundary, FailureBoundary::InvalidStorageStartup) {
        std::fs::write(&args.storage_path, b"not a directory")
            .expect("actual invalid storage path");
    } else {
        let command = storage_command(&owner, "init", &args);
        require_success(&mut owner, command);
    }
    let mut command = storage_command(&owner, "serve", &args);
    command.args(["--bind", "127.0.0.1:0"]);
    owner.spawn(command).expect("owned failure-path server");
    let raw_pid = i32::try_from(owner.pid()).expect("owned PID fits platform");
    let pid = rustix::process::Pid::from_raw(raw_pid).expect("owned positive PID");
    let runtime = args.storage_path.join("runtime.json");
    match boundary {
        FailureBoundary::InvalidStorageStartup => {
            assert!(
                !owner
                    .wait()
                    .expect("confirmed failed startup wait")
                    .success()
            );
            assert!(
                !runtime.exists(),
                "failed startup cannot publish ready identity"
            );
        }
        FailureBoundary::ImmediatePostSpawnAbandonment => {
            // Immediate caller abandonment without runtime discovery; no claim
            // about the child's scheduling order or readiness state.
        }
        FailureBoundary::StalledHttpObservation => {
            runtime_ready(&mut owner, &runtime).await;
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("controlled stalled HTTP observer");
            let url = format!(
                "http://{}/",
                listener.local_addr().expect("stalled observer address")
            );
            let observation = observe_http(&url, deadline);
            tokio::pin!(observation);
            let (stream, _) = tokio::select! {
                accepted = listener.accept() => accepted.expect("real HTTP connection accepted"),
                result = &mut observation => panic!("HTTP observation ended before listener admission: {result:?}"),
            };
            // Hold an accepted socket without responding: a real stalled
            // observer, not an injected result or production test knob.
            let failure = observation
                .await
                .expect_err("stalled HTTP observation deadline");
            assert!(
                failure.is::<tokio::time::error::Elapsed>(),
                "actual deadline failure, not a request error"
            );
            drop(stream);
        }
        FailureBoundary::ShutdownIdentityRefusal => {
            runtime_ready(&mut owner, &runtime).await;
            std::fs::write(&runtime, b"invalid-json").expect("controlled runtime identity refusal");
            let diagnostics = base.join("refused-shutdown");
            std::fs::create_dir(&diagnostics).expect("private refusal diagnostics");
            let mut shutdown = ApplicationOwner::new(env.clone(), &diagnostics, deadline);
            let command = storage_command(&shutdown, "shut-down", &args);
            assert!(
                !shutdown
                    .run(command)
                    .expect("confirmed shutdown refusal wait")
                    .success()
            );
        }
    }
    // From this point the owner is the last backend/temp lifetime holder. An
    // uncertain settlement must retain it rather than deleting live resources.
    drop(env);
    assert!(
        base.exists(),
        "backend resources retained while child is owned"
    );
    drop(owner);
    assert_eq!(
        rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG).err(),
        Some(rustix::io::Errno::CHILD),
        "cleanup already performed the owned wait, not just TERM delivery"
    );
    assert!(
        !base.exists(),
        "confirmed child settlement permits resource teardown"
    );
}

#[apply(backends)]
#[tokio::test]
async fn actual_application_initializes_fresh_command_storage(#[case] backend: Backend) {
    // Provision the empty backend through the existing harness before starting
    // the application workflow; no child or workflow-owned database work exists yet.
    let base = TempDir::new().expect("owned bare command resources");
    let (args, postgres) = storage_args(backend, &base).await;
    let deadline = workflow_deadline();
    assert!(
        !args.storage_path.exists(),
        "init starts with no storage structure"
    );
    // This existing command harness creates an EMPTY PostgreSQL database, not
    // the migrated/seeding TestEnv template. SQLite has no database file yet.
    let resources = Rc::new((base, postgres));
    let mut owner = ApplicationOwner::new(resources.clone(), resources.0.path(), deadline);
    let command = storage_command(&owner, "init", &args);
    require_success(&mut owner, command);
    assert!(args.storage_path.join("media").is_dir());
    assert!(args.storage_path.join("backups").is_dir());
    let factory = tokio::time::timeout_at(
        deadline.into(),
        storage::open_existing_database(&args.db, &storage::StorageRuntimeConfig::default()),
    )
    .await
    .expect("fresh-init database observation within workflow deadline")
    .expect("actual CLI initialized selected backend");
    drop(factory);
    drop(owner);
}

#[apply(backends)]
#[tokio::test]
async fn actual_application_creates_production_password_and_serves_owned_instance(
    #[case] backend: Backend,
) {
    // Shared-harness provisioning is outside the application workflow. Its
    // completed guard then remains owned through every child and observation.
    let env = Rc::new(backend.setup().await);
    let deadline = workflow_deadline();
    let db = match backend {
        Backend::Sqlite => sqlite_url(&env.base),
        Backend::Postgres => recorded_postgres_url(&env.base)
            .parse()
            .expect("owned PostgreSQL URL"),
    };
    let args = StorageArgs {
        storage_path: env.base.path().join("storage"),
        db,
    };
    let mut owner = ApplicationOwner::new(env.clone(), env.base.path(), deadline);
    // The shared backend is already provisioned; init here prepares its fresh
    // FILESYSTEM structure. The separate test above certifies true empty init.
    let command = storage_command(&owner, "init", &args);
    require_success(&mut owner, command);
    let mut command = storage_command(&owner, "user-create", &args);
    command.args([
        "--username",
        "native-workflow",
        "--password",
        "synthetic-development-password",
    ]);
    require_success(&mut owner, command);
    let hash: String = tokio::time::timeout_at(deadline.into(), async {
        storage::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query_scalar("SELECT password_hash FROM users WHERE username = $1")
                .bind("native-workflow")
                .fetch_one(pool)
                .await
                .expect("stored actual CLI password")
        })
    })
    .await
    .expect("stored PHC observation within workflow deadline");
    let phc = hash.split('$').collect::<Vec<_>>();
    assert_eq!(phc.len(), 6, "complete stored PHC");
    assert_eq!(phc[1], "argon2id");
    assert_eq!(phc[2], "v=19");
    assert_eq!(
        phc[3], "m=19456,t=2,p=1",
        "production, not cheap test parameters"
    );
    assert!(
        !phc[4].is_empty() && !phc[5].is_empty(),
        "stored salt and digest"
    );
    let mut command = storage_command(&owner, "serve", &args);
    command.args(["--bind", "127.0.0.1:0"]);
    owner.spawn(command).expect("owned actual server");
    let runtime = args.storage_path.join("runtime.json");
    let info = runtime_ready(&mut owner, &runtime).await;
    let url = format!(
        "http://127.0.0.1:{}/",
        info["port"].as_u64().expect("runtime port")
    );
    let response = observe_http(&url, deadline)
        .await
        .expect("bounded actual served HTTP");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let diagnostics = env.base.path().join("shutdown-observer");
    std::fs::create_dir(&diagnostics).expect("private shutdown diagnostics");
    // A separate short CLI child retains the SAME backend ownership if its
    // settlement becomes uncertain; Rc is local lifetime ownership, not DI.
    let mut shutdown = ApplicationOwner::new(env.clone(), &diagnostics, deadline);
    let command = storage_command(&shutdown, "shut-down", &args);
    require_success(&mut shutdown, command);
    assert!(owner.wait().expect("confirmed server wait").success());
    assert!(
        !runtime.exists(),
        "identity relinquished before resource teardown"
    );
}
