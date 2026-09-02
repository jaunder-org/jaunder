//! Repository-census report contract and host-side orchestration.
//!
//! This module assembles the census surface: collectors produce isolated cells,
//! orchestration preserves their state, and rendering presents the same report
//! data to humans and JSON consumers. A collector failure remains visible without
//! discarding completed cells; unavailable capabilities remain unavailable rather
//! than being inferred as clean.

pub(crate) mod adapters;
mod clone;
mod common;
mod conversion;
mod dependency;
mod elisp;
pub(crate) mod history;
mod lsp;
pub(crate) mod model;
mod orchestrate;
mod process;
mod registry;
mod render;
mod semantic;
mod snapshot;
mod source;

pub use model::{
    CellCapability, CellReport, CellSpec, CellState, EvidenceMethod, Language, SignalFamily,
};
pub use orchestrate::{CensusReport, CollectorContext, SignalSection, collect};
pub(crate) use registry::catalog;
pub use render::render_human;
pub use snapshot::SourceSnapshot;
