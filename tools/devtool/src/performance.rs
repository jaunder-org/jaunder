//! Performance fragment producers that run inside Nix derivations.

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Instant,
};

use anyhow::{Context, bail, ensure};
use clap::{Args, Subcommand, ValueEnum};
use common::{
    ids::{PostId, RevisionId},
    pagination::PageSize,
    seed::TimelineOrder,
    time::UtcInstant,
    username::Username,
    visibility::ViewerIdentity,
};
use performance::{
    Backend, BuildMode, CompatibilityKey, DatasetManifest, Fragment, FragmentEnvelope,
    MeasurementFrame, MeasurementPosition, NamedDerivationIdentity, PersistedCursor, Producer,
    RESULT_SCHEMA_VERSION, RawSample, SetupDuration, Workload, WorkloadResult,
    storage_fragment_filename, summarize, validate_fragment, validate_manifest,
};
use storage::{
    DbConnectOptions, PostCursor, PostRevisionCursor, PostStorage, PublishedPageRequest,
    StorageFactory, StorageRuntimeConfig,
};

const WARM_SAMPLES: usize = 30;

fn page_size() -> PageSize {
    PageSize::default()
}

/// Commands that produce or validate performance fragments.
#[derive(Subcommand)]
pub enum PerformanceCmd {
    /// Measure the direct typed storage read paths.
    Storage(Box<StorageArgs>),
    /// Validate and atomically retain a browser-produced fragment.
    ValidateBrowser(ValidateBrowserArgs),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum BackendArg {
    Sqlite,
    Postgres,
}

impl From<BackendArg> for Backend {
    fn from(value: BackendArg) -> Self {
        match value {
            BackendArg::Sqlite => Self::Sqlite,
            BackendArg::Postgres => Self::Postgres,
        }
    }
}

#[derive(Args)]
pub struct StorageArgs {
    /// Existing fixture database URL.
    #[arg(long)]
    db: DbConnectOptions,
    /// Backend whose fixture this command consumes.
    #[arg(long, value_enum)]
    backend: BackendArg,
    /// Authoritative Task 2 dataset manifest.
    #[arg(long)]
    manifest: PathBuf,
    /// Directory receiving `storage-<backend>-v1.json`.
    #[arg(long)]
    output: PathBuf,
    /// Fixture provisioning duration, reported but never measured as a workload.
    #[arg(long)]
    provisioning_us: u64,
    /// Fixture seeding duration, reported but never measured as a workload.
    #[arg(long)]
    seeding_us: u64,
    #[arg(long)]
    nix_system: String,
    #[arg(long)]
    runner_image: String,
    #[arg(long)]
    runner_architecture: String,
    #[arg(long)]
    cpu_model: String,
    #[arg(long)]
    database_version: String,
    /// Stable `name=identity` derivation identity; repeat for every identity.
    #[arg(long = "stable-derivation-identity", required = true)]
    stable_derivation_identities: Vec<String>,
}

#[derive(Args)]
pub struct ValidateBrowserArgs {
    /// Authoritative Task 2 dataset manifest.
    #[arg(long)]
    manifest: PathBuf,
    /// Browser producer's JSON envelope.
    #[arg(long)]
    input: PathBuf,
    /// Optional directory receiving the validated canonical browser fragment name.
    #[arg(long)]
    output: Option<PathBuf>,
}

pub fn run(command: PerformanceCmd) -> anyhow::Result<()> {
    match command {
        PerformanceCmd::Storage(args) => tokio::runtime::Runtime::new()?.block_on(storage(*args)),
        PerformanceCmd::ValidateBrowser(args) => validate_browser(args),
    }
}

async fn storage(args: StorageArgs) -> anyhow::Result<()> {
    ensure!(
        !cfg!(debug_assertions),
        "storage performance measurements require a release devtool build"
    );
    let manifest = read_manifest(&args.manifest)?;
    let backend = args.backend.into();
    ensure!(
        matches_backend(&args.db, backend),
        "database URL does not match --backend"
    );
    let identities = parse_identities(args.stable_derivation_identities)?;
    let runtime = StorageRuntimeConfig::default();
    let shared = IdentityFields {
        manifest: &manifest,
        backend,
        nix_system: &args.nix_system,
        stable_derivation_identities: &identities,
        runner_image: &args.runner_image,
        runner_architecture: &args.runner_architecture,
        cpu_model: &args.cpu_model,
        database_version: &args.database_version,
    };
    validate_identity_fields(&shared)?;

    let mut workloads = Vec::with_capacity(18);
    for &(workload, position) in workload_positions().iter() {
        let cold = sample_cold(&args.db, &runtime, &manifest, workload, position).await?;
        workloads.push(result(
            &shared,
            workload,
            position,
            MeasurementFrame::Cold,
            cold,
        )?);
        let warm = sample_warm(&args.db, &runtime, &manifest, workload, position).await?;
        workloads.push(result(
            &shared,
            workload,
            position,
            MeasurementFrame::Warm,
            warm,
        )?);
    }
    let envelope = FragmentEnvelope {
        schema_version: RESULT_SCHEMA_VERSION,
        manifest,
        fragment: Fragment::Storage(performance::StorageFragment {
            setup: SetupDuration {
                producer: Producer::Storage,
                backend,
                provisioning_us: args.provisioning_us,
                seeding_us: args.seeding_us,
            },
            workloads,
        }),
    };
    validate_fragment(&envelope).context("storage fragment violates the shared contract")?;
    let output = args.output.join(storage_fragment_filename(backend));
    write_json_atomic(&output, &envelope)?;
    println!("{}", output.display());
    Ok(())
}

fn validate_browser(args: ValidateBrowserArgs) -> anyhow::Result<()> {
    let manifest = read_manifest(&args.manifest)?;
    let envelope: FragmentEnvelope = read_json(&args.input)?;
    ensure!(
        envelope.manifest == manifest,
        "browser fragment manifest is stale or mismatched"
    );
    ensure!(
        matches!(envelope.fragment, Fragment::Browser(_)),
        "input is not a browser fragment"
    );
    validate_fragment(&envelope).context("browser fragment violates the shared contract")?;
    if let Some(output) = args.output {
        let Fragment::Browser(fragment) = &envelope.fragment else {
            unreachable!()
        };
        let browser = fragment
            .workloads
            .first()
            .and_then(|item| item.key.browser)
            .context("validated browser fragment has no browser identity")?;
        let backend = fragment.setup.backend;
        let path = output.join(performance::browser_fragment_filename(backend, browser));
        write_json_atomic(&path, &envelope)?;
        println!("{}", path.display());
    }
    Ok(())
}

struct IdentityFields<'a> {
    manifest: &'a DatasetManifest,
    backend: Backend,
    nix_system: &'a str,
    stable_derivation_identities: &'a [NamedDerivationIdentity],
    runner_image: &'a str,
    runner_architecture: &'a str,
    cpu_model: &'a str,
    database_version: &'a str,
}

