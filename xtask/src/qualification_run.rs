//! Fixed isolated release-transition qualification; never production acceptance.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::{
    production_baseline::{BehaviorSession, RunLease, RuntimeIdentity, StorageBackend},
    production_baseline_lifecycle::{BaselineLifecycle, QualifiedPackage},
    qualification::{
        AssetExpectation, PublicPresentationExpectation, QualificationFixture as Fixture,
        QualificationPhase as Phase, QualificationRequest, QualificationResult,
        QualificationSurface as Surface, QualificationTransition as Transition, SourcePin,
        SurfaceExpectation,
    },
    qualification_inventory::{InventoryContent, compare_qualification_inventories},
    result::{CommandResult, StepResult},
};

const CUTOVERS: [(Phase, Fixture, Transition); 4] = [
    (Phase::AppB, Fixture::BApplication, Transition::Application),
    (Phase::RollbackA, Fixture::A, Transition::Application),
    (Phase::ThemeB, Fixture::BTheme, Transition::Studio),
    (Phase::RollbackA, Fixture::A, Transition::Studio),
];

#[derive(Debug, Serialize)]
struct PhaseEvidence {
    runtime: RuntimeIdentity,
    request: QualificationRequest,
    result: QualificationResult,
}
#[derive(Debug, Serialize)]
struct BackendEvidence {
    backend: StorageBackend,
    phases: Vec<PhaseEvidence>,
    backup_sha256: String,
    backup_format: u32,
    backup_schema: u32,
}
#[derive(Debug, Serialize)]
struct PreparedProfile {
    fixture: Fixture,
    backend: StorageBackend,
    package_output: String,
    vm: crate::production_baseline_lifecycle::StoreArtifactIdentity,
}

#[derive(Debug, Serialize)]
struct Evidence {
    source: crate::production_baseline::ResolvedRevision,
    packages: Vec<QualifiedPackage>,
    profiles: Vec<PreparedProfile>,
    backends: Vec<BackendEvidence>,
    actual_safari_qualified: bool,
}

fn asset(role: &str, value: &InventoryContent, revision: Option<String>) -> AssetExpectation {
    AssetExpectation {
        role: role.to_owned(),
        url: value.url.clone(),
        digest: value.digest.clone(),
        revision,
        sha256: value.sha256.clone(),
        mime: value.mime.clone(),
        bytes: value.bytes,
        etag: value.etag.clone(),
    }
}

fn http_assets(packages: &[QualifiedPackage]) -> Vec<AssetExpectation> {
    let mut assets = BTreeMap::new();
    for package in packages {
        let app = asset("application", &package.inventory.application, None);
        assets.insert(app.url.clone(), app);
        for (name, theme) in &package.inventory.themes {
            let css = asset(name, &theme.stylesheet, Some(theme.revision_digest.clone()));
            assets.insert(css.url.clone(), css);
        }
    }
    assets.into_values().collect()
}

