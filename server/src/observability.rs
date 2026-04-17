use std::sync::Once;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static INIT_TRACING: Once = Once::new();

fn default_filter() -> EnvFilter {
    EnvFilter::new("jaunder=debug,web=debug,common=debug,tower_http=debug,sqlx=info")
}

fn resolved_filter() -> EnvFilter {
    EnvFilter::try_from_env("JAUNDER_LOG_FILTER")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| default_filter())
}

fn use_json_format() -> bool {
    matches!(
        std::env::var("JAUNDER_LOG_FORMAT")
            .unwrap_or_else(|_| "pretty".to_owned())
            .as_str(),
        "json" | "JSON"
    )
}

pub fn init_tracing() {
    INIT_TRACING.call_once(|| {
        // Forward any existing `log` macros to tracing so we can migrate in
        // phases without duplicate logging calls.
        let _ = tracing_log::LogTracer::init();

        let env_filter = resolved_filter();
        let registry = tracing_subscriber::registry().with(env_filter);

        if use_json_format() {
            let _ = registry.with(fmt::layer().json()).try_init();
        } else {
            let _ = registry.with(fmt::layer()).try_init();
        }
    });
}
