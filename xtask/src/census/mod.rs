//! Repository-census report contract and host-side orchestration.

pub(crate) mod adapters;
pub(crate) mod collectors;
pub(crate) mod history;
pub(crate) mod model;
mod orchestrate;
mod render;
mod snapshot;

pub use model::{CellReport, CellState, CollectorSpec, EvidenceMethod, Language, SignalFamily};
pub use orchestrate::{CensusReport, CollectorContext, SignalSection, collect};
pub use render::render_human;
pub use snapshot::SourceSnapshot;
