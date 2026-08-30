//! Private bridge types used only by post storage regression tests.

/// Database-provided physical identity retained only by the no-write regression.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct PhysicalPostTagRowId(String);

impl PhysicalPostTagRowId {
    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}
