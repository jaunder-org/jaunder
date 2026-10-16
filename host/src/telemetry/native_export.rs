//! Stock native logger ownership. Resolve inherited batch configuration at the
//! process periphery; provider shutdown supplies its bounded drain, without an
//! additional flush. Diagnostics stay on the process subscriber and the owning
//! local sinks apply `SafeSdkFormat` to the six remote-detail OTLP events.
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{
    Resource,
    logs::{BatchConfig, BatchLogProcessor, LogExporter, SdkLoggerProvider},
};

pub(super) struct BatchPolicy(BatchConfig);
impl BatchPolicy {
    pub(super) fn resolve() -> Self {
        Self(BatchConfig::default())
    }
}
pub(super) fn provider<E: LogExporter + 'static>(
    resource: Resource,
    policy: BatchPolicy,
    exporter: E,
) -> SdkLoggerProvider {
    let processor = BatchLogProcessor::builder(exporter)
        .with_batch_config(policy.0)
        .build();
    SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_log_processor(processor)
        .build()
}
pub(super) fn build(
    endpoint: &str,
    resource: Resource,
    policy: BatchPolicy,
) -> anyhow::Result<SdkLoggerProvider> {
    let exporter = anyhow::Context::context(
        opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build(),
        "failed to build OTLP log exporter",
    )?;
    Ok(provider(resource, policy, exporter))
}
pub(super) fn setup_failure(error: &anyhow::Error) {
    super::exporter_fallback(super::FallbackKind::LoggerSetup, error);
}
pub(super) fn shutdown(provider: &SdkLoggerProvider) {
    // Synchronous SDK teardown must not feed the providers being drained.
    let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
    if provider.shutdown().is_err() {
        super::fallback(super::FallbackKind::LoggerShutdown);
    }
}

#[cfg(test)]
#[path = "native_export_tests.rs"]
mod tests;
