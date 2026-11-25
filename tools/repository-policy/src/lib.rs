//! Native repository policy and checkout admission, independent of product tooling.
pub mod adr;
pub mod checkout;
#[cfg(test)]
mod concern_test_support;
pub mod files;
pub mod ids;
pub mod markers;
pub mod repository_docs;
pub mod scoped_leptosfmt;
pub mod server_fns;
pub mod source_contracts;
pub mod source_inventory;
#[cfg(test)]
mod test_support;
pub mod web_server_fns;
