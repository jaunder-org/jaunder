//! Trace analysis: `Vec<Span>` → a typed [`Analysis`] of every report section.

mod browser;
mod model;
mod orchestrate;
mod span_tree;
mod summary;

pub use model::{
    Analysis, AssetRow, BootCoverageRow, ByProjectRow, E2eTestRow, HotspotRow, LongTaskProjectRow,
    SlowSpanRow, SpanCoverageRow, TargetRow, TraceTotalRow,
};
pub use orchestrate::analyze;

#[cfg(test)]
pub use model::LIFECYCLE_SPAN_NAME;
#[cfg(test)]
pub use orchestrate::analyze_spans;
#[cfg(test)]
pub use span_tree::span_coverage;
