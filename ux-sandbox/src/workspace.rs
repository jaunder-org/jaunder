//! Legacy-compatible workspace publication and the three distinct exclusion roles.
use crate::{cli::Profile, sandbox_name::SandboxName, session::Signals};
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

fn exists_as(path: &Path, directory: bool) -> anyhow::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                !meta.file_type().is_symlink(),
                "refusing symlink {}",
                path.display()
            );
            ensure!(
                if directory {
                    meta.is_dir()
                } else {
                    meta.is_file()
                },
                "unexpected path type {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}
pub fn regular(path: &Path) -> anyhow::Result<bool> {
    exists_as(path, false)
}
pub fn directory(path: &Path) -> anyhow::Result<()> {
    if !exists_as(path, true)? {
        fs::create_dir(path).with_context(|| format!("creating {}", path.display()))?;
    }
    Ok(())
}

/// Explicit unlock prevents an unrelated inherited descriptor from extending ownership.
pub struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        if let Err(error) = self.0.unlock() {
            eprintln!("sandbox unlock failed: {error}");
        }
    }
}
impl Lock {
    fn open(path: &Path) -> anyhow::Result<Self> {
        regular(path)?;
        Ok(Self(
            OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path)
                .with_context(|| format!("opening lock {}", path.display()))?,
        ))
    }
    pub fn try_exclusive(path: &Path) -> anyhow::Result<Option<Self>> {
        let lock = Self::open(path)?;
        match lock.0.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("locking {}", path.display()))
            }
        }
    }
    pub fn shared(path: &Path) -> anyhow::Result<Self> {
        let lock = Self::open(path)?;
        lock.0
            .try_lock_shared()
            .with_context(|| format!("sandbox is starting/stopping: {}", path.display()))?;
        Ok(lock)
    }
    pub async fn exclusive(path: &Path, signals: &mut Signals) -> anyhow::Result<Self> {
        loop {
            if let Some(lock) = Self::try_exclusive(path)? {
                return Ok(lock);
            }
            tokio::select! {
                _ = signals.next() => bail!("interrupted waiting for {}", path.display()),
                () = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    }
    pub fn runtime(workspace: &Path) -> anyhow::Result<Self> {
        Self::try_exclusive(&workspace.join("runtime.lock"))?
            .context("sandbox runtime is held by an external server")
    }
}

#[derive(Clone)]
pub struct Named {
    pub current: PathBuf,
    pub new: PathBuf,
    pub old: PathBuf,
    pub lease: PathBuf,
    pub ownership: PathBuf,
    pub metadata: PathBuf,
    #[cfg(test)]
    fail_publication: bool,
}
impl Named {
    pub fn open(root: &Path, name: &SandboxName) -> anyhow::Result<Self> {
        let xtask = root.join(".xtask");
        directory(&xtask)?;
        let parent = xtask.join("sandboxes");
        directory(&parent)?;
        let control = parent.join(".locks");
        directory(&control)?;
        let paths = Self {
            current: parent.join(name.as_ref()),
            new: parent.join(format!(".{name}.reset-new")),
            old: parent.join(format!(".{name}.reset-old")),
            lease: control.join(format!("{name}.server.lock")),
            ownership: control.join(format!("{name}.workspace.lock")),
            metadata: control.join(format!("{name}.server.json")),
            #[cfg(test)]
            fail_publication: false,
        };
        paths.validate()?;
        Ok(paths)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        let parent = self.current.parent().context("workspace parent absent")?;
        let xtask = parent.parent().context("sandbox root parent absent")?;
        let control = self.metadata.parent().context("control parent absent")?;
        for path in [xtask, parent, control] {
            ensure!(
                exists_as(path, true)?,
                "sandbox root disappeared: {}",
                path.display()
            );
        }
        for path in [&self.lease, &self.ownership, &self.metadata] {
            regular(path)?;
        }
        for path in [&self.current, &self.new, &self.old] {
            if exists_as(path, true)? {
                for leaf in [
                    ".sandbox.json",
                    "runtime.lock",
                    "runtime.json",
                    "jaunder.db",
                ] {
                    regular(&path.join(leaf))?;
                }
            }
        }
        Ok(())
    }
    pub fn recover(&self) -> anyhow::Result<()> {
        self.validate()?;
        let current = self.current.exists();
        let old = self.old.exists();
        if !current && old {
            let _runtime = Lock::runtime(&self.old)?;
            fs::rename(&self.old, &self.current).context("restoring interrupted reset")?;
        } else if current && old {
            discard(&self.old).context("removing retired reset workspace")?;
        }
        if self.new.exists() {
            discard(&self.new).context("discarding unpublished reset workspace")?;
        }
        Ok(())
    }
    pub fn profile(&self, selected: Option<Profile>, reset: bool) -> anyhow::Result<Profile> {
        if !self.current.exists() {
            ensure!(!reset, "cannot reset absent sandbox workspace");
            return Ok(selected.unwrap_or_default());
        }
        let recorded = read_profile(&self.current)?;
        ensure!(
            reset || selected.is_none(),
            "--profile is only valid when creating or resetting a named workspace"
        );
        Ok(selected.unwrap_or(recorded))
    }
    pub fn publish(&mut self) -> anyhow::Result<()> {
        self.validate()?;
        read_profile(&self.new)?;
        ensure!(
            !self.old.exists(),
            "reset recovery must precede publication"
        );
        if !self.current.exists() {
            fs::rename(&self.new, &self.current)?;
            return Ok(());
        }
        fs::rename(&self.current, &self.old).context("retaining old sandbox workspace")?;
        let publish = self.publish_rename();
        if let Err(error) = publish {
            if let Err(restore) = fs::rename(&self.old, &self.current) {
                return Err(error).with_context(|| {
                    format!("publishing replacement; restoring old also failed: {restore}")
                });
            }
            return Err(error).context("publishing replacement sandbox; old restored");
        }
        fs::remove_dir_all(&self.old).context("removing replaced sandbox")?;
        Ok(())
    }
    fn publish_rename(&mut self) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_publication {
            self.fail_publication = false;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "one-shot publication failure",
            ));
        }
        fs::rename(&self.new, &self.current)
    }
    pub fn remove_metadata(&self) -> anyhow::Result<()> {
        if regular(&self.metadata)? {
            fs::remove_file(&self.metadata)?;
        }
        Ok(())
    }
}

