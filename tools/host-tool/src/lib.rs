//! The host tool contract shared by concern-owned executables (ADR-0246): the
//! single JSON stdout report, and the Git and checkout admission every host tool
//! performs before it touches the caller's checkout.

pub mod checkout;
pub mod git;
pub mod report;
