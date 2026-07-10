//! Shared, cross-cutting UI widgets, co-located per component.
//!
//! Each widget's reactive `#[component]` (client) and its pure render twin
//! (projector) live together in one file here (ADR-0056).

pub mod topbar;

pub use topbar::Topbar;
