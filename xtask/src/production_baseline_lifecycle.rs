//! Host-owned immutable package and isolated VM lifecycle boundary.

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::production_baseline::{
    EvidenceCanaries, PackageIdentity, ResolvedRevision, RuntimeIdentity, StorageBackend,
};

const ORIGIN: &str = "https://localhost:8443";
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS: &str = "__JAUNDER_BASELINE_STATUS__";
fn validate_harness_commit(commit: &str) -> Result<()> {
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("harness commit must be a full hexadecimal Git commit");
    }
    Ok(())
}

pub struct BaselineLifecycle {
    workspace: PathBuf,
    harness_ref: String,
    nix_cache: PathBuf,
    proxy: Option<Child>,
    proxy_log: Option<PathBuf>,
    package_cache: BTreeMap<String, PackageIdentity>,
    proxy_cache: BTreeMap<String, PathBuf>,
    vm_cache: BTreeMap<(String, String, StorageBackend), PathBuf>,
    deployments: BTreeMap<String, Deployment>,
}

struct Deployment {
    backend: StorageBackend,
    revision: ResolvedRevision,
    package: PackageIdentity,
    application_port: u16,
    control_port: u16,
    vm_log: PathBuf,
    running: bool,
    vm: Child,
}

/// Restricted backup material retained only for the lifetime of a baseline run.
///
/// Its parsed manifest is intentionally the only backup content callers receive.
#[derive(Debug, Clone)]
pub struct BackupArtifact {
    pub path: PathBuf,
    pub sha256: String,
    pub format_version: u32,
    pub schema_version: u32,
}

#[derive(Deserialize)]
struct BackupManifest {
    #[serde(default = "legacy_format_version")]
    format_version: u32,
    schema_version: i64,
}

const fn legacy_format_version() -> u32 {
    1
}
impl BaselineLifecycle {
    /// Construct the immutable harness boundary used by discovery and acceptance.
    pub fn create_immutable(root: &Path, harness_commit: &str) -> Result<Self> {
        validate_harness_commit(harness_commit)?;
        Self::create_with_harness(root, format!("github:jaunder-org/jaunder/{harness_commit}"))
    }

    /// Construct the intentionally mutable path-flake boundary for the opt-in
    /// no-evidence lifecycle smoke only.
    pub fn create_current_checkout(root: &Path) -> Result<Self> {
        Self::create_with_harness(root, format!("path:{}", root.display()))
    }

