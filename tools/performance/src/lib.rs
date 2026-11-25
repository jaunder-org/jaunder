//! Pure versioned contracts shared by performance producers and host analysis.
mod aggregate;
mod dataset;
mod model;
mod statistics;
pub use aggregate::{
    AggregateError, assemble_run, is_canonical_selection, is_valid_selection, validate_fragment,
    validate_run,
};
pub use dataset::{ManifestError, canonical_plan, plan, validate_manifest};
pub use model::*;
pub use statistics::{StatisticsError, summarize};
