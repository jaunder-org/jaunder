//! Native repository policy and checkout admission, independent of product tooling.
pub mod adr;
pub mod census;
pub mod checkout;
#[cfg(test)]
mod concern_test_support;
pub mod files;
pub mod git;
pub mod ids;
pub mod markers;
pub mod org_timezone_manifest;
pub mod repository_docs;
pub mod safety;
pub mod scoped_leptosfmt;
pub mod server_fns;
pub mod source_contracts;
pub mod source_inventory;
#[cfg(test)]
mod test_support;
pub mod web_server_fns;