    fn create_with_harness(root: &Path, harness_ref: String) -> Result<Self> {
        let parent = root.join(".xtask/production-baseline");
        fs::create_dir_all(&parent)?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let workspace = parent.join(format!("run-{}-{nonce}", std::process::id()));
        fs::create_dir(&workspace)?;
        restrict(&workspace)?;
        let nix_cache = workspace.join("nix-cache");
        fs::create_dir(&nix_cache)?;
        restrict(&nix_cache)?;
        let gcroots = workspace.join("gcroots");
        fs::create_dir(&gcroots)?;
        restrict(&gcroots)?;
        Ok(Self {
            workspace,
            harness_ref,
            nix_cache,
            proxy: None,
            proxy_log: None,
            package_cache: BTreeMap::new(),
            proxy_cache: BTreeMap::new(),
            vm_cache: BTreeMap::new(),
            deployments: BTreeMap::new(),
        })
    }
    /// Allocate a private state file for an external behavior runner.
    pub fn private_path(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            bail!("baseline private state name is unsafe");
        }
        let path = self.workspace.join(name);
        if path.exists() {
            bail!("baseline private state already exists: {}", path.display());
        }
        Ok(path)
    }

    /// Collect the secrets created for this run while they remain in the
    /// access-restricted workspace. The returned registry is never serialized.
    pub fn evidence_canaries(&self, paths: &[&Path]) -> Result<EvidenceCanaries> {
        let mut credentials = Vec::new();
        for path in paths {
            let text = fs::read_to_string(path).with_context(|| {
                format!("reading restricted canary registry {}", path.display())
            })?;
            for line in text.lines() {
                credentials.push(
                    serde_json::from_str::<String>(line)
                        .context("parsing restricted canary registry")?,
                );
            }
        }
        let key = fs::read_to_string(
            self.workspace
                .join("proxy-data/caddy/pki/authorities/local/root.key"),
        )
        .context("reading restricted Caddy private key")?;
        EvidenceCanaries::new(credentials.clone(), credentials, vec![key])
    }
    pub fn smoke(root: &Path, revision: ResolvedRevision) -> Result<Vec<RuntimeIdentity>> {
        let mut lifecycle = Self::create_current_checkout(root)?;
        let mut identities = Vec::new();
        for (id, backend) in [
            ("smoke-sqlite", StorageBackend::Sqlite),
            ("smoke-postgres", StorageBackend::Postgres),
        ] {
            identities.push(
                lifecycle
                    .start(id, backend, revision.clone())
                    .with_context(|| format!("{id}: start"))?,
            );
            lifecycle
                .assert_secure_session(id)
                .with_context(|| format!("{id}: secure session cookie"))?;
            lifecycle
                .wait_for_http_redirect()
                .with_context(|| format!("{id}: HTTP redirect"))?;
            lifecycle
                .restart_service(id)
                .with_context(|| format!("{id}: service restart"))?;
            lifecycle
                .wait_for_https()
                .with_context(|| format!("{id}: HTTPS after restart"))?;
            identities.push(
                lifecycle
                    .runtime_identity(id)
                    .with_context(|| format!("{id}: runtime identity after restart"))?,
            );
            lifecycle
                .reboot(id)
                .with_context(|| format!("{id}: reboot"))?;
            lifecycle
                .wait_for_https()
                .with_context(|| format!("{id}: HTTPS after reboot"))?;
            identities.push(
                lifecycle
                    .runtime_identity(id)
                    .with_context(|| format!("{id}: runtime identity after reboot"))?,
            );
            lifecycle
                .select_proxy(id)
                .with_context(|| format!("{id}: proxy cutover"))?;
        }
        lifecycle.cleanup()?;
        if identities.len() != 6 {
            bail!("lifecycle smoke produced an incomplete runtime identity population");
        }
        Ok(identities)
    }
    /// Write a private Node executable that forwards the existing `test-support`
    /// seed CLI to one VM's lifecycle control channel. The service never sees
    /// this tool; it runs as the isolated VM's `jaunder` account.
    pub fn seed_process(&self, deployment_id: &str) -> Result<PathBuf> {
        let port = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment for seed bridge")?
            .control_port;
        let path = self.private_path(&format!("{deployment_id}-seed.mjs"))?;
        fs::write(
            &path,
            format!(
                r#"#!/usr/bin/env node
import net from "node:net";
const quote = (value) => "'" + value.replaceAll("'", "'\"'\"'") + "'";
const args = process.argv.slice(2).map(quote).join(" ");
const inner = `export $(systemctl show --property=Environment --value jaunder.service | tr ' ' '\\n' | grep '^JAUNDER_'); export JAUNDER_STORAGE_PATH=/var/lib/jaunder/data; test-support ${{args}}`;
const command = `diagnostic=/tmp/jaunder-baseline-seed-$$.err; ${{inner}} 2>"$diagnostic"; code=$?; if [ "$code" -ne 0 ]; then cat "$diagnostic"; fi; rm -f "$diagnostic"; printf '\\n{STATUS}%s\\n' "$code"`;
const socket = net.createConnection({{host: "127.0.0.1", port: {port}}});
let output = "";
socket.on("data", (chunk) => output += chunk);
socket.on("error", (error) => {{ console.error(error.message); process.exitCode = 1; }});
socket.on("end", () => {{
  const marker = "{STATUS}";
  const index = output.lastIndexOf(marker);
  const statusMatch = index < 0 ? null : output.slice(index + marker.length).trimStart().match(/^(\d+)/);
  const status = statusMatch ? Number(statusMatch[1]) : null;
  const body = index < 0 ? output : output.slice(0, index);
  if (status !== 0) {{
    if (body.trim()) process.stderr.write(body.trimEnd() + "\n");
    console.error(`test-support seed command failed with status ${{status ?? "missing"}}`);
    process.exitCode = 1;
    return;
  }}
  process.stdout.write(output.slice(0, index).trimEnd());
}});
socket.end(command + "\\n");
"#
            ),
        )?;
        restrict_executable(&path)?;
        Ok(path)
    }

    pub fn realize_package(&mut self, revision: &ResolvedRevision) -> Result<PackageIdentity> {
        if let Some(identity) = self.package_cache.get(&revision.commit) {
            return Ok(identity.clone());
        }
        let installable = format!("{}#jaunder", revision.flake_ref);
        let out_link = self
            .workspace
            .join("gcroots")
            .join(format!("package-{}", revision.commit));
        let out_link = out_link.to_str().context("non-UTF-8 package GC root")?;
        let output_path = store_path(&nix(
            &self.nix_cache,
            &[
                "build",
                "--out-link",
                out_link,
                "--print-out-paths",
                &installable,
            ],
        )?)?;
        let derivation = store_path(&nix(
            &self.nix_cache,
            &["path-info", "--derivation", &installable],
        )?)?;
        let document: serde_json::Value = serde_json::from_str(&nix(
            &self.nix_cache,
            &["path-info", "--json", &output_path],
        )?)?;
        let entries = document
            .as_object()
            .context("Nix v2 path-info must be an object keyed by store path")?;
        if entries.len() != 1 || !entries.contains_key(&output_path) {
            bail!("Nix path-info did not return exactly the requested immutable output");
        }
        let nar_hash = entries[&output_path]
            .get("narHash")
            .and_then(serde_json::Value::as_str)
            .context("immutable package path info omitted narHash")?
            .to_owned();
        let executable_sha256 = sha256_file(&Path::new(&output_path).join("bin/jaunder"))?;
        let identity = PackageIdentity {
            installable,
            derivation,
            output_path,
            nar_hash,
            executable_sha256,
        };
        self.package_cache
            .insert(revision.commit.clone(), identity.clone());
        Ok(identity)
    }

    pub fn start(
        &mut self,
        deployment_id: &str,
        backend: StorageBackend,
        revision: ResolvedRevision,
    ) -> Result<RuntimeIdentity> {
        self.launch(deployment_id, backend, revision, false)
    }

    /// Reboot a persisted deployment disk under a newly realized immutable
    /// package. The disk is deliberately retained; only its declarative VM
    /// profile and service package change.
    pub fn upgrade(
        &mut self,
        deployment_id: &str,
        revision: ResolvedRevision,
    ) -> Result<RuntimeIdentity> {
        let backend = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment for upgrade")?
            .backend;
        self.park(deployment_id)?;
        self.deployments.remove(deployment_id);
        self.launch(deployment_id, backend, revision, true)
    }

    fn launch(
        &mut self,
        deployment_id: &str,
        backend: StorageBackend,
        revision: ResolvedRevision,
        reuse_disk: bool,
    ) -> Result<RuntimeIdentity> {
        if self.deployments.contains_key(deployment_id) {
            bail!("deployment {deployment_id} already exists");
        }
        let package = self.realize_package(&revision)?;
        let vm_key = (self.harness_ref.clone(), revision.commit.clone(), backend);
        let vm_output = if let Some(output) = self.vm_cache.get(&vm_key) {
            output.clone()
        } else {
            let expression = format!(
                "let harness = builtins.getFlake \"{}\"; package = builtins.storePath \"{}\"; in (harness.lib.productionBaselineVm {{ system = builtins.currentSystem; backend = \"{}\"; inherit package; }}).config.system.build.vm",
                self.harness_ref,
                package.output_path,
                backend_name(backend),
            );
            let out_link = self.workspace.join("gcroots").join(format!(
                "vm-{}-{}",
                revision.commit,
                backend_name(backend)
            ));
            let out_link = out_link.to_str().context("non-UTF-8 VM GC root")?;
            let output = PathBuf::from(store_path(&nix(
                &self.nix_cache,
                &[
                    "build",
                    "--impure",
                    "--out-link",
                    out_link,
                    "--print-out-paths",
                    "--expr",
                    &expression,
                ],
            )?)?);
            self.vm_cache.insert(vm_key, output.clone());
            output
        };
        let runner = Path::new(&vm_output).join("bin").join(format!(
            "run-jaunder-production-baseline-{}-vm",
            backend_name(backend)
        ));
        if !runner.is_file() {
            bail!("immutable VM profile omitted runner {}", runner.display());
        }
        let disk = self.workspace.join(format!("{deployment_id}.qcow2"));
        if disk.exists() != reuse_disk {
            bail!(
                "{} deployment disk {}",
                if reuse_disk {
                    "missing persisted"
                } else {
                    "fresh"
                },
                disk.display()
            );
        }
        let (application_port, control_port) = allocate_ports()?;
        let network = format!(
            "hostfwd=tcp:127.0.0.1:{application_port}-:3000,hostfwd=tcp:127.0.0.1:{control_port}-:39000"
        );
        let vm_log = self.workspace.join(format!("{deployment_id}.vm.log"));
        let vm_output = fs::File::create(&vm_log)?;
        let vm_error = vm_output.try_clone()?;
        let vm = Command::new(&runner)
            .current_dir(&self.workspace)
            .env("NIX_DISK_IMAGE", &disk)
            .env("QEMU_NET_OPTS", network)
            .stdin(Stdio::null())
            .stdout(Stdio::from(vm_output))
            .stderr(Stdio::from(vm_error))
            .spawn()
            .with_context(|| format!("starting {}", runner.display()))?;
        self.deployments.insert(
            deployment_id.to_owned(),
            Deployment {
                backend,
                revision: revision.clone(),
                package: package.clone(),
                application_port,
                control_port,
                vm_log,
                running: true,
                vm,
            },
        );
        self.wait_for_guest(deployment_id)?;
        self.select_proxy(deployment_id)?;
        self.runtime_identity(deployment_id)
    }

    pub fn select_proxy(&mut self, deployment_id: &str) -> Result<()> {
        let application_port = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment for proxy cutover")?
            .application_port;
        let config = self.workspace.join("Caddyfile");
        fs::write(
            &config,
            format!(
                "{{\n  admin off\n  auto_https disable_redirects\n  skip_install_trust\n}}\n\nhttp://localhost:8080 {{\n  redir {ORIGIN}{{uri}} permanent\n}}\n\n{ORIGIN} {{\n  tls internal\n  reverse_proxy 127.0.0.1:{} {{\n    header_up X-Forwarded-Proto https\n  }}\n}}\n",
                application_port
            ),
        )?;
        restrict(&config)?;
        if let Some(mut proxy) = self.proxy.take() {
            stop_child(&mut proxy)?;
        }
        let proxy = if let Some(output) = self.proxy_cache.get(&self.harness_ref) {
            output.clone()
        } else {
            let out_link = self.workspace.join("gcroots/proxy");
            let out_link = out_link.to_str().context("non-UTF-8 proxy GC root")?;
            let output = PathBuf::from(store_path(&nix(
                &self.nix_cache,
                &[
                    "build",
                    "--out-link",
                    out_link,
                    "--print-out-paths",
                    &format!("{}#production-baseline-proxy", self.harness_ref),
                ],
            )?)?);
            self.proxy_cache
                .insert(self.harness_ref.clone(), output.clone());
            output
        };
        let proxy_home = self.workspace.join("proxy-home");
        let proxy_data = self.workspace.join("proxy-data");
        let proxy_config = self.workspace.join("proxy-config");
        for directory in [&proxy_home, &proxy_data, &proxy_config] {
            fs::create_dir_all(directory)?;
            restrict(directory)?;
        }
        let proxy_log = self.workspace.join("proxy.log");
        let proxy_output = fs::File::create(&proxy_log)?;
        let proxy_error = proxy_output.try_clone()?;
        self.proxy = Some(
            Command::new(Path::new(&proxy).join("bin/production-baseline-proxy"))
                .arg(&config)
                .current_dir(&self.workspace)
                .env("HOME", &proxy_home)
                .env("XDG_DATA_HOME", &proxy_data)
                .env("XDG_CONFIG_HOME", &proxy_config)
                .stdin(Stdio::null())
                .stdout(Stdio::from(proxy_output))
                .stderr(Stdio::from(proxy_error))
                .spawn()?,
        );

        self.proxy_log = Some(proxy_log);
        self.wait_for_https()?;
        Ok(())
    }

    fn assert_secure_session(&self, deployment_id: &str) -> Result<()> {
        let package = &self
            .deployments
            .get(deployment_id)
            .context("unknown deployment")?
            .package;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let username = format!("smoke{}", std::process::id());
        let password = format!("smoke{nonce}");
        self.guest(
            deployment_id,
            &format!(
                "export $(systemctl show --property=Environment --value jaunder.service | tr ' ' '\\n' | grep '^JAUNDER_'); {} user-create --username {username} --password {password}",
                Path::new(&package.output_path).join("bin/jaunder").display()
            ),
        )?;
        let headers = self
            .workspace
            .join(format!("{deployment_id}.login-headers"));
        fs::File::create(&headers)?;
        restrict(&headers)?;
        let mut request = Command::new("curl")
            .args([
                "--insecure",
                "--silent",
                "--show-error",
                "--connect-timeout",
                "2",
                "--max-time",
                "10",
                "--output",
                "/dev/null",
                "--dump-header",
                headers.to_str().context("non-UTF-8 workspace path")?,
                "--request",
                "POST",
                "--data-binary",
                "@-",
                &format!("{ORIGIN}/api/auth/login"),
            ])
            .stdin(Stdio::piped())
            .spawn()?;
        request
            .stdin
            .as_mut()
            .context("opening login request stdin")?
            .write_all(format!("username={username}&password={password}").as_bytes())?;
        if !request.wait()?.success() {
            bail!("stable HTTPS login request failed");
        }
        let headers = fs::read_to_string(headers)?;
        let secure_session = headers.lines().any(|line| {
            let Some((name, value)) = line.split_once(':') else {
                return false;
            };
            name.eq_ignore_ascii_case("set-cookie")
                && value.trim_start().starts_with("session=")
                && value.contains("; HttpOnly;")
                && value.contains("; SameSite=Lax;")
                && value.contains("; Path=/;")
                && value.contains("; Secure")
        });
        if !secure_session {
            bail!("stable HTTPS login did not issue the required secure session cookie");
        }
        Ok(())
    }

    pub fn configure_base_url(&self, deployment_id: &str) -> Result<()> {
        let package = &self
            .deployments
            .get(deployment_id)
            .context("unknown deployment for base URL configuration")?
            .package;
        self.guest(
            deployment_id,
            &baseline_command(
                package,
                "site-config set site.base_url https://localhost:8443",
            ),
        )
        .context("configuring stable site base URL")
        .map(|_| ())
    }

    /// Export an archive through the source deployment and retain it only in the
    /// restricted run workspace. The manifest is parsed before returning it.
    pub fn backup(&self, deployment_id: &str) -> Result<BackupArtifact> {
        let package = &self
            .deployments
            .get(deployment_id)
            .context("unknown backup source deployment")?
            .package;
        let guest_path = format!("/var/lib/jaunder/{deployment_id}.tar.gz");
        let command = baseline_command(
            package,
            &format!("backup --mode archive --path {guest_path}"),
        );
        self.guest(deployment_id, &command)
            .context("creating source backup")?;
        let encoded = self
            .guest(deployment_id, &format!("base64 -w0 {guest_path}"))
            .context("copying source backup from isolated VM")?;
        let bytes = decode_base64(&encoded)?;
        let path = self.workspace.join(format!("{deployment_id}.tar.gz"));
        fs::write(&path, &bytes)?;
        restrict(&path)?;
        let manifest = read_backup_manifest(&path)?;
        let schema_version = u32::try_from(manifest.schema_version)
            .context("backup manifest has a negative schema version")?;
        if manifest.format_version != 1 || schema_version == 0 {
            bail!("unsupported backup compatibility before target mutation");
        }
        Ok(BackupArtifact {
            sha256: sha256_file(&path)?,
            path,
            format_version: manifest.format_version,
            schema_version,
        })
    }
    /// Observe the running deployment's schema through the supported archive
    /// manifest path. The ephemeral probe is removed from guest and host state
    /// before this method returns, so it cannot become a workflow backup.
    pub fn observe_schema(&self, deployment_id: &str) -> Result<u32> {
        let package = &self
            .deployments
            .get(deployment_id)
            .context("unknown schema observation deployment")?
            .package;
        let guest_path = format!("/var/lib/jaunder/.{deployment_id}-schema-probe.tar.gz");
        let host_path = self
            .workspace
            .join(format!(".{deployment_id}-schema-probe.tar.gz"));
        let observation = (|| {
            self.guest(
                deployment_id,
                &baseline_command(
                    package,
                    &format!("backup --mode archive --path {guest_path}"),
                ),
            )?;
            let encoded = self.guest(deployment_id, &format!("base64 -w0 {guest_path}"))?;
            fs::write(&host_path, decode_base64(&encoded)?)?;
            restrict(&host_path)?;
            let manifest = read_backup_manifest(&host_path)?;
            let schema_version = u32::try_from(manifest.schema_version)
                .context("schema probe manifest has a negative schema version")?;
            if manifest.format_version != 1 || schema_version == 0 {
                bail!("schema probe reported unsupported backup compatibility");
            }
            Ok(schema_version)
        })();
        let guest_cleanup = self.guest(deployment_id, &format!("rm -f {guest_path}"));
        let host_cleanup = if host_path.exists() {
            fs::remove_file(&host_path)
        } else {
            Ok(())
        };
        match observation {
            Ok(schema_version) => {
                guest_cleanup.context("removing guest schema probe")?;
                host_cleanup.context("removing host schema probe")?;
                Ok(schema_version)
            }
            Err(error) => {
                let _ = guest_cleanup;
                let _ = host_cleanup;
                Err(error)
            }
        }
    }

    /// Restore a compatibility-validated archive into a freshly started target.
    /// The lifecycle owns the transfer and in-guest mutation so workflow code
    /// cannot reach VM disks or control channels directly.
    pub fn restore(&self, deployment_id: &str, backup: &BackupArtifact) -> Result<()> {
        if backup.format_version != 1 || backup.schema_version == 0 {
            bail!("unsupported backup compatibility before target mutation");
        }
        let package = &self
            .deployments
            .get(deployment_id)
            .context("unknown restore target deployment")?
            .package;
        let encoded = encode_base64(&fs::read(&backup.path)?);
        let guest_path = format!("/var/lib/jaunder/{deployment_id}-restore.tar.gz");
        self.upload_base64(deployment_id, &guest_path, &encoded)?;
        let actual = self.guest(deployment_id, &format!("sha256sum {guest_path}"))?;
        if actual.split_whitespace().next() != Some(backup.sha256.as_str()) {
            bail!("backup transfer hash mismatch");
        }
        self.guest(
            deployment_id,
            &baseline_command(package, &format!("restore {guest_path}")),
        )
        .context("restoring compatibility-validated backup")
        .map(|_| ())
    }

    pub fn restart_service(&mut self, deployment_id: &str) -> Result<()> {
        self.guest(deployment_id, "systemctl restart jaunder.service")?;
        self.wait_for_guest(deployment_id)
    }

    pub fn reboot(&mut self, deployment_id: &str) -> Result<()> {
        let previous_boot = self
            .guest(deployment_id, "cat /proc/sys/kernel/random/boot_id")
            .context("reading pre-reboot boot identity")?;
        self.send_only(deployment_id, "systemctl reboot")?;
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            match self.guest(deployment_id, "cat /proc/sys/kernel/random/boot_id") {
                Ok(current_boot) if current_boot != previous_boot => return Ok(()),
                Ok(_) | Err(_) if Instant::now() < deadline => {
                    self.ensure_vm_alive(deployment_id)?;
                    thread::sleep(Duration::from_millis(200));
                }
                Ok(_) => bail!("VM boot identity did not change during reboot"),
                Err(error) => return Err(error).context("reading post-reboot boot identity"),
            }
        }
    }

    /// Stop one VM cleanly while retaining its persistent disk for later restore work.
    pub fn park(&mut self, deployment_id: &str) -> Result<()> {
        if !self
            .deployments
            .get(deployment_id)
            .context("unknown deployment")?
            .running
        {
            return Ok(());
        }
        self.send_only(deployment_id, "systemctl poweroff")?;
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        loop {
            let deployment = self
                .deployments
                .get_mut(deployment_id)
                .context("unknown deployment")?;
            if deployment.vm.try_wait()?.is_some() {
                deployment.running = false;
                return Ok(());
            }
            if Instant::now() >= deadline {
                stop_child(&mut deployment.vm)?;
                deployment.running = false;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn runtime_identity(&mut self, deployment_id: &str) -> Result<RuntimeIdentity> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let (backend, revision, package) = {
                let deployment = self
                    .deployments
                    .get(deployment_id)
                    .context("unknown deployment")?;
                (
                    deployment.backend,
                    deployment.revision.clone(),
                    deployment.package.clone(),
                )
            };
            match self.guest(deployment_id, "pid=$(systemctl show --value --property MainPID jaunder.service); printf '%s\\n%s\\n' \"$(systemctl show --value --property ExecStart jaunder.service)\" \"$(readlink -f /proc/$pid/exe)\"") {
                Ok(output) => {
                    let mut lines = output.lines().filter(|line| !line.is_empty());
                    if let (Some(exec_start), Some(executable)) = (lines.next(), lines.next()) {
                        verify_runtime_paths(&package.output_path, exec_start, executable)?;
                        return Ok(RuntimeIdentity { deployment_id: deployment_id.to_owned(), backend, revision, executable: executable.to_owned(), exec_start: exec_start.to_owned(), package });
                    }
                }
                Err(error) if Instant::now() < deadline => { self.ensure_vm_alive(deployment_id)?; let _ = error; thread::sleep(Duration::from_millis(200)); }
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for runtime identity");
            }
        }
    }

    fn wait_for_guest(&mut self, deployment_id: &str) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            match self.guest(deployment_id, "true") {
                Ok(_) => return Ok(()),
                Err(error) if Instant::now() < deadline => {
                    self.ensure_vm_alive(deployment_id)?;
                    let _ = error;
                    thread::sleep(Duration::from_millis(200));
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn wait_for_https(&mut self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            if let Some(proxy) = self.proxy.as_mut()
                && let Some(status) = proxy.try_wait()?
            {
                bail!(
                    "HTTPS proxy exited early with {status}; diagnostics: {}",
                    self.proxy_log.as_ref().map_or_else(
                        || "<unavailable>".to_owned(),
                        |path| path.display().to_string()
                    )
                );
            }
            let output = Command::new("curl")
                .args([
                    "--insecure",
                    "--silent",
                    "--show-error",
                    "--connect-timeout",
                    "2",
                    "--max-time",
                    "5",
                    "--output",
                    "/dev/null",
                    "--write-out",
                    "%{http_code}",
                    ORIGIN,
                ])
                .output();
            if let Ok(result) = &output
                && result.status.success()
                && (result.stdout.starts_with(b"2")
                    || result.stdout.starts_with(b"3")
                    || result.stdout.starts_with(b"4"))
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for HTTPS proxy readiness");
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    fn wait_for_http_redirect(&mut self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let output = Command::new("curl")
                .args([
                    "--silent",
                    "--show-error",
                    "--connect-timeout",
                    "2",
                    "--max-time",
                    "5",
                    "--output",
                    "/dev/null",
                    "--write-out",
                    "%{redirect_url}",
                    "http://localhost:8080",
                ])
                .output();
            if let Ok(result) = &output {
                let redirect = String::from_utf8_lossy(&result.stdout);
                if result.status.success() && redirect.trim_end_matches('/') == ORIGIN {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                bail!("timed out waiting for HTTP-to-HTTPS redirect");
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn ensure_vm_alive(&mut self, deployment_id: &str) -> Result<()> {
        let deployment = self
            .deployments
            .get_mut(deployment_id)
            .context("unknown deployment")?;
        if let Some(status) = deployment.vm.try_wait()? {
            bail!(
                "VM {deployment_id} exited early with {status}; diagnostics: {}",
                deployment.vm_log.display()
            );
        }
        Ok(())
    }

    fn upload_base64(&self, deployment_id: &str, path: &str, encoded: &str) -> Result<()> {
        let port = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment")?
            .control_port;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.write_all(format!("base64 -d > {path}\n{encoded}").as_bytes())?;
        stream.shutdown(Shutdown::Write)?;
        let mut output = String::new();
        stream.read_to_string(&mut output)?;
        if !output.trim().is_empty() {
            bail!("backup upload emitted unexpected control output: {output}");
        }
        Ok(())
    }

    fn guest(&self, deployment_id: &str, command: &str) -> Result<String> {
        let port = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment")?
            .control_port;
        let framed = format!("{command}; code=$?; printf '\\n{STATUS}%s\\n' \"$code\"");
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.set_read_timeout(Some(Duration::from_millis(200)))?;
        stream.write_all(framed.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.shutdown(Shutdown::Write)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut output = Vec::new();
        let mut bytes = [0_u8; 4096];
        loop {
            match stream.read(&mut bytes) {
                Ok(0) => bail!("guest control response closed before exit-status frame"),
                Ok(count) => {
                    output.extend_from_slice(&bytes[..count]);
                    if let Some(body) = complete_status(&output)? {
                        return Ok(body);
                    }
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::TimedOut
                        && Instant::now() < deadline => {}
                Err(error) => return Err(error.into()),
            }
            if Instant::now() >= deadline {
                bail!("guest control response omitted a complete exit-status frame");
            }
        }
    }

    fn send_only(&self, deployment_id: &str, command: &str) -> Result<()> {
        let port = self
            .deployments
            .get(deployment_id)
            .context("unknown deployment")?
            .control_port;
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.write_all(command.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.shutdown(Shutdown::Write)?;
        Ok(())
    }

    /// Stop all owned processes while preserving the restricted run workspace.
    pub fn retain_for_diagnostics(mut self) -> Result<PathBuf> {
        let workspace = self.workspace.clone();
        self.stop_owned_processes()?;
        Ok(workspace)
    }

    pub fn cleanup(mut self) -> Result<()> {
        self.stop_owned_processes()?;
        cleanup_workspace(&self.workspace)
    }

    fn stop_owned_processes(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        let ids = self.deployments.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            if let Err(error) = self.park(&id) {
                errors.push(error);
            }
        }
        if let Some(mut proxy) = self.proxy.take()
            && let Err(error) = stop_child(&mut proxy)
        {
            errors.push(error);
        }
        for deployment in self.deployments.values_mut() {
            if let Err(error) = stop_child(&mut deployment.vm) {
                errors.push(error);
            }
        }
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for BaselineLifecycle {
    fn drop(&mut self) {
        let ids = self.deployments.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let _ = self.park(&id);
        }
        if let Some(proxy) = self.proxy.as_mut() {
            let _ = stop_child(proxy);
        }
        for deployment in self.deployments.values_mut() {
            let _ = stop_child(&mut deployment.vm);
        }
    }
}

fn complete_status(output: &[u8]) -> Result<Option<String>> {
    let output = std::str::from_utf8(output).context("guest control output was not UTF-8")?;
    let Some((body, status)) = output.rsplit_once(STATUS) else {
        return Ok(None);
    };
    let Some(code) = status.strip_suffix('\n') else {
        return Ok(None);
    };
    if body.contains(STATUS) {
        bail!("guest control response contains multiple exit-status frames");
    }
    if code
        .trim()
        .parse::<i32>()
        .context("malformed guest exit-status frame")?
        != 0
    {
        bail!("guest lifecycle command failed: {}", code.trim());
    }
    Ok(Some(body.trim_end_matches('\n').to_owned()))
}

fn allocate_ports() -> Result<(u16, u16)> {
    let application = TcpListener::bind("127.0.0.1:0")?;
    let control = TcpListener::bind("127.0.0.1:0")?;
    let application_port = application.local_addr()?.port();
    let control_port = control.local_addr()?.port();
    if application_port == control_port {
        bail!("allocated duplicate VM host ports");
    }
    drop((application, control));
    Ok((application_port, control_port))
}
fn backend_name(backend: StorageBackend) -> &'static str {
    match backend {
        StorageBackend::Sqlite => "sqlite",
        StorageBackend::Postgres => "postgres",
    }
}
fn nix(cache: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("nix")
        .env("XDG_CACHE_HOME", cache)
        .args(args)
        .output()?;
    if !output.status.success() {
        bail!(
            "nix {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("Nix output was not UTF-8")
}
fn store_path(output: &str) -> Result<String> {
    output
        .lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("/nix/store/"))
        .map(str::to_owned)
        .context("Nix did not print a store path")
}
fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;

    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 32768];
    loop {
        let read = file.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        hash.update(&bytes[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn baseline_command(package: &PackageIdentity, args: &str) -> String {
    format!(
        "export $(systemctl show --property=Environment --value jaunder.service | tr ' ' '\\n' | grep '^JAUNDER_'); export JAUNDER_STORAGE_PATH=/var/lib/jaunder/data; {} {args}",
        Path::new(&package.output_path)
            .join("bin/jaunder")
            .display()
    )
}

fn read_backup_manifest(path: &Path) -> Result<BackupManifest> {
    let file = fs::File::open(path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry
            .path()?
            .file_name()
            .is_some_and(|name| name == "manifest.json")
        {
            return serde_json::from_reader(&mut entry).context("parsing backup manifest");
        }
    }
    bail!("backup archive does not contain manifest.json")
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64[((value >> 18) & 63) as usize] as char);
        out.push(BASE64[((value >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            BASE64[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(encoded.len() / 4 * 3);
    let mut chunk = [0_u8; 4];
    let mut count = 0;
    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        chunk[count] = byte;
        count += 1;
        if count == 4 {
            let value = chunk
                .iter()
                .enumerate()
                .try_fold(0_u32, |value, (index, byte)| {
                    let digit = match byte {
                        b'A'..=b'Z' => byte - b'A',
                        b'a'..=b'z' => byte - b'a' + 26,
                        b'0'..=b'9' => byte - b'0' + 52,
                        b'+' => 62,
                        b'/' => 63,
                        b'=' if index >= 2 => 0,
                        _ => bail!("malformed base64 backup transfer"),
                    };
                    Ok::<_, anyhow::Error>((value << 6) | u32::from(digit))
                })?;
            out.push((value >> 16) as u8);
            if chunk[2] != b'=' {
                out.push((value >> 8) as u8);
            }
            if chunk[3] != b'=' {
                out.push(value as u8);
            }
            count = 0;
        }
    }
    if count != 0 {
        bail!("truncated base64 backup transfer");
    }
    Ok(out)
}
fn verify_runtime_paths(output: &str, exec_start: &str, executable: &str) -> Result<()> {
    let expected = format!("{output}/bin/jaunder");
    if !exec_start.contains(&expected) || executable != expected {
        bail!("runtime identity does not resolve inside recorded output {output}");
    }
    Ok(())
}
fn cleanup_workspace(workspace: &Path) -> Result<()> {
    let parent = workspace.parent().context("workspace has no parent")?;
    if parent.file_name().and_then(|name| name.to_str()) != Some("production-baseline")
        || !workspace
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("run-"))
    {
        bail!("refusing cleanup outside a baseline run workspace");
    }
    if workspace.exists() {
        fs::remove_dir_all(workspace)?;
    }
    Ok(())
}
fn stop_child(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                bail!("owned lifecycle process did not exit after kill");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    Ok(())
}
fn restrict(path: &Path) -> Result<()> {
    let metadata = regular_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

/// Restrict the lifecycle's private executable bridge without making ordinary
/// sensitive files executable.
fn restrict_executable(path: &Path) -> Result<()> {
    let metadata = regular_metadata(path)?;
    if !metadata.is_file() {
        bail!("refusing to make a non-file executable");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn regular_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("refusing to change permissions through a symlink");
    }
    if !metadata.is_dir() && !metadata.is_file() {
        bail!("refusing to restrict a non-file, non-directory path");
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn restriction_distinguishes_file_and_directory_modes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("directory");
        let file = temp.path().join("file");
        let executable = temp.path().join("executable");
        fs::create_dir(&directory).unwrap();
        fs::write(&file, "secret").unwrap();
        fs::write(&executable, "#!/bin/sh\n").unwrap();
        restrict(&directory).unwrap();
        restrict(&file).unwrap();
        restrict_executable(&executable).unwrap();
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(executable).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn runtime_identity_rejects_mismatch() {
        assert!(
            verify_runtime_paths(
                "/nix/store/p",
                "/nix/store/p/bin/jaunder serve",
                "/nix/store/x/bin/jaunder"
            )
            .is_err()
        );
    }
}
