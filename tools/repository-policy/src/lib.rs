//! Native repository policy and checkout admission, independent of product tooling.
pub mod adr;
pub mod checkout;
pub mod git;
pub mod ids;
pub mod safety;
#[cfg(test)]
mod test_support;
