//! Versioned contracts the performance producers write: the dataset plan and
//! manifest, the fragment types, statistics, and their validation.
mod dataset;
mod fragment;
mod model;
mod statistics;
pub use dataset::{ManifestError, canonical_plan, plan, validate_manifest};
pub use fragment::{FragmentError, RequiredKey, validate_fragment};
pub use model::*;
pub use statistics::{StatisticsError, summarize};