fn validate_identity_fields(fields: &IdentityFields<'_>) -> anyhow::Result<()> {
    for value in [
        fields.nix_system,
        fields.runner_image,
        fields.runner_architecture,
        fields.cpu_model,
        fields.database_version,
    ] {
        ensure!(
            !value.trim().is_empty(),
            "performance identity fields must not be blank"
        );
    }
    Ok(())
}

fn result(
    fields: &IdentityFields<'_>,
    workload: Workload,
    position: MeasurementPosition,
    frame: MeasurementFrame,
    samples: Measured,
) -> anyhow::Result<WorkloadResult> {
    let rows_returned = samples.rows;
    let samples = samples.samples;
    let sample_count = u32::try_from(samples.len()).context("sample count exceeds u32")?;
    let cursor = deep_cursor(fields.manifest, workload, position)?;
    Ok(WorkloadResult {
        key: CompatibilityKey {
            result_schema_version: RESULT_SCHEMA_VERSION,
            generator: fields.manifest.plan.generator.clone(),
            profile: fields.manifest.plan.profile,
            workload,
            backend: fields.backend,
            browser: None,
            build_mode: BuildMode::Release,
            measurement_frame: frame,
            measurement_position: position,
            sample_count,
            page_size: if position == MeasurementPosition::Point {
                None
            } else {
                Some(50)
            },
            cursor_target_percent: cursor.map(|cursor| cursor.target_percent),
            cursor_resolved_rank: cursor.map(|cursor| cursor.resolved_rank),
            nix_system: fields.nix_system.to_owned(),
            stable_derivation_identities: fields.stable_derivation_identities.to_vec(),
            runner_image: fields.runner_image.to_owned(),
            runner_architecture: fields.runner_architecture.to_owned(),
            cpu_model: fields.cpu_model.to_owned(),
            database_version: fields.database_version.to_owned(),
            browser_version: None,
        },
        summary: summarize(&samples)?,
        samples,
        rows_returned,
    })
}

