use serde::{Deserialize, Serialize};

pub const RESULT_SCHEMA_VERSION: u32 = 1;
pub const DATASET_SCHEMA_VERSION: u32 = 1;
pub const GENERATOR_VERSION: u32 = 1;
pub const GENERATOR_SEED: u64 = 1_434;
pub const DATASET_MANIFEST_FILENAME: &str = "dataset-manifest-v1.json";
pub const RESULT_FILENAME: &str = "performance-result-v1.json";
pub const BASELINE_FILENAME: &str = "baseline-v1.json";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetProfile {
    Small,
    Medium,
    Large,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Sqlite,
    Postgres,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Browser {
    Chromium,
    Firefox,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Producer {
    Storage,
    Browser,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildMode {
    Release,
    Debug,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementFrame {
    Cold,
    Warm,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementPosition {
    Initial,
    Deep,
    Point,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Workload {
    PublicTimeline,
    AuthenticatedTimeline,
    OwnerHistory,
    PostHistory,
    RevisionDetail,
    Home,
    App,
    GlobalHistory,
    BrowserPostHistory,
    BrowserRevisionDetail,
}

#[must_use]
pub fn storage_fragment_filename(backend: Backend) -> String {
    format!("storage-{}-v1.json", backend_name(backend))
}
#[must_use]
pub fn browser_fragment_filename(backend: Backend, browser: Browser) -> String {
    format!(
        "browser-{}-{}-v1.json",
        backend_name(backend),
        browser_name(browser)
    )
}
fn backend_name(value: Backend) -> &'static str {
    match value {
        Backend::Sqlite => "sqlite",
        Backend::Postgres => "postgres",
    }
}
fn browser_name(value: Browser) -> &'static str {
    match value {
        Browser::Chromium => "chromium",
        Browser::Firefox => "firefox",
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GeneratorIdentity {
    pub version: u32,
    pub seed: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Distribution {
    pub bucket: String,
    pub count: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CursorRequirement {
    pub workload: Workload,
}
/// Pure allocation plan. It intentionally contains no database-generated identifiers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DatasetPlan {
    pub profile: DatasetProfile,
    pub generator: GeneratorIdentity,
    pub posts: u64,
    pub authors: u64,
    pub revisions: u64,
    pub lifecycle: Vec<Distribution>,
    pub revision_distribution: Vec<Distribution>,
    pub tag_distribution: Vec<Distribution>,
    pub audience_distribution: Vec<Distribution>,
    pub media_distribution: Vec<Distribution>,
    pub body_distribution: Vec<Distribution>,
    pub follows_per_author: u64,
    pub cursor_requirements: Vec<CursorRequirement>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserInitialRows {
    pub home: u64,
    pub app: u64,
    pub global_history: u64,
    pub post_history: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkloadSubjects {
    pub username: String,
    pub history_post_id: u64,
    pub revision_id: u64,
    pub browser_initial_rows: BrowserInitialRows,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimelineCursor {
    pub created_at_us: i64,
    pub post_id: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryCursor {
    pub revision_id: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PersistedCursor {
    Timeline(TimelineCursor),
    History(HistoryCursor),
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cursor {
    pub workload: Workload,
    pub target_percent: u8,
    pub matching_result_count: u64,
    pub resolved_rank: u64,
    pub cursor: PersistedCursor,
}
/// Task 2 resolves this authority from stored records; producers only consume it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DatasetManifest {
    pub schema_version: u32,
    pub plan: DatasetPlan,
    pub subjects: WorkloadSubjects,
    pub cursors: Vec<Cursor>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RawSample {
    pub duration_us: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Summary {
    pub sample_count: u32,
    pub minimum_us: u64,
    pub maximum_us: u64,
    pub mean_us: u64,
    pub median_us: u64,
    pub p95_us: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NamedDerivationIdentity {
    pub name: String,
    pub identity: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompatibilityKey {
    pub result_schema_version: u32,
    pub generator: GeneratorIdentity,
    pub profile: DatasetProfile,
    pub workload: Workload,
    pub backend: Backend,
    pub browser: Option<Browser>,
    pub build_mode: BuildMode,
    pub measurement_frame: MeasurementFrame,
    pub measurement_position: MeasurementPosition,
    pub sample_count: u32,
    pub page_size: Option<u32>,
    pub cursor_target_percent: Option<u8>,
    pub cursor_resolved_rank: Option<u64>,
    pub nix_system: String,
    pub stable_derivation_identities: Vec<NamedDerivationIdentity>,
    pub runner_image: String,
    pub runner_architecture: String,
    pub cpu_model: String,
    pub database_version: String,
    pub browser_version: Option<String>,
}
impl CompatibilityKey {
    #[must_use]
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self == other
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkloadResult {
    pub key: CompatibilityKey,
    pub samples: Vec<RawSample>,
    pub summary: Summary,
    pub rows_returned: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetupDuration {
    pub producer: Producer,
    pub backend: Backend,
    pub provisioning_us: u64,
    pub seeding_us: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorageFragment {
    pub setup: SetupDuration,
    pub workloads: Vec<WorkloadResult>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserDiagnostics {
    pub navigation_artifacts: Vec<String>,
    pub trace_artifacts: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowserFragment {
    pub setup: SetupDuration,
    pub diagnostics: BrowserDiagnostics,
    pub workloads: Vec<WorkloadResult>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "producer", content = "result", rename_all = "snake_case")]
pub enum Fragment {
    Storage(StorageFragment),
    Browser(BrowserFragment),
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FragmentEnvelope {
    pub schema_version: u32,
    pub manifest: DatasetManifest,
    pub fragment: Fragment,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunSelection {
    pub backends: Vec<Backend>,
    pub browsers: Vec<Browser>,
    pub storage: bool,
    pub browser: bool,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunEvidence {
    pub freshness_nonce: String,
    pub producer_derivation_identities: Vec<NamedDerivationIdentity>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunEnvelope {
    pub schema_version: u32,
    pub manifest: DatasetManifest,
    pub selection: RunSelection,
    pub provenance: Provenance,
    pub evidence: RunEvidence,
    pub setup: Vec<SetupDuration>,
    pub workloads: Vec<WorkloadResult>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalProvenance {
    pub git_commit: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitHubProvenance {
    pub repository: String,
    pub workflow: String,
    pub job: String,
    pub reference: String,
    pub head_sha: String,
    pub run_id: u64,
    pub attempt: u32,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "source", content = "details", rename_all = "snake_case")]
pub enum Provenance {
    Local(LocalProvenance),
    Github(GitHubProvenance),
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Baseline {
    pub schema_version: u32,
    pub run: RunEnvelope,
}
