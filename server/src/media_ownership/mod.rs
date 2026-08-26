//! Bounded live ownership checks for persisted media references.

mod resolver;
mod transport;

#[cfg(test)]
mod tests;

pub use resolver::LiveMediaReferenceOwnershipResolver;
pub use storage::MediaReferenceOwnershipResolver;
pub use transport::{HeadResponse, HeadTransport, HeadTransportError, ReqwestHeadTransport};