struct Measured {
    samples: Vec<RawSample>,
    rows: u64,
}

async fn sample_cold(
    db: &DbConnectOptions,
    runtime: &StorageRuntimeConfig,
    manifest: &DatasetManifest,
    workload: Workload,
    position: MeasurementPosition,
) -> anyhow::Result<Measured> {
    let factory = storage::open_existing_database(db, runtime).await?;
    let call = resolve_call(&factory, manifest, workload, position).await?;
    let (sample, rows) = measure(call).await?;
    Ok(Measured {
        samples: vec![sample],
        rows,
    })
}

async fn sample_warm(
    db: &DbConnectOptions,
    runtime: &StorageRuntimeConfig,
    manifest: &DatasetManifest,
    workload: Workload,
    position: MeasurementPosition,
) -> anyhow::Result<Measured> {
    let factory = storage::open_existing_database(db, runtime).await?;
    let mut samples = Vec::with_capacity(WARM_SAMPLES);
    let mut rows = None;
    for _ in 0..WARM_SAMPLES {
        let call = resolve_call(&factory, manifest, workload, position).await?;
        let (sample, actual_rows) = measure(call).await?;
        if let Some(expected) = rows {
            ensure!(expected == actual_rows, "warm workload row count changed");
        }
        rows = Some(actual_rows);
        samples.push(sample);
    }
    Ok(Measured {
        samples,
        rows: rows.unwrap_or(0),
    })
}

async fn resolve_call(
    factory: &StorageFactory,
    manifest: &DatasetManifest,
    workload: Workload,
    position: MeasurementPosition,
) -> anyhow::Result<Call> {
    let username =
        Username::from_str(&manifest.subjects.username).context("manifest username is invalid")?;
    let owner = factory
        .users()
        .get_user_by_username(&username)
        .await?
        .context("manifest subject user is absent")?
        .user_id;
    let post_id = PostId::from(
        i64::try_from(manifest.subjects.history_post_id)
            .context("manifest post id overflows storage id")?,
    );
    let revision_id = RevisionId::from(
        i64::try_from(manifest.subjects.revision_id)
            .context("manifest revision id overflows storage id")?,
    );
    let posts = factory.posts();
    let cursor = deep_cursor(manifest, workload, position)?;
    Ok(Call {
        posts,
        owner,
        post_id,
        revision_id,
        workload,
        position,
        cursor: cursor.cloned(),
        now: UtcInstant::now(),
    })
}

struct Call {
    posts: std::sync::Arc<dyn PostStorage>,
    owner: common::ids::UserId,
    post_id: PostId,
    revision_id: RevisionId,
    workload: Workload,
    position: MeasurementPosition,
    cursor: Option<performance::Cursor>,
    now: UtcInstant,
}

