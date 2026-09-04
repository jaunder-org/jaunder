//! Trace analysis: `Vec<Span>` → a typed [`Analysis`] of every report section.

mod browser;
mod model;
mod orchestrate;
mod span_tree;
mod summary;

pub use model::{
    Analysis, AssetRow, BootCoverageRow, ByProjectRow, E2eTestRow, HotspotRow, LIFECYCLE_SPAN_NAME,
    LongTaskProjectRow, SlowSpanRow, SpanCoverageRow, TargetRow, TraceTotalRow,
};
pub use orchestrate::{analyze, analyze_spans};
pub use span_tree::span_coverage;
