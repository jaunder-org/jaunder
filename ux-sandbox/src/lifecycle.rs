//! Sandbox-specific transitions; process ownership stays in the direct async session owner.
use crate::{
    cli::{Cli, Profile},
    preparation::{self, Artifacts, Prepared},
    session::{self, Signals},
    workspace::{self, Lock, Named, Server, State},
};
use anyhow::{Context, ensure};
use std::path::Path;
use tokio::process::Command;

pub async fn run(cli: Cli, signals: &mut Signals) -> anyhow::Result<i32> {
    if let Some(name) = cli.name.as_ref() {
        let mut paths = Named::open(&std::env::current_dir()?.canonicalize()?, name)?;
        if !cli.command.is_empty() {
            let artifacts = match preparation::prepare(false, signals).await? {
                Prepared::Ready(artifacts) => artifacts,
                Prepared::Exited(code) => return Ok(code),
            };
            let _guards = admit(&paths, &artifacts.digest, signals).await?;
            let mut command = app_command(&artifacts.jaunder, &paths.current);
            command.args(&cli.command);
            return session::run(command, None, signals).await;
        }
        // Never wait for a lease while holding workspace ownership.
        let lease =
            Lock::try_exclusive(&paths.lease)?.context("sandbox already has a managed server")?;
        let exclusive = Lock::exclusive(&paths.ownership, signals).await?;
        paths.recover()?;
        paths.remove_metadata()?;
        let profile = paths.profile(cli.profile, cli.reset)?;
        let artifacts = match preparation::prepare(true, signals).await? {
            Prepared::Ready(artifacts) => artifacts,
            Prepared::Exited(code) => return Ok(code),
        };
        let code = prepare_named(&mut paths, &artifacts, profile, cli.reset, signals).await?;
        if code != 0 {
            return Ok(code);
        }
        let runtime = paths.current.join("runtime.json");
        let mut command = app_command(&artifacts.jaunder, &paths.current);
        command.arg("serve");
        let mut server = Server::new(paths, lease, exclusive, artifacts.digest);
        let result = session::named_server(command, &runtime, signals, &mut server).await;
        let cleanup = server.cleanup().await;
        completed(result, cleanup)
    } else {
        let artifacts = match preparation::prepare(true, signals).await? {
            Prepared::Ready(artifacts) => artifacts,
            Prepared::Exited(code) => return Ok(code),
        };
        let disposable = tempfile::Builder::new()
            .prefix("ux-sandbox-")
            .tempdir()?
            .keep();
        let storage = disposable.join("storage");
        eprintln!("Disposable storage: {}", storage.display());
        let result = async {
            let code = initialize(
                &storage,
                &artifacts,
                cli.profile.unwrap_or_default(),
                signals,
            )
            .await?;
            if code != 0 {
                return Ok(code);
            }
            let mut command = app_command(&artifacts.jaunder, &storage);
            command.arg("serve");
            session::run(command, Some(&storage.join("runtime.json")), signals).await
        }
        .await;
        // Session errors return only after explicit terminal confirmation.
        completed(
            result,
            std::fs::remove_dir_all(&disposable).context("removing reaped disposable sandbox"),
        )
    }
}

fn completed(result: anyhow::Result<i32>, cleanup: anyhow::Result<()>) -> anyhow::Result<i32> {
    match (result, cleanup) {
        (Err(error), cleanup) => {
            if let Err(cleanup) = cleanup {
                eprintln!("sandbox cleanup also failed: {cleanup:#}");
            }
            Err(error)
        }
        (Ok(code), cleanup) => {
            cleanup?;
            Ok(code)
        }
    }
}

async fn prepare_named(
    paths: &mut Named,
    artifacts: &Artifacts,
    profile: Profile,
    reset: bool,
    signals: &mut Signals,
) -> anyhow::Result<i32> {
    paths.validate()?;
    let _old_runtime = if paths.current.exists() {
        Some(Lock::runtime(&paths.current)?)
    } else {
        None
    };
    if paths.current.exists() && !reset {
        return Ok(0);
    }
    let result = initialize(&paths.new, artifacts, profile, signals).await;
    match result {
        Ok(0) => {
            let _new_runtime = Lock::runtime(&paths.new)?;
            paths.publish()?;
            Ok(0)
        }
        result => {
            let cleanup = if paths.new.exists() {
                std::fs::remove_dir_all(&paths.new).context("discarding failed sandbox preparation")
            } else {
                Ok(())
            };
            completed(result, cleanup)
        }
    }
}