fn request(
    package: &QualifiedPackage,
    backend: StorageBackend,
    phase: Phase,
    transition: Transition,
    baseline: Option<&QualificationResult>,
    retained: &[QualifiedPackage],
) -> Result<QualificationRequest> {
    let studio = package
        .inventory
        .themes
        .get("studio")
        .context("qualified inventory omits Studio")?;
    let surfaces: &[Surface] = match (phase, transition) {
        (Phase::AppB | Phase::RollbackA, Transition::Application) => {
            &[Surface::Local, Surface::Home]
        }
        (Phase::ThemeB | Phase::RollbackA, Transition::Studio) => {
            &[Surface::Local, Surface::AuthorPermalink]
        }
        _ => &[Surface::Local, Surface::AuthorPermalink, Surface::Home],
    };
    let expected_surfaces = surfaces
        .iter()
        .map(|surface| {
            let original = baseline.and_then(|baseline| {
                baseline
                    .surfaces
                    .iter()
                    .find(|value| value.surface == *surface)
            });
            SurfaceExpectation {
                surface: *surface,
                application: asset("application", &package.inventory.application, None),
                public_presentation: (*surface != Surface::Home).then(|| {
                    PublicPresentationExpectation {
                        stylesheet: asset(
                            "studio",
                            &studio.stylesheet,
                            Some(studio.revision_digest.clone()),
                        ),
                        revision: studio.revision_digest.clone(),
                    }
                }),
                topbar_border_color: if phase == Phase::AppB {
                    Some("rgb(79, 70, 229)".into())
                } else {
                    original.and_then(|value| value.topbar_border_color.clone())
                },
                studio_accent: if *surface == Surface::Home {
                    None
                } else if phase == Phase::ThemeB {
                    Some("#0f766e".into())
                } else {
                    original.and_then(|value| value.studio_accent.clone())
                },
            }
        })
        .collect();
    let request = QualificationRequest {
        sequence: 0,
        phase,
        fixture: package.fixture,
        transition,
        backend,
        source: package.source.clone(),
        package: package.package.clone(),
        expected_surfaces,
        requested_cache_urls: if phase == Phase::AWarm {
            vec![
                package.inventory.application.url.clone(),
                studio.stylesheet.url.clone(),
            ]
        } else {
            Vec::new()
        },
        requested_http_assets: http_assets(retained),
        seed_process: None,
    };
    request.validate()?;
    Ok(request)
}

fn observe(
    browser: &mut BehaviorSession,
    runtime: RuntimeIdentity,
    mut request: QualificationRequest,
    baseline: Option<&QualificationResult>,
) -> Result<PhaseEvidence> {
    if runtime.package != request.package
        || runtime.revision != request.source
        || runtime.backend != request.backend
    {
        bail!("executing runtime differs from qualified request");
    }
    let result = browser.run_qualification(request.clone())?;
    request.sequence = result.sequence;
    result.validate_for(&request)?;
    if let Some(baseline) = baseline {
        if result.anonymous_context_id != baseline.anonymous_context_id
            || result.authenticated_context_id != baseline.authenticated_context_id
        {
            bail!("qualification recreated a retained browser context");
        }
    } else if result.anonymous_context_id == result.authenticated_context_id {
        bail!("anonymous and authenticated observations share a context identity");
    }
    Ok(PhaseEvidence {
        runtime,
        request,
        result,
    })
}