pub fn discard(path: &Path) -> anyhow::Result<()> {
    ensure!(
        exists_as(path, true)?,
        "workspace disappeared before removal: {}",
        path.display()
    );
    let _runtime = Lock::runtime(path)?;
    fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
}

#[derive(Serialize, Deserialize)]
struct ProfileMetadata {
    version: u8,
    profile: Profile,
}
pub fn read_profile(path: &Path) -> anyhow::Result<Profile> {
    let path = path.join(".sandbox.json");
    regular(&path)?;
    let metadata: ProfileMetadata = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    ensure!(
        metadata.version == 1,
        "unsupported sandbox profile metadata version {}",
        metadata.version
    );
    Ok(metadata.profile)
}
pub fn write_profile(path: &Path, profile: Profile) -> anyhow::Result<()> {
    write_json(
        &path.join(".sandbox.json"),
        &ProfileMetadata {
            version: 1,
            profile,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ready,
    Stopping,
}
#[derive(Serialize, Deserialize)]
pub struct ServerMetadata {
    pub version: u8,
    pub state: State,
    pub executable_digest: String,
}
pub fn read_server(path: &Path) -> anyhow::Result<Option<ServerMetadata>> {
    if !regular(path)? {
        return Ok(None);
    }
    let metadata: ServerMetadata = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    ensure!(
        metadata.version == 1,
        "unsupported sandbox server metadata version {}",
        metadata.version
    );
    Ok(Some(metadata))
}
fn write_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    regular(path)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("metadata parent absent")?)?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("publishing {}", path.display()))?;
    Ok(())
}

/// A lease survives readiness, drain and the exclusive command-cleanup barrier.
pub struct Server {
    pub paths: Named,
    _lease: Lock,
    ownership: Option<Lock>,
    digest: String,
}
impl Server {
    pub fn new(paths: Named, lease: Lock, exclusive: Lock, digest: String) -> Self {
        Self {
            paths,
            _lease: lease,
            ownership: Some(exclusive),
            digest,
        }
    }
    fn write_state(&self, state: State) -> anyhow::Result<()> {
        self.paths.validate()?;
        write_json(
            &self.paths.metadata,
            &ServerMetadata {
                version: 1,
                state,
                executable_digest: self.digest.clone(),
            },
        )
    }
    pub fn ready(&mut self) -> anyhow::Result<()> {
        self.write_state(State::Ready)?;
        drop(self.ownership.take());
        self.ownership = Some(Lock::shared(&self.paths.ownership)?);
        Ok(())
    }
    pub fn stopping(&self) -> anyhow::Result<()> {
        self.write_state(State::Stopping)
    }
    pub async fn cleanup(&mut self) -> anyhow::Result<()> {
        drop(self.ownership.take());
        let mut reported = false;
        let _barrier = loop {
            match Lock::try_exclusive(&self.paths.ownership) {
                Ok(Some(lock)) => break lock,
                Err(error) if !reported => {
                    eprintln!(
                        "sandbox cleanup barrier unresolved; retaining server lease: {error:#}"
                    );
                    reported = true;
                }
                Ok(None) | Err(_) => {}
            }
            // Admitted commands own their generation until their supervised children finish.
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        self.paths.remove_metadata()
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