async fn initialize(
    storage: &Path,
    artifacts: &Artifacts,
    profile: Profile,
    signals: &mut Signals,
) -> anyhow::Result<i32> {
    let mut init = app_command(&artifacts.jaunder, storage);
    init.arg("init");
    let mut code = session::run(init, None, signals).await?;
    if code == 0 && profile != Profile::Empty {
        let support = artifacts
            .support
            .as_ref()
            .context("profile seeder was not built")?;
        let mut seed = app_command(support, storage);
        seed.args(["seed-sandbox-profile", "--db"])
            .arg(database_url(storage))
            .args(["--profile", profile.as_str(), "--storage-path"])
            .arg(storage)
            .env("JAUNDER_TEST_EXECUTABLE", &artifacts.jaunder);
        code = session::run(seed, None, signals).await?;
    }
    if code == 0 {
        workspace::write_profile(storage, profile)?;
    }
    Ok(code)
}

struct CommandGuards {
    // Release storage exclusion before admitting the next workspace-exclusive transition.
    _runtime: Option<Lock>,
    _workspace: Lock,
}
async fn admit(
    paths: &Named,
    digest: &str,
    signals: &mut Signals,
) -> anyhow::Result<CommandGuards> {
    if let Some(lease) = Lock::try_exclusive(&paths.lease)? {
        let exclusive = Lock::exclusive(&paths.ownership, signals).await?;
        paths.recover()?;
        workspace::read_profile(&paths.current)?;
        let runtime = Lock::runtime(&paths.current)?;
        paths.remove_metadata()?;
        // Retain lease AND runtime guard across the exclusive-to-shared ownership gap.
        drop(exclusive);
        let shared = Lock::shared(&paths.ownership)?;
        drop(lease);
        return Ok(CommandGuards {
            _workspace: shared,
            _runtime: Some(runtime),
        });
    }
    let shared = Lock::shared(&paths.ownership)?;
    paths.validate()?;
    workspace::read_profile(&paths.current)?;
    let metadata =
        workspace::read_server(&paths.metadata)?.context("sandbox is starting/stopping")?;
    ensure!(
        metadata.state == State::Ready,
        "sandbox is starting/stopping"
    );
    ensure!(
        metadata.executable_digest == digest,
        "sandbox server uses a different executable; restart it before running commands"
    );
    session::live_ready(&paths.current.join("runtime.json")).await?;
    Ok(CommandGuards {
        _workspace: shared,
        _runtime: None,
    })
}

fn database_url(storage: &Path) -> String {
    format!("sqlite:{}", storage.join("jaunder.db").display())
}
fn app_command(executable: &Path, storage: &Path) -> Command {
    let mut command = Command::new(executable);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("JAUNDER_")
            || name.to_string_lossy().starts_with("OTEL_")
        {
            command.env_remove(name);
        }
    }
    command
        .env("JAUNDER_STORAGE_PATH", storage)
        .env("JAUNDER_DB", database_url(storage))
        .env("JAUNDER_ENV", "dev")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_RUNTIME_FILE", storage.join("runtime.json"));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopped_admission_keeps_lease_while_waiting_and_does_not_self_deadlock() {
        let root = tempfile::tempdir().unwrap();
        let paths = Named::open(root.path(), &"writing".parse().unwrap()).unwrap();
        std::fs::create_dir(&paths.current).unwrap();
        workspace::write_profile(&paths.current, Profile::Empty).unwrap();
        let mut first_signals = Signals::new().unwrap();
        let first = admit(&paths, "abc", &mut first_signals).await.unwrap();
        assert!(Lock::try_exclusive(&paths.lease).unwrap().is_some());
        let mut second_signals = Signals::new().unwrap();
        let second = admit(&paths, "abc", &mut second_signals);
        tokio::pin!(second);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
                .await
                .is_err()
        );
        assert!(Lock::try_exclusive(&paths.lease).unwrap().is_none());
        drop(first);
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap();
        assert!(Lock::try_exclusive(&paths.lease).unwrap().is_some());
        assert!(Lock::try_exclusive(&paths.ownership).unwrap().is_none());
        drop(second);
        assert!(Lock::try_exclusive(&paths.ownership).unwrap().is_some());
    }
}
