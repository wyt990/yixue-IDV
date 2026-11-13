use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize structured logging. Honors `RUST_LOG`, defaults to `info`.
pub fn init_tracing(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init();
}