async fn measure(call: Call) -> anyhow::Result<(RawSample, u64)> {
    let result = match call.workload {
        Workload::PublicTimeline | Workload::AuthenticatedTimeline => {
            let viewer = if call.workload == Workload::PublicTimeline {
                ViewerIdentity::Anonymous
            } else {
                ViewerIdentity::local(call.owner)
            };
            let cursor = timeline_cursor(call.cursor.as_ref())?;
            let page = match cursor.as_ref() {
                Some(cursor) => PublishedPageRequest::after(cursor, page_size().fetch_limit()),
                None => {
                    PublishedPageRequest::first(TimelineOrder::Newest, page_size().fetch_limit())
                }
            };
            let started = Instant::now();
            let records = call.posts.list_published(page, &viewer, call.now).await?;
            QueryResult::Timeline(records, elapsed_us(started))
        }
        Workload::OwnerHistory | Workload::PostHistory => {
            let cursor = history_cursor(call.cursor.as_ref())?;
            let started = Instant::now();
            let page = if call.workload == Workload::OwnerHistory {
                call.posts
                    .list_owned_revision_history(call.owner, cursor, page_size())
                    .await?
            } else {
                call.posts
                    .list_post_revision_history(call.owner, call.post_id, cursor, page_size())
                    .await?
                    .context("manifest history post is absent")?
            };
            QueryResult::History(page.revisions, elapsed_us(started))
        }
        Workload::RevisionDetail => {
            let started = Instant::now();
            let detail = call
                .posts
                .get_post_revision_detail(call.owner, call.post_id, call.revision_id)
                .await?
                .context("manifest revision detail is absent")?;
            QueryResult::Detail(Box::new(detail), elapsed_us(started))
        }
        _ => bail!("not a storage workload"),
    };
    let (duration_us, rows) = match result {
        QueryResult::Timeline(records, duration_us) => {
            assert_timeline(&records, call.position, call.cursor.as_ref())?;
            (duration_us, records.len() as u64)
        }
        QueryResult::History(records, duration_us) => {
            assert_history(&records, call.position, call.cursor.as_ref())?;
            (duration_us, records.len() as u64)
        }
        QueryResult::Detail(detail, duration_us) => {
            ensure!(
                detail.revision.post_id == call.post_id
                    && detail.revision.revision_id == call.revision_id
                    && detail.revision.user_id == call.owner,
                "revision detail identity differs from manifest"
            );
            (duration_us, 1)
        }
    };
    Ok((RawSample { duration_us }, rows))
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

enum QueryResult {
    Timeline(Vec<storage::PostRecord>, u64),
    History(Vec<storage::PostRevisionMetadata>, u64),
    Detail(Box<storage::PostRevisionDetail>, u64),
}

fn assert_timeline(
    records: &[storage::PostRecord],
    position: MeasurementPosition,
    cursor: Option<&performance::Cursor>,
) -> anyhow::Result<()> {
    ensure!(!records.is_empty(), "timeline workload returned no rows");
    if position == MeasurementPosition::Deep {
        let PersistedCursor::Timeline(expected) =
            &cursor.context("missing deep timeline cursor")?.cursor
        else {
            bail!("timeline workload received history cursor")
        };
        ensure!(
            records.iter().all(|record| {
                let published_at = record
                    .published_at
                    .expect("published timeline storage contract");
                let timestamp = published_at.value().as_microsecond();
                timestamp < expected.created_at_us
                    || (timestamp == expected.created_at_us
                        && i64::from(record.post_id).unsigned_abs() < expected.post_id)
            }),
            "timeline deep result violates manifest cursor"
        );
    }
    Ok(())
}

fn assert_history(
    records: &[storage::PostRevisionMetadata],
    position: MeasurementPosition,
    cursor: Option<&performance::Cursor>,
) -> anyhow::Result<()> {
    ensure!(!records.is_empty(), "history workload returned no rows");
    if position == MeasurementPosition::Deep {
        let PersistedCursor::History(expected) =
            &cursor.context("missing deep history cursor")?.cursor
        else {
            bail!("history workload received timeline cursor")
        };
        ensure!(
            records
                .iter()
                .all(|record| i64::from(record.revision_id).unsigned_abs() < expected.revision_id),
            "history deep result violates manifest cursor"
        );
    }
    Ok(())
}

fn timeline_cursor(cursor: Option<&performance::Cursor>) -> anyhow::Result<Option<PostCursor>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let PersistedCursor::Timeline(cursor) = &cursor.cursor else {
        bail!("timeline workload received history cursor")
    };
    let micros = cursor.created_at_us;
    Ok(Some(PostCursor {
        published_at: jiff::Timestamp::from_microsecond(micros)?.into(),
        post_id: PostId::from(i64::try_from(cursor.post_id)?),
        order: TimelineOrder::Newest,
    }))
}

