use crate::model::{
    Backend, Browser, BuildMode, CompatibilityKey, CountOverrides, DatasetManifest, DatasetPlan,
    Fragment, FragmentEnvelope, MeasurementFrame, MeasurementPosition, NamedDerivationIdentity,
    Producer, Provenance, RESULT_SCHEMA_VERSION, RunEnvelope, RunEvidence, RunSelection,
    SetupDuration, Workload, WorkloadResult,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AggregateError {
    #[error("fragment schema is unsupported")]
    UnsupportedSchema,
    #[error("fragment manifest is invalid: {0}")]
    InvalidManifest(crate::ManifestError),
    #[error("fragment manifests differ")]
    IncompatibleManifest,
    #[error("setup producer does not match fragment producer")]
    SetupProducerMismatch,
    #[error("setup backend does not match workload backend")]
    SetupBackendMismatch,
    #[error("duplicate setup for {0:?} on {1:?}")]
    DuplicateSetup(Producer, Backend),
    #[error("run setup records do not match the selected producer matrix")]
    IncompatibleSetup,
    #[error("browser diagnostics contain a blank artifact path")]
    InvalidDiagnostics,
    #[error("run selection is invalid")]
    InvalidSelection,
    #[error("run provenance is invalid")]
    InvalidProvenance,
    #[error("run freshness evidence is invalid")]
    InvalidEvidence,
    #[error("workload is incompatible")]
    IncompatibleWorkload,
    #[error("duplicate workload {0:?}")]
    Duplicate(RequiredKey),
    #[error("missing workload {0:?}")]
    Missing(RequiredKey),
    #[error("unselected workload {0:?}")]
    Unselected(RequiredKey),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequiredKey {
    producer: Producer,
    workload: Workload,
    backend: Backend,
    browser: Option<Browser>,
    frame: MeasurementFrame,
    position: MeasurementPosition,
}

/// Combines producer fragments into one validated performance run artifact.
///
/// # Errors
///
/// Returns [`AggregateError`] when selection, provenance, freshness evidence,
/// manifests, setup metadata, workload identities, sample counts, or browser
/// diagnostics violate the shared artifact contract.
pub fn assemble_run(
    selection: RunSelection,
    provenance: Provenance,
    evidence: RunEvidence,
    fragments: &[FragmentEnvelope],
) -> Result<RunEnvelope, AggregateError> {
    if !valid_selection(&selection) {
        return Err(AggregateError::InvalidSelection);
    }
    if !valid_provenance(&provenance) {
        return Err(AggregateError::InvalidProvenance);
    }
    if !valid_evidence(&evidence) {
        return Err(AggregateError::InvalidEvidence);
    }
    let expected = required(&selection);
    let mut seen_workloads = BTreeSet::new();
    let mut seen_setup = BTreeSet::new();
    let mut manifest = None;
    let mut setup = Vec::new();
    let mut workloads = Vec::new();

    for envelope in fragments {
        if envelope.schema_version != RESULT_SCHEMA_VERSION {
            return Err(AggregateError::UnsupportedSchema);
        }
        crate::validate_manifest(&envelope.manifest).map_err(AggregateError::InvalidManifest)?;
        if let Some(current) = manifest {
            if current != &envelope.manifest {
                return Err(AggregateError::IncompatibleManifest);
            }
        } else {
            manifest = Some(&envelope.manifest);
        }

        let (producer, fragment_setup, items) = match &envelope.fragment {
            Fragment::Storage(fragment) => {
                (Producer::Storage, &fragment.setup, &fragment.workloads)
            }
            Fragment::Browser(fragment) => {
                let expected_artifacts = fragment.workloads.len() * 20;
                let navigation = &fragment.diagnostics.navigation_artifacts;
                let traces = &fragment.diagnostics.trace_artifacts;
                let otel_trace = &fragment.diagnostics.otel_trace_artifact;
                if navigation.len() != expected_artifacts
                    || traces.len() != expected_artifacts
                    || otel_trace.trim().is_empty()
                    || navigation
                        .iter()
                        .chain(traces)
                        .any(|path| path.trim().is_empty())
                    || navigation.iter().collect::<BTreeSet<_>>().len() != navigation.len()
                    || traces.iter().collect::<BTreeSet<_>>().len() != traces.len()
                {
                    return Err(AggregateError::InvalidDiagnostics);
                }
                (Producer::Browser, &fragment.setup, &fragment.workloads)
            }
        };
        validate_setup(producer, fragment_setup, items, &mut seen_setup)?;
        collect(
            producer,
            items,
            &envelope.manifest,
            &expected,
            &mut seen_workloads,
            &mut workloads,
        )?;
        setup.push(fragment_setup.clone());
    }

    let Some(manifest) = manifest else {
        return Err(AggregateError::IncompatibleManifest);
    };
    if let Some(key) = expected.difference(&seen_workloads).next() {
        return Err(AggregateError::Missing(key.clone()));
    }
    if !selection_matches_plan(&selection, &manifest.plan) {
        return Err(AggregateError::InvalidSelection);
    }
    let canonical_selection = is_canonical_selection(&selection);
    Ok(RunEnvelope {
        schema_version: RESULT_SCHEMA_VERSION,
        manifest: manifest.clone(),
        selection,
        canonical_selection,
        provenance,
        evidence,
        setup,
        workloads,
    })
}

/// Validates one complete producer fragment without assigning host provenance.
///
/// # Errors
///
/// Returns [`AggregateError`] when the fragment cannot satisfy its complete
/// backend/producer workload contract.
pub fn validate_fragment(envelope: &FragmentEnvelope) -> Result<(), AggregateError> {
    let (storage, browser, backend) = match &envelope.fragment {
        Fragment::Storage(fragment) => (true, Vec::new(), fragment.setup.backend),
        Fragment::Browser(fragment) => {
            let browser = fragment
                .workloads
                .first()
                .and_then(|item| item.key.browser)
                .ok_or(AggregateError::IncompatibleWorkload)?;
            (false, vec![browser], fragment.setup.backend)
        }
    };
    let canonical = crate::canonical_plan(envelope.manifest.plan.profile);
    let count_overrides = CountOverrides {
        posts: (envelope.manifest.plan.posts != canonical.posts)
            .then_some(envelope.manifest.plan.posts),
        authors: (envelope.manifest.plan.authors != canonical.authors)
            .then_some(envelope.manifest.plan.authors),
        revisions: (envelope.manifest.plan.revisions != canonical.revisions)
            .then_some(envelope.manifest.plan.revisions),
    };
    assemble_run(
        RunSelection {
            backends: vec![backend],
            count_overrides,
            browsers: browser,
            storage,
            browser: !storage,
        },
        Provenance::Local(crate::LocalProvenance {
            git_commit: "single-fragment-validation".to_owned(),
        }),
        RunEvidence {
            measured_at_unix_ms: 1,
            freshness_nonce: "single-fragment-validation".to_owned(),
            producer_derivation_identities: vec![NamedDerivationIdentity {
                name: "validator".to_owned(),
                identity: "single-fragment-validation".to_owned(),
            }],
        },
        std::slice::from_ref(envelope),
    )
    .map(|_| ())
}
/// Validates a complete combined run without trusting its source.
///
/// # Errors
///
/// Returns [`AggregateError`] when the run's schema, manifest, selection,
/// provenance, evidence, setup matrix, workload identities, or statistics drift
/// from the versioned contract.
pub fn validate_run(run: &RunEnvelope) -> Result<(), AggregateError> {
    if run.schema_version != RESULT_SCHEMA_VERSION {
        return Err(AggregateError::UnsupportedSchema);
    }
    if run.canonical_selection != is_canonical_selection(&run.selection) {
        return Err(AggregateError::InvalidSelection);
    }
    crate::validate_manifest(&run.manifest).map_err(AggregateError::InvalidManifest)?;
    if !valid_selection(&run.selection)
        || !selection_matches_plan(&run.selection, &run.manifest.plan)
    {
        return Err(AggregateError::InvalidSelection);
    }
    if !valid_provenance(&run.provenance) {
        return Err(AggregateError::InvalidProvenance);
    }
    if !valid_evidence(&run.evidence) {
        return Err(AggregateError::InvalidEvidence);
    }

    let mut setup_counts = BTreeMap::new();
    for setup in &run.setup {
        let selected = run.selection.backends.contains(&setup.backend)
            && match setup.producer {
                Producer::Storage => run.selection.storage,
                Producer::Browser => run.selection.browser,
            };
        if !selected {
            return Err(AggregateError::IncompatibleSetup);
        }
        *setup_counts
            .entry((setup.producer, setup.backend))
            .or_insert(0_usize) += 1;
    }
    for backend in &run.selection.backends {
        if run.selection.storage && setup_counts.get(&(Producer::Storage, *backend)) != Some(&1) {
            return Err(AggregateError::IncompatibleSetup);
        }
        if run.selection.browser
            && setup_counts.get(&(Producer::Browser, *backend))
                != Some(&run.selection.browsers.len())
        {
            return Err(AggregateError::IncompatibleSetup);
        }
    }

    let expected = required(&run.selection);
    let mut seen = BTreeSet::new();
    let mut checked = Vec::with_capacity(run.workloads.len());
    for item in &run.workloads {
        let producer = if valid_producer_workload(Producer::Storage, &item.key) {
            Producer::Storage
        } else {
            Producer::Browser
        };
        collect(
            producer,
            std::slice::from_ref(item),
            &run.manifest,
            &expected,
            &mut seen,
            &mut checked,
        )?; // cov:ignore: rustc maps only collect's residual error edge to this continuation line
    }
    if let Some(key) = expected.difference(&seen).next() {
        return Err(AggregateError::Missing(key.clone()));
    }
    Ok(())
}

fn valid_selection(selection: &RunSelection) -> bool {
    let unique_backends =
        selection.backends.iter().collect::<BTreeSet<_>>().len() == selection.backends.len();
    let unique_browsers =
        selection.browsers.iter().collect::<BTreeSet<_>>().len() == selection.browsers.len();
    unique_backends
        && unique_browsers
        && !selection.backends.is_empty()
        && (selection.storage || selection.browser)
        && (selection.browser != selection.browsers.is_empty())
}

fn selection_matches_plan(selection: &RunSelection, plan: &DatasetPlan) -> bool {
    let canonical = crate::canonical_plan(plan.profile);
    selection.count_overrides.posts.unwrap_or(canonical.posts) == plan.posts
        && selection
            .count_overrides
            .authors
            .unwrap_or(canonical.authors)
            == plan.authors
        && selection
            .count_overrides
            .revisions
            .unwrap_or(canonical.revisions)
            == plan.revisions
}

#[must_use]
pub fn is_canonical_selection(selection: &RunSelection) -> bool {
    selection.backends == [Backend::Sqlite, Backend::Postgres]
        && selection.browsers == [Browser::Chromium]
        && selection.storage
        && selection.browser
        && selection.count_overrides.is_empty()
}

fn valid_provenance(provenance: &Provenance) -> bool {
    match provenance {
        Provenance::Local(local) => nonblank(&local.git_commit),
        Provenance::Github(github) => {
            [
                github.repository.as_str(),
                github.workflow.as_str(),
                github.job.as_str(),
                github.reference.as_str(),
                github.head_sha.as_str(),
            ]
            .into_iter()
            .all(nonblank)
                && github.run_id != 0
                && github.attempt != 0
        }
    }
}

fn valid_evidence(evidence: &RunEvidence) -> bool {
    evidence.measured_at_unix_ms != 0
        && nonblank(&evidence.freshness_nonce)
        && valid_named_identities(&evidence.producer_derivation_identities)
}

fn validate_setup(
    producer: Producer,
    setup: &SetupDuration,
    items: &[WorkloadResult],
    seen: &mut BTreeSet<(Producer, Backend, Option<Browser>)>,
) -> Result<(), AggregateError> {
    if setup.producer != producer {
        return Err(AggregateError::SetupProducerMismatch);
    }
    if items.iter().any(|item| item.key.backend != setup.backend) {
        return Err(AggregateError::SetupBackendMismatch);
    }
    let browser = match producer {
        Producer::Storage => None,
        Producer::Browser => items.first().and_then(|item| item.key.browser),
    };
    if !seen.insert((setup.producer, setup.backend, browser)) {
        return Err(AggregateError::DuplicateSetup(
            setup.producer,
            setup.backend,
        ));
    }
    Ok(())
}

fn collect(
    producer: Producer,
    items: &[WorkloadResult],
    manifest: &DatasetManifest,
    expected: &BTreeSet<RequiredKey>,
    seen: &mut BTreeSet<RequiredKey>,
    out: &mut Vec<WorkloadResult>,
) -> Result<(), AggregateError> {
    for item in items {
        if item.key.result_schema_version != RESULT_SCHEMA_VERSION
            || item.key.generator != manifest.plan.generator
            || item.key.profile != manifest.plan.profile
            || !valid_result(producer, item, manifest)
        {
            return Err(AggregateError::IncompatibleWorkload);
        }
        let key = RequiredKey {
            producer,
            workload: item.key.workload,
            backend: item.key.backend,
            browser: item.key.browser,
            frame: item.key.measurement_frame,
            position: item.key.measurement_position,
        };
        if !expected.contains(&key) {
            return Err(AggregateError::Unselected(key));
        }
        if !seen.insert(key.clone()) {
            return Err(AggregateError::Duplicate(key));
        }
        out.push(item.clone());
    }
    Ok(())
}

fn valid_result(producer: Producer, item: &WorkloadResult, manifest: &DatasetManifest) -> bool {
    let key = &item.key;
    let expected_samples = match (producer, key.measurement_frame) {
        (Producer::Storage, MeasurementFrame::Cold) => 1,
        (Producer::Storage, MeasurementFrame::Warm) => 30,
        (Producer::Browser, MeasurementFrame::Cold) => 20,
        (Producer::Browser, MeasurementFrame::Warm) => return false,
    };
    if key.build_mode != BuildMode::Release
        || key.sample_count != expected_samples
        || item.samples.len() != expected_samples as usize
        || item.rows_returned == 0
        || crate::summarize(&item.samples).ok().as_ref() != Some(&item.summary)
        || !valid_stable_fields(key)
        || !valid_producer_workload(producer, key)
        || !valid_position(key, manifest)
    {
        return false;
    }
    match producer {
        Producer::Storage => key.browser.is_none() && key.browser_version.is_none(),
        Producer::Browser => {
            key.browser.is_some() && key.browser_version.as_deref().is_some_and(nonblank)
        }
    }
}

fn valid_stable_fields(key: &CompatibilityKey) -> bool {
    [
        key.nix_system.as_str(),
        key.runner_image.as_str(),
        key.runner_architecture.as_str(),
        key.cpu_model.as_str(),
        key.database_version.as_str(),
    ]
    .into_iter()
    .all(nonblank)
        && valid_named_identities(&key.stable_derivation_identities)
}

fn valid_named_identities(identities: &[NamedDerivationIdentity]) -> bool {
    !identities.is_empty()
        && identities
            .iter()
            .all(|identity| nonblank(&identity.name) && nonblank(&identity.identity))
        && identities
            .windows(2)
            .all(|pair| pair[0].name < pair[1].name)
}

fn nonblank(value: &str) -> bool {
    !value.trim().is_empty()
}

fn valid_producer_workload(producer: Producer, key: &CompatibilityKey) -> bool {
    match producer {
        Producer::Storage => matches!(
            key.workload,
            Workload::PublicTimeline
                | Workload::AuthenticatedTimeline
                | Workload::OwnerHistory
                | Workload::PostHistory
                | Workload::RevisionDetail
        ),
        Producer::Browser => match key.workload {
            Workload::Home
            | Workload::App
            | Workload::GlobalHistory
            | Workload::BrowserPostHistory
            | Workload::BrowserRevisionDetail => true,
            Workload::PublicTimeline
            | Workload::AuthenticatedTimeline
            | Workload::OwnerHistory
            | Workload::PostHistory
            | Workload::RevisionDetail => false,
        },
    }
}

fn valid_position(key: &CompatibilityKey, manifest: &DatasetManifest) -> bool {
    if matches!(
        key.workload,
        Workload::RevisionDetail | Workload::BrowserRevisionDetail
    ) {
        return key.measurement_position == MeasurementPosition::Point
            && key.page_size.is_none()
            && key.cursor_target_percent.is_none()
            && key.cursor_resolved_rank.is_none();
    }
    match key.measurement_position {
        MeasurementPosition::Initial => {
            key.page_size == Some(50)
                && key.cursor_target_percent.is_none()
                && key.cursor_resolved_rank.is_none()
        }
        MeasurementPosition::Deep if key.workload == Workload::GlobalHistory => {
            key.page_size == Some(50)
                && key.cursor_target_percent.is_none()
                && key.cursor_resolved_rank.is_none()
        }
        MeasurementPosition::Deep => cursor_workload(key.workload).is_some_and(|workload| {
            manifest.cursors.iter().any(|cursor| {
                cursor.workload == workload
                    && key.page_size == Some(50)
                    && key.cursor_target_percent == Some(80)
                    && key.cursor_resolved_rank == Some(cursor.resolved_rank)
            })
        }),
        MeasurementPosition::Point => false,
    }
}

fn cursor_workload(workload: Workload) -> Option<Workload> {
    match workload {
        Workload::Home => Some(Workload::PublicTimeline),
        Workload::App => Some(Workload::AuthenticatedTimeline),
        Workload::BrowserPostHistory => Some(Workload::PostHistory),
        Workload::PublicTimeline
        | Workload::AuthenticatedTimeline
        | Workload::OwnerHistory
        | Workload::PostHistory => Some(workload),
        Workload::GlobalHistory | Workload::RevisionDetail | Workload::BrowserRevisionDetail => {
            None // cov:ignore: dedicated Global History and point-detail rules return before cursor mapping
        }
    }
}

fn required(selection: &RunSelection) -> BTreeSet<RequiredKey> {
    let mut keys = BTreeSet::new();
    let storage = [
        Workload::PublicTimeline,
        Workload::AuthenticatedTimeline,
        Workload::OwnerHistory,
        Workload::PostHistory,
    ];
    let browser_workloads = [
        (Workload::Home, MeasurementPosition::Initial),
        (Workload::App, MeasurementPosition::Initial),
        (Workload::GlobalHistory, MeasurementPosition::Initial),
        (Workload::GlobalHistory, MeasurementPosition::Deep),
        (Workload::BrowserPostHistory, MeasurementPosition::Initial),
    ];
    if selection.storage {
        for &backend in &selection.backends {
            for workload in storage {
                for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
                    for position in [MeasurementPosition::Initial, MeasurementPosition::Deep] {
                        keys.insert(RequiredKey {
                            producer: Producer::Storage,
                            workload,
                            backend,
                            browser: None,
                            frame,
                            position,
                        });
                    }
                }
            }
            for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
                keys.insert(RequiredKey {
                    producer: Producer::Storage,
                    workload: Workload::RevisionDetail,
                    backend,
                    browser: None,
                    frame,
                    position: MeasurementPosition::Point,
                });
            }
        }
    }
    if selection.browser {
        for &backend in &selection.backends {
            for &browser in &selection.browsers {
                for (workload, position) in browser_workloads {
                    keys.insert(RequiredKey {
                        producer: Producer::Browser,
                        workload,
                        backend,
                        browser: Some(browser),
                        frame: MeasurementFrame::Cold,
                        position,
                    });
                }
                keys.insert(RequiredKey {
                    producer: Producer::Browser,
                    workload: Workload::BrowserRevisionDetail,
                    backend,
                    browser: Some(browser),
                    frame: MeasurementFrame::Cold,
                    position: MeasurementPosition::Point,
                });
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BrowserDiagnostics, BrowserFragment, BrowserInitialRows, Cursor, DATASET_SCHEMA_VERSION,
        DatasetProfile, HistoryCursor, LocalProvenance, PersistedCursor, RawSample,
        StorageFragment, TimelineCursor, WorkloadSubjects, canonical_plan,
    };

    fn manifest() -> DatasetManifest {
        let plan = canonical_plan(DatasetProfile::Small);
        DatasetManifest {
            schema_version: DATASET_SCHEMA_VERSION,
            plan,
            subjects: WorkloadSubjects {
                username: "user".into(),
                history_post_id: 1,
                revision_id: 1,
                browser_initial_rows: BrowserInitialRows {
                    home: 100,
                    app: 100,
                    global_history: 100,
                    post_history: 100,
                },
            },
            cursors: [
                Workload::PublicTimeline,
                Workload::AuthenticatedTimeline,
                Workload::OwnerHistory,
                Workload::PostHistory,
            ]
            .into_iter()
            .map(|workload| Cursor {
                workload,
                target_percent: 80,
                matching_result_count: 100,
                resolved_rank: 80,
                cursor: if matches!(
                    workload,
                    Workload::PublicTimeline | Workload::AuthenticatedTimeline
                ) {
                    PersistedCursor::Timeline(TimelineCursor {
                        created_at_us: 1,
                        post_id: 1,
                    })
                } else {
                    PersistedCursor::History(HistoryCursor { revision_id: 1 })
                },
            })
            .collect(),
        }
    }

    fn item(
        workload: Workload,
        frame: MeasurementFrame,
        position: MeasurementPosition,
    ) -> WorkloadResult {
        let plan = canonical_plan(DatasetProfile::Small);
        let browser = matches!(
            workload,
            Workload::Home
                | Workload::App
                | Workload::GlobalHistory
                | Workload::BrowserPostHistory
                | Workload::BrowserRevisionDetail
        )
        .then_some(Browser::Chromium);
        let count: u32 = if browser.is_some() {
            20
        } else {
            match frame {
                MeasurementFrame::Cold => 1,
                MeasurementFrame::Warm => 30,
            }
        };
        let samples = (1..=u64::from(count))
            .map(|duration_us| RawSample { duration_us })
            .collect::<Vec<_>>();
        let (page_size, cursor_target_percent, cursor_resolved_rank) = match position {
            MeasurementPosition::Initial => (Some(50), None, None),
            MeasurementPosition::Deep if workload == Workload::GlobalHistory => {
                (Some(50), None, None)
            }
            MeasurementPosition::Deep => (Some(50), Some(80), Some(80)),
            MeasurementPosition::Point => (None, None, None),
        };
        WorkloadResult {
            key: CompatibilityKey {
                result_schema_version: RESULT_SCHEMA_VERSION,
                generator: plan.generator,
                profile: plan.profile,
                workload,
                backend: Backend::Sqlite,
                browser,
                build_mode: BuildMode::Release,
                measurement_frame: frame,
                measurement_position: position,
                sample_count: count,
                page_size,
                cursor_target_percent,
                cursor_resolved_rank,
                nix_system: "x86_64-linux".into(),
                stable_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "stable".into(),
                }],
                runner_image: "image".into(),
                runner_architecture: "x86_64".into(),
                cpu_model: "cpu".into(),
                database_version: "database".into(),
                browser_version: browser.map(|_| "browser".into()),
            },
            summary: crate::summarize(&samples).unwrap(),
            samples,
            rows_returned: 50,
        }
    }

    fn storage_workloads() -> Vec<WorkloadResult> {
        let mut workloads = Vec::new();
        for workload in [
            Workload::PublicTimeline,
            Workload::AuthenticatedTimeline,
            Workload::OwnerHistory,
            Workload::PostHistory,
        ] {
            for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
                for position in [MeasurementPosition::Initial, MeasurementPosition::Deep] {
                    workloads.push(item(workload, frame, position));
                }
            }
        }
        for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
            workloads.push(item(
                Workload::RevisionDetail,
                frame,
                MeasurementPosition::Point,
            ));
        }
        workloads
    }

    fn storage_envelope(workloads: Vec<WorkloadResult>) -> FragmentEnvelope {
        FragmentEnvelope {
            schema_version: RESULT_SCHEMA_VERSION,
            manifest: manifest(),
            fragment: Fragment::Storage(StorageFragment {
                setup: SetupDuration {
                    producer: Producer::Storage,
                    backend: Backend::Sqlite,
                    provisioning_us: 1,
                    seeding_us: 1,
                },
                workloads,
            }),
        }
    }
    fn browser_workloads() -> Vec<WorkloadResult> {
        [
            (Workload::Home, MeasurementPosition::Initial),
            (Workload::App, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Deep),
            (Workload::BrowserPostHistory, MeasurementPosition::Initial),
            (Workload::BrowserRevisionDetail, MeasurementPosition::Point),
        ]
        .into_iter()
        .map(|(workload, position)| item(workload, MeasurementFrame::Cold, position))
        .collect()
    }

    fn browser_envelope(workloads: Vec<WorkloadResult>) -> FragmentEnvelope {
        FragmentEnvelope {
            schema_version: RESULT_SCHEMA_VERSION,
            manifest: manifest(),
            fragment: Fragment::Browser(BrowserFragment {
                setup: SetupDuration {
                    producer: Producer::Browser,
                    backend: Backend::Sqlite,
                    provisioning_us: 1,
                    seeding_us: 1,
                },
                diagnostics: BrowserDiagnostics {
                    navigation_artifacts: (0..workloads.len() * 20)
                        .map(|index| format!("navigation-{index}.json"))
                        .collect(),
                    trace_artifacts: (0..workloads.len() * 20)
                        .map(|index| format!("trace-{index}.zip"))
                        .collect(),
                    otel_trace_artifact: "otel-traces.jsonl".into(),
                },
                workloads,
            }),
        }
    }

    fn selection() -> RunSelection {
        RunSelection {
            backends: vec![Backend::Sqlite],
            count_overrides: CountOverrides::default(),
            browsers: vec![],
            storage: true,
            browser: false,
        }
    }

    #[test]
    fn count_overrides_make_an_otherwise_default_selection_noncanonical() {
        assert!(!is_canonical_selection(&RunSelection {
            backends: vec![Backend::Sqlite, Backend::Postgres],
            count_overrides: CountOverrides {
                posts: Some(100),
                authors: None,
                revisions: None,
            },
            browsers: vec![Browser::Chromium],
            storage: true,
            browser: true,
        }));
    }

    fn assemble(fragments: &[FragmentEnvelope]) -> Result<RunEnvelope, AggregateError> {
        assemble_run(
            selection(),
            Provenance::Local(LocalProvenance {
                git_commit: "commit".into(),
            }),
            RunEvidence {
                measured_at_unix_ms: 1,
                freshness_nonce: "one".into(),
                producer_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "salted-one".into(),
                }],
            },
            fragments,
        )
    }

    #[test]
    fn validates_a_complete_run_and_rejects_a_missing_workload() {
        let mut run = assemble(&[storage_envelope(storage_workloads())]).unwrap();
        assert_eq!(run.workloads.len(), 18);
        assert_eq!(run.setup.len(), 1);
        assert_eq!(validate_run(&run), Ok(()));
        run.workloads.pop();
        assert!(matches!(
            validate_run(&run),
            Err(AggregateError::Missing(_))
        ));
    }

    #[test]
    fn validates_a_complete_single_storage_fragment() {
        assert_eq!(
            validate_fragment(&storage_envelope(storage_workloads())),
            Ok(())
        );
    }

    #[test]
    fn validates_a_complete_fragment_with_custom_counts() {
        let mut envelope = storage_envelope(storage_workloads());
        envelope.manifest.plan = crate::plan(
            DatasetProfile::Small,
            CountOverrides {
                posts: Some(120),
                authors: Some(12),
                revisions: Some(777),
            },
        )
        .expect("custom plan");
        assert_eq!(validate_fragment(&envelope), Ok(()));
    }
    #[test]
    fn validates_the_six_canonical_browser_measurements() {
        assert_eq!(
            validate_fragment(&browser_envelope(browser_workloads())),
            Ok(())
        );
    }

    #[test]
    fn assembles_both_selected_browsers_without_collapsing_setup() {
        let chromium = browser_envelope(browser_workloads());
        let mut firefox_workloads = browser_workloads();
        for workload in &mut firefox_workloads {
            workload.key.browser = Some(Browser::Firefox);
        }
        let firefox = browser_envelope(firefox_workloads);
        let run = assemble_run(
            RunSelection {
                backends: vec![Backend::Sqlite],
                count_overrides: CountOverrides::default(),
                browsers: vec![Browser::Chromium, Browser::Firefox],
                storage: false,
                browser: true,
            },
            Provenance::Local(LocalProvenance {
                git_commit: "commit".into(),
            }),
            RunEvidence {
                measured_at_unix_ms: 1,
                freshness_nonce: "two-browsers".into(),
                producer_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "stable".into(),
                }],
            },
            &[chromium, firefox],
        )
        .expect("both browser fragments assemble");
        assert_eq!(run.setup.len(), 2);
        assert_eq!(validate_run(&run), Ok(()));
    }

    #[test]
    fn single_fragment_validation_rejects_a_duplicate_workload() {
        let mut workloads = storage_workloads();
        workloads.push(workloads[0].clone());
        assert!(matches!(
            validate_fragment(&storage_envelope(workloads)),
            Err(AggregateError::Duplicate(_))
        ));
    }

    #[test]
    fn rejects_invalid_manifest_with_its_specific_error() {
        let mut fragment = storage_envelope(storage_workloads());
        fragment.manifest.subjects.username.clear();
        assert_eq!(
            assemble(&[fragment]),
            Err(AggregateError::InvalidManifest(
                crate::ManifestError::Cursor
            ))
        );

        let mut fragment = storage_envelope(storage_workloads());
        fragment.manifest.subjects.username = " ".into();
        assert_eq!(
            assemble(&[fragment]),
            Err(AggregateError::InvalidManifest(
                crate::ManifestError::Cursor
            ))
        );
    }

    #[test]
    fn rejects_setup_identity_and_backend_mismatches() {
        let mut fragment = storage_envelope(storage_workloads());
        let Fragment::Storage(storage) = &mut fragment.fragment else {
            unreachable!("constructed storage fragment")
        };
        storage.setup.producer = Producer::Browser;
        assert_eq!(
            assemble(&[fragment]),
            Err(AggregateError::SetupProducerMismatch)
        );

        let mut fragment = storage_envelope(storage_workloads());
        let Fragment::Storage(storage) = &mut fragment.fragment else {
            unreachable!("constructed storage fragment")
        };
        storage.setup.backend = Backend::Postgres;
        assert_eq!(
            assemble(&[fragment]),
            Err(AggregateError::SetupBackendMismatch)
        );
    }

    #[test]
    fn rejects_noncanonical_samples_and_summaries() {
        let mut workloads = storage_workloads();
        workloads[0].samples.push(RawSample { duration_us: 2 });
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );

        let mut workloads = storage_workloads();
        workloads[0].summary.mean_us += 1;
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_debug_builds_and_duplicate_setup_identities() {
        let mut workloads = storage_workloads();
        workloads[0].key.build_mode = BuildMode::Debug;
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );

        let fragment = storage_envelope(storage_workloads());
        assert_eq!(
            assemble(&[fragment.clone(), fragment]),
            Err(AggregateError::DuplicateSetup(
                Producer::Storage,
                Backend::Sqlite
            ))
        );
    }

    #[test]
    fn rejects_cross_producer_and_bad_position_workloads() {
        let mut workloads = storage_workloads();
        workloads[0].key.workload = Workload::Home;
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );

        let mut workloads = storage_workloads();
        workloads[0].key.page_size = None;
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_browser_warm_measurements_and_wrong_deep_cursor_rank() {
        let mut browser_warm = item(
            Workload::Home,
            MeasurementFrame::Warm,
            MeasurementPosition::Initial,
        );
        browser_warm.key.sample_count = 20;
        browser_warm.samples = vec![RawSample { duration_us: 1 }; 20];
        browser_warm.summary = crate::summarize(&browser_warm.samples).unwrap();
        let fragment = browser_envelope(vec![browser_warm]);
        assert_eq!(
            assemble(&[fragment]),
            Err(AggregateError::IncompatibleWorkload)
        );

        let mut workloads = storage_workloads();
        workloads[1].key.cursor_resolved_rank = Some(79);
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_blank_or_missing_browser_diagnostics_and_stable_derivations() {
        let mut workloads = storage_workloads();
        workloads[0].key.stable_derivation_identities[0]
            .identity
            .clear();
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );

        for diagnostics in [
            BrowserDiagnostics {
                navigation_artifacts: vec![" ".into()],
                trace_artifacts: vec!["trace.json".into()],
                otel_trace_artifact: "otel-traces.jsonl".into(),
            },
            BrowserDiagnostics {
                navigation_artifacts: vec![],
                trace_artifacts: vec!["trace.json".into()],
                otel_trace_artifact: "otel-traces.jsonl".into(),
            },
            BrowserDiagnostics {
                navigation_artifacts: vec![],
                trace_artifacts: vec![],
                otel_trace_artifact: " ".into(),
            },
        ] {
            let fragment = FragmentEnvelope {
                schema_version: RESULT_SCHEMA_VERSION,
                manifest: manifest(),
                fragment: Fragment::Browser(BrowserFragment {
                    setup: SetupDuration {
                        producer: Producer::Browser,
                        backend: Backend::Sqlite,
                        provisioning_us: 1,
                        seeding_us: 1,
                    },
                    diagnostics,
                    workloads: vec![],
                }),
            };
            assert_eq!(
                assemble(&[fragment]),
                Err(AggregateError::InvalidDiagnostics)
            );
        }
    }

    #[test]
    fn rejects_duplicate_stable_derivation_names() {
        let mut workloads = storage_workloads();
        workloads[0]
            .key
            .stable_derivation_identities
            .push(NamedDerivationIdentity {
                name: "producer".into(),
                identity: "other".into(),
            });
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_unsorted_stable_derivation_identities() {
        let mut workloads = storage_workloads();
        workloads[0]
            .key
            .stable_derivation_identities
            .push(NamedDerivationIdentity {
                name: "alpha".into(),
                identity: "stable".into(),
            });
        assert_eq!(
            assemble(&[storage_envelope(workloads)]),
            Err(AggregateError::IncompatibleWorkload)
        );
    }

    #[test]
    fn rejects_invalid_selection_provenance_and_evidence() {
        let provenance = Provenance::Local(LocalProvenance {
            git_commit: "commit".into(),
        });
        let evidence = RunEvidence {
            measured_at_unix_ms: 1,
            freshness_nonce: "nonce".into(),
            producer_derivation_identities: vec![NamedDerivationIdentity {
                name: "producer".into(),
                identity: "salted".into(),
            }],
        };
        assert_eq!(
            assemble_run(
                RunSelection {
                    backends: vec![],
                    count_overrides: CountOverrides::default(),
                    browsers: vec![],
                    storage: true,
                    browser: false,
                },
                provenance.clone(),
                evidence.clone(),
                &[],
            ),
            Err(AggregateError::InvalidSelection)
        );
        assert_eq!(
            assemble_run(
                selection(),
                Provenance::Local(LocalProvenance {
                    git_commit: " ".into(),
                }),
                evidence.clone(),
                &[],
            ),
            Err(AggregateError::InvalidProvenance)
        );
        assert_eq!(
            assemble_run(
                selection(),
                provenance,
                RunEvidence {
                    measured_at_unix_ms: 1,
                    freshness_nonce: " ".into(),
                    producer_derivation_identities: evidence.producer_derivation_identities,
                },
                &[],
            ),
            Err(AggregateError::InvalidEvidence)
        );
    }

    #[test]
    fn preserves_compatibility_when_freshness_evidence_changes() {
        let fragments = [storage_envelope(storage_workloads())];
        let first = assemble_run(
            selection(),
            Provenance::Local(LocalProvenance {
                git_commit: "commit".into(),
            }),
            RunEvidence {
                measured_at_unix_ms: 1,
                freshness_nonce: "one".into(),
                producer_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "salted-one".into(),
                }],
            },
            &fragments,
        )
        .unwrap();
        let second = assemble_run(
            selection(),
            Provenance::Local(LocalProvenance {
                git_commit: "commit".into(),
            }),
            RunEvidence {
                measured_at_unix_ms: 1,
                freshness_nonce: "two".into(),
                producer_derivation_identities: vec![NamedDerivationIdentity {
                    name: "producer".into(),
                    identity: "salted-two".into(),
                }],
            },
            &fragments,
        )
        .unwrap();
        assert_ne!(first.evidence, second.evidence);
        assert!(
            first.workloads[0]
                .key
                .is_compatible_with(&second.workloads[0].key)
        );
    }
}
