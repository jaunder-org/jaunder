//! Fault injection available only in the host test build or through `test-utils`.

/// Corrupt the Tree-sitter query only within this async task. Tests can drive
/// real API and storage paths without a process-global override or affecting
/// concurrently running tests.
pub async fn with_invalid_highlight_query<F: std::future::Future>(future: F) -> F::Output {
    crate::code_highlight::with_invalid_query_for_test(future).await
}