fn history_cursor(
    cursor: Option<&performance::Cursor>,
) -> anyhow::Result<Option<PostRevisionCursor>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let PersistedCursor::History(cursor) = &cursor.cursor else {
        bail!("history workload received timeline cursor")
    };
    Ok(Some(PostRevisionCursor {
        revision_id: RevisionId::from(i64::try_from(cursor.revision_id)?),
    }))
}

fn deep_cursor(
    manifest: &DatasetManifest,
    workload: Workload,
    position: MeasurementPosition,
) -> anyhow::Result<Option<&performance::Cursor>> {
    if position != MeasurementPosition::Deep {
        return Ok(None);
    }
    manifest
        .cursors
        .iter()
        .find(|cursor| cursor.workload == workload)
        .map(Some)
        .context("manifest lacks workload cursor")
}

fn workload_positions() -> [(Workload, MeasurementPosition); 9] {
    [
        (Workload::PublicTimeline, MeasurementPosition::Initial),
        (Workload::PublicTimeline, MeasurementPosition::Deep),
        (
            Workload::AuthenticatedTimeline,
            MeasurementPosition::Initial,
        ),
        (Workload::AuthenticatedTimeline, MeasurementPosition::Deep),
        (Workload::OwnerHistory, MeasurementPosition::Initial),
        (Workload::OwnerHistory, MeasurementPosition::Deep),
        (Workload::PostHistory, MeasurementPosition::Initial),
        (Workload::PostHistory, MeasurementPosition::Deep),
        (Workload::RevisionDetail, MeasurementPosition::Point),
    ]
}

fn matches_backend(db: &DbConnectOptions, backend: Backend) -> bool {
    matches!(
        (db, backend),
        (DbConnectOptions::Sqlite(_), Backend::Sqlite)
            | (DbConnectOptions::Postgres { .. }, Backend::Postgres)
    )
}

fn parse_identities(values: Vec<String>) -> anyhow::Result<Vec<NamedDerivationIdentity>> {
    let mut identities = values
        .into_iter()
        .map(|value| {
            let (name, identity) = value
                .split_once('=')
                .context("derivation identity must be name=identity")?;
            ensure!(
                !name.trim().is_empty() && !identity.trim().is_empty(),
                "derivation identity must not be blank"
            );
            Ok(NamedDerivationIdentity {
                name: name.to_owned(),
                identity: identity.to_owned(),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    ensure!(
        !identities.is_empty(),
        "at least one stable derivation identity is required"
    );
    identities.sort_by(|left, right| left.name.cmp(&right.name));
    ensure!(
        identities
            .windows(2)
            .all(|pair| pair[0].name != pair[1].name),
        "derivation identity names must be unique"
    );
    Ok(identities)
}

fn read_manifest(path: &Path) -> anyhow::Result<DatasetManifest> {
    let manifest = read_json(path)?;
    validate_manifest(&manifest).context("dataset manifest is invalid")?;
    Ok(manifest)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> anyhow::Result<()> {
    let parent = path.parent().context("output path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .context("output path has no filename")?
            .to_string_lossy()
    ));
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
