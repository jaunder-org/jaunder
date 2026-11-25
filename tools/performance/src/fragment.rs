use crate::model::{
    Browser, BrowserDiagnostics, BuildMode, CompatibilityKey, DatasetManifest, Fragment,
    FragmentEnvelope, MeasurementFrame, MeasurementPosition, NamedDerivationIdentity, Producer,
    RESULT_SCHEMA_VERSION, Workload, WorkloadResult,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum FragmentError {
    #[error("fragment schema is unsupported")]
    UnsupportedSchema,
    #[error("fragment manifest is invalid: {0}")]
    InvalidManifest(crate::ManifestError),
    #[error("setup producer does not match fragment producer")]
    SetupProducerMismatch,
    #[error("setup backend does not match workload backend")]
    SetupBackendMismatch,
    #[error("browser diagnostics do not hold 20 distinct nonblank artifacts per workload")]
    InvalidDiagnostics,
    #[error("workload is incompatible")]
    IncompatibleWorkload,
    #[error("duplicate workload {0:?}")]
    Duplicate(RequiredKey),
    #[error("missing workload {0:?}")]
    Missing(RequiredKey),
    #[error("unexpected workload {0:?}")]
    Unselected(RequiredKey),
}

/// One measurement a fragment owes for its producer, backend and browser.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RequiredKey {
    workload: Workload,
    browser: Option<Browser>,
    frame: MeasurementFrame,
    position: MeasurementPosition,
}

/// Validates one producer's fragment: the complete, well-formed measurement
/// set for the producer, backend and browser it declares.
///
/// # Errors
///
/// Returns [`FragmentError`] when the schema, manifest, setup, browser
/// diagnostics, workload identities, sample counts, statistics or positions
/// violate the fragment contract, or when a workload is missing, duplicated
/// or not owed by the fragment.
pub fn validate_fragment(envelope: &FragmentEnvelope) -> Result<(), FragmentError> {
    if envelope.schema_version != RESULT_SCHEMA_VERSION {
        return Err(FragmentError::UnsupportedSchema);
    }
    crate::validate_manifest(&envelope.manifest).map_err(FragmentError::InvalidManifest)?;
    let (producer, setup, items) = match &envelope.fragment {
        Fragment::Storage(fragment) => (Producer::Storage, &fragment.setup, &fragment.workloads),
        Fragment::Browser(fragment) => {
            if !valid_diagnostics(&fragment.diagnostics, fragment.workloads.len()) {
                return Err(FragmentError::InvalidDiagnostics);
            }
            (Producer::Browser, &fragment.setup, &fragment.workloads)
        }
    };
    if setup.producer != producer {
        return Err(FragmentError::SetupProducerMismatch);
    }
    if items.iter().any(|item| item.key.backend != setup.backend) {
        return Err(FragmentError::SetupBackendMismatch);
    }
    let browser = match producer {
        Producer::Storage => None,
        Producer::Browser => Some(
            items
                .first()
                .and_then(|item| item.key.browser)
                .ok_or(FragmentError::IncompatibleWorkload)?,
        ),
    };
    account(producer, items, &envelope.manifest, &required(browser))
}

/// Twenty distinct, nonblank navigation and trace artifacts per workload, and
/// a nonblank OTLP trace.
fn valid_diagnostics(diagnostics: &BrowserDiagnostics, workloads: usize) -> bool {
    let navigation = &diagnostics.navigation_artifacts;
    let traces = &diagnostics.trace_artifacts;
    navigation.len() == workloads * 20
        && traces.len() == workloads * 20
        && nonblank(&diagnostics.otel_trace_artifact)
        && navigation.iter().chain(traces).all(|path| nonblank(path))
        && navigation.iter().collect::<BTreeSet<_>>().len() == navigation.len()
        && traces.iter().collect::<BTreeSet<_>>().len() == traces.len()
}

/// Checks each workload and that together they are exactly the `expected` set.
fn account(
    producer: Producer,
    items: &[WorkloadResult],
    manifest: &DatasetManifest,
    expected: &BTreeSet<RequiredKey>,
) -> Result<(), FragmentError> {
    let mut seen = BTreeSet::new();
    for item in items {
        if item.key.result_schema_version != RESULT_SCHEMA_VERSION
            || item.key.generator != manifest.plan.generator
            || item.key.profile != manifest.plan.profile
            || !valid_result(producer, item, manifest)
        {
            return Err(FragmentError::IncompatibleWorkload);
        }
        let key = RequiredKey {
            workload: item.key.workload,
            browser: item.key.browser,
            frame: item.key.measurement_frame,
            position: item.key.measurement_position,
        };
        if !expected.contains(&key) {
            return Err(FragmentError::Unselected(key));
        }
        if !seen.insert(key.clone()) {
            return Err(FragmentError::Duplicate(key));
        }
    }
    if let Some(key) = expected.difference(&seen).next() {
        return Err(FragmentError::Missing(key.clone()));
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

/// The measurements a storage fragment (`None`) or a browser fragment owes.
fn required(browser: Option<Browser>) -> BTreeSet<RequiredKey> {
    let key = |workload, frame, position| RequiredKey {
        workload,
        browser,
        frame,
        position,
    };
    let mut keys = BTreeSet::new();
    if browser.is_none() {
        for workload in [
            Workload::PublicTimeline,
            Workload::AuthenticatedTimeline,
            Workload::OwnerHistory,
            Workload::PostHistory,
        ] {
            for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
                for position in [MeasurementPosition::Initial, MeasurementPosition::Deep] {
                    keys.insert(key(workload, frame, position));
                }
            }
        }
        for frame in [MeasurementFrame::Cold, MeasurementFrame::Warm] {
            keys.insert(key(
                Workload::RevisionDetail,
                frame,
                MeasurementPosition::Point,
            ));
        }
    } else {
        for (workload, position) in [
            (Workload::Home, MeasurementPosition::Initial),
            (Workload::App, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Initial),
            (Workload::GlobalHistory, MeasurementPosition::Deep),
            (Workload::BrowserPostHistory, MeasurementPosition::Initial),
            (Workload::BrowserRevisionDetail, MeasurementPosition::Point),
        ] {
            keys.insert(key(workload, MeasurementFrame::Cold, position));
        }
    }
    keys
}