fn backend(
    root: &Path,
    source: &SourcePin,
    lifecycle: &mut BaselineLifecycle,
    packages: &[QualifiedPackage],
    backend: StorageBackend,
    canary_path: &Path,
) -> Result<BackendEvidence> {
    let id = match backend {
        StorageBackend::Sqlite => "styling-sqlite",
        StorageBackend::Postgres => "styling-postgres",
    };
    let a = &packages[0];
    let runtime = lifecycle.start_package(id, backend, a.source.clone(), a.package.clone())?;
    lifecycle.configure_base_url(id)?;
    let state = lifecycle.private_path(&format!("{id}-state.json"))?;
    let seed = lifecycle.seed_process(id)?;
    source
        .verify_current(root)
        .context("checking harness source before browser execution")?;
    let mut browser = BehaviorSession::start_qualification(lifecycle, id, &state, canary_path)?;
    let run = (|| -> Result<BackendEvidence> {
        let mut initial = request(
            a,
            backend,
            Phase::Create,
            Transition::Application,
            None,
            &packages[..1],
        )?;
        initial.seed_process = Some(seed);
        let first = observe(&mut browser, runtime.clone(), initial, None)?;
        let baseline = first.result.clone();
        if baseline
            .surfaces
            .iter()
            .any(|value| value.topbar_border_color.as_deref() == Some("rgb(79, 70, 229)"))
            || baseline
                .surfaces
                .iter()
                .any(|value| value.studio_accent.as_deref() == Some("#0f766e"))
        {
            bail!("bounded B declarations already match the ordinary baseline");
        }
        let mut phases = vec![first];
        phases.push(observe(
            &mut browser,
            runtime,
            request(
                a,
                backend,
                Phase::AWarm,
                Transition::Application,
                Some(&baseline),
                &packages[..1],
            )?,
            Some(&baseline),
        )?);
        let mut installed = 1;
        for (phase, fixture, transition) in CUTOVERS {
            let index = match fixture {
                Fixture::A => 0,
                Fixture::BApplication => 1,
                Fixture::BTheme => 2,
            };
            installed = installed.max(index + 1);
            let package = &packages[index];
            let runtime =
                lifecycle.upgrade_package(id, package.source.clone(), package.package.clone())?;
            phases.push(observe(
                &mut browser,
                runtime,
                request(
                    package,
                    backend,
                    phase,
                    transition,
                    Some(&baseline),
                    &packages[..installed],
                )?,
                Some(&baseline),
            )?);
        }
        let backup = lifecycle.backup(id)?;
        let restored_id = format!("{id}-restored");
        lifecycle.start_package(&restored_id, backend, a.source.clone(), a.package.clone())?;
        lifecycle.restore(&restored_id, &backup)?;
        lifecycle.restart_service(&restored_id)?;
        if lifecycle.observe_schema(&restored_id)? != backup.schema_version {
            bail!("restored qualification schema differs from its source backup");
        }
        lifecycle.configure_base_url(&restored_id)?;
        lifecycle.select_proxy(&restored_id)?;
        let restored = lifecycle.runtime_identity(&restored_id)?;
        phases.push(observe(
            &mut browser,
            restored,
            request(
                a,
                backend,
                Phase::Restored,
                Transition::Studio,
                Some(&baseline),
                packages,
            )?,
            Some(&baseline),
        )?);
        Ok(BackendEvidence {
            backend,
            phases,
            backup_sha256: backup.sha256,
            backup_format: backup.format_version,
            backup_schema: backup.schema_version,
        })
    })();
    let close = browser.close().and_then(|()| {
        source
            .verify_current(root)
            .context("checking harness source after browser execution")
    });
    match (run, close) {
        (Ok(evidence), Ok(())) => Ok(evidence),
        (Err(error), Err(close)) => {
            Err(error.context(format!("browser cleanup also failed: {close}")))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

impl Evidence {
    fn validate(&self) -> Result<()> {
        if self
            .packages
            .iter()
            .map(|value| value.fixture)
            .collect::<Vec<_>>()
            != [Fixture::A, Fixture::BApplication, Fixture::BTheme]
        {
            bail!("qualification package matrix is incomplete");
        }
        if self
            .backends
            .iter()
            .map(|value| value.backend)
            .collect::<Vec<_>>()
            != [StorageBackend::Sqlite, StorageBackend::Postgres]
        {
            bail!("qualification backend matrix is incomplete");
        }
        for package in &self.packages {
            if package.source != self.source {
                bail!("mixed qualification sources");
            }
        }
        let expected_profiles = self
            .packages
            .iter()
            .flat_map(|package| {
                [StorageBackend::Sqlite, StorageBackend::Postgres].map(|backend| {
                    (
                        package.fixture,
                        backend,
                        package.package.output_path.as_str(),
                    )
                })
            })
            .collect::<Vec<_>>();
        if self
            .profiles
            .iter()
            .map(|profile| {
                (
                    profile.fixture,
                    profile.backend,
                    profile.package_output.as_str(),
                )
            })
            .collect::<Vec<_>>()
            != expected_profiles
            || self.profiles.iter().any(|profile| {
                profile.vm.derivation.is_empty()
                    || profile.vm.output_path.is_empty()
                    || profile.vm.nar_hash.is_empty()
            })
        {
            bail!("prepared VM profile identities do not match the fixed package/backend matrix");
        }
        for backend in &self.backends {
            let expected = [
                (Phase::Create, Fixture::A, Transition::Application),
                (Phase::AWarm, Fixture::A, Transition::Application),
                CUTOVERS[0],
                CUTOVERS[1],
                CUTOVERS[2],
                CUTOVERS[3],
                (Phase::Restored, Fixture::A, Transition::Studio),
            ];
            if backend
                .phases
                .iter()
                .map(|phase| {
                    (
                        phase.request.phase,
                        phase.request.fixture,
                        phase.request.transition,
                    )
                })
                .collect::<Vec<_>>()
                != expected
                || backend.backup_format != common::backup::CURRENT_BACKUP_FORMAT_VERSION
                || backend.backup_schema == 0
                || backend.backup_sha256.len() != 64
            {
                bail!("qualification phase or backup matrix is incomplete");
            }
            let baseline = &backend.phases[0].result;
            for (index, phase) in backend.phases.iter().enumerate() {
                phase.result.validate_for(&phase.request)?;
                let package = self
                    .packages
                    .iter()
                    .find(|package| package.fixture == phase.request.fixture)
                    .context("phase fixture has no qualified package")?;
                if phase.request.package != package.package || phase.request.source != self.source {
                    bail!("phase request is not owned by its qualified package/source");
                }
                let installed = [1, 1, 2, 2, 3, 3, 3][index];
                let mut authoritative = request(
                    package,
                    backend.backend,
                    phase.request.phase,
                    phase.request.transition,
                    (index != 0).then_some(baseline),
                    &self.packages[..installed],
                )?;
                authoritative.sequence = phase.request.sequence;
                if index == 0 {
                    authoritative.seed_process = phase.request.seed_process.clone();
                    if authoritative.seed_process.is_none() {
                        bail!("Create evidence omits its real seed composition");
                    }
                }
                if authoritative != phase.request {
                    bail!("phase expectations are not the canonical package/baseline projection");
                }
                if phase.result.sequence != index as u32 + 1
                    || phase.runtime.package != phase.request.package
                    || phase.runtime.revision != self.source
                    || phase.runtime.backend != backend.backend
                    || phase.result.anonymous_context_id != baseline.anonymous_context_id
                    || phase.result.authenticated_context_id != baseline.authenticated_context_id
                {
                    bail!(
                        "qualification evidence has mixed runtime, sequence or context identities"
                    );
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn run(root: &Path) -> Result<CommandResult> {
    let started = Instant::now();
    let source = SourcePin::admit_current(root)?;
    crate::production_baseline::verify_styling_tool_source(&source)?;
    let _lease = RunLease::acquire(root)?;
    let mut lifecycle = BaselineLifecycle::create_qualification(root, &source)?;
    let canary_path = lifecycle.private_path("styling-canaries.jsonl")?;
    fs::File::create(&canary_path)?;
    let workflow = (|| -> Result<Evidence> {
        let packages = [Fixture::A, Fixture::BApplication, Fixture::BTheme]
            .into_iter()
            .map(|fixture| lifecycle.realize_qualification_package(&source, fixture))
            .collect::<Result<Vec<_>>>()?;
        let ordinary = crate::qualification_inventory::map_inventory(
            &host::system_theme::compile_system_artifact_inventory()?,
        );
        if packages[0].inventory != ordinary {
            bail!("fixture A differs from the ordinary compiler inventory");
        }
        let outputs = packages
            .iter()
            .map(|package| package.package.output_path.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if outputs.len() != packages.len() {
            bail!("qualification fixtures alias the same product output");
        }
        for (index, application, studio) in [(1, true, false), (2, false, true)] {
            compare_qualification_inventories(
                &packages[0].inventory,
                &packages[index].inventory,
                application,
                studio,
            )?;
            crate::qualification_csr::compare_csr_payloads(
                &packages[0].csr_payload,
                &packages[index].csr_payload,
                application,
            )?;
        }
        let mut profiles = Vec::new();
        for package in &packages {
            for backend in [StorageBackend::Sqlite, StorageBackend::Postgres] {
                profiles.push(PreparedProfile {
                    fixture: package.fixture,
                    backend,
                    package_output: package.package.output_path.clone(),
                    vm: lifecycle.qualified_vm_identity(&package.package, backend)?,
                });
            }
        }
        let backends = [StorageBackend::Sqlite, StorageBackend::Postgres]
            .into_iter()
            .map(|storage| {
                backend(
                    root,
                    &source,
                    &mut lifecycle,
                    &packages,
                    storage,
                    &canary_path,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let evidence = Evidence {
            source: packages[0].source.clone(),
            packages,
            profiles,
            backends,
            actual_safari_qualified: false,
        };
        evidence.validate()?;
        Ok(evidence)
    })();
    let evidence = match workflow {
        Ok(evidence) => evidence,
        Err(error) => return Err(retain_failure(lifecycle, error)),
    };
    let publication = (|| -> Result<_> {
        source
            .verify_current(root)
            .context("checking source before evidence preparation")?;
        let canaries = lifecycle.evidence_canaries(&[&canary_path])?;
        let json = serde_json::to_string_pretty(&evidence)?;
        crate::production_baseline::scan_retained("styling-summary.json", &json, &canaries)?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let destination = root
            .join("docs/evidence/styling-qualification")
            .join(format!("{}-{nonce}", source.commit()));
        fs::create_dir_all(
            destination
                .parent()
                .context("missing qualification evidence parent")?,
        )?;
        let staging = crate::production_baseline::publication_workspace(root)?;
        fs::write(staging.join("summary.json"), json)?;
        let markdown = format!(
            "# Styling release qualification\n\nSource: `{}`\n\nSQLite and PostgreSQL: cached A → B application → A → B Studio → A → isolated same-backend restore.\n\nActual browser: Chromium. Actual Safari/iPhone: not qualified.\n\nThis is not #1419 production acceptance or milestone closure. Exact identities and observations are in `summary.json`.\n",
            source.commit(),
        );
        crate::production_baseline::scan_retained("styling-summary.md", &markdown, &canaries)?;
        fs::write(staging.join("summary.md"), markdown)?;
        Ok((staging, destination))
    })();
    let (staging, destination) = match publication {
        Ok(prepared) => prepared,
        Err(error) => return Err(retain_failure(lifecycle, error)),
    };
    // A passed record becomes public only after owned processes and runtime
    // workspaces are cleaned. Failure leaves the sanitized staging private.
    lifecycle.cleanup().with_context(|| {
        format!(
            "qualification cleanup failed; unpublished staging retained at {}",
            staging.display(),
        )
    })?;
    source.verify_current(root).with_context(|| {
        format!(
            "source drift before publication; private staging retained at {}",
            staging.display(),
        )
    })?;
    fs::rename(&staging, &destination).with_context(|| {
        format!(
            "publishing qualification evidence; private staging retained at {}",
            staging.display(),
        )
    })?;
    let mut result = CommandResult::new("production-baseline-qualify-styling");
    result.push(
        StepResult::ok("styling-qualification")
            .detail(format!("evidence={}", destination.display())),
    );
    crate::lifecycle::finalize(&mut result, started);
    Ok(result)
}

fn retain_failure(lifecycle: BaselineLifecycle, primary: anyhow::Error) -> anyhow::Error {
    match lifecycle.retain_for_diagnostics() {
        Ok(path) => primary.context(format!(
            "restricted diagnostics retained at {}",
            path.display()
        )),
        Err(cleanup) => primary.context(format!("diagnostic shutdown also failed: {cleanup}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn qualification_cli_admits_only_the_fixed_mode() {
        let cli =
            crate::cli::Cli::try_parse_from(["xtask", "production-baseline", "qualify-styling"])
                .unwrap();
        assert!(matches!(
            cli.command,
            crate::cli::Command::ProductionBaseline(
                crate::cli::ProductionBaselineCommand::QualifyStyling
            )
        ));
        assert!(
            crate::cli::Cli::try_parse_from([
                "xtask",
                "production-baseline",
                "qualify-styling",
                "--fixture",
                "b-app",
            ])
            .is_err()
        );
    }

    #[test]
    fn dirty_source_refuses_before_workspace_or_runtime_mutation() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            crate::git::at(root.path())
                .arg("init")
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.path().join("uncommitted"), "unit fixture").unwrap();
        let error = run(root.path()).err().expect("dirty source must refuse");
        assert!(error.to_string().contains("clean committed checkout"));
        assert!(!root.path().join(".xtask").exists());
    }
}
