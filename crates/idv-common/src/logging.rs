use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Initialize structured logging. Honors `RUST_LOG`, defaults to `info`.
pub fn init_tracing(default_filter: &str) {
    init_tracing_with_log_dir(default_filter, None);
}

/// Initialize tracing with an optional daily-rotated log directory.
/// Empty/None keeps the original stdout behavior (console mode).
pub fn init_tracing_with_log_dir(default_filter: &str, log_dir: Option<&str>) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    if let Some(dir) = log_dir.map(str::trim).filter(|s| !s.is_empty()) {
        let result = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix("idv")
            .filename_suffix("log")
            .max_log_files(14)
            .build(dir);
        if let Ok(appender) = result {
            let layer = fmt::layer().with_ansi(false).with_writer(appender);
            if tracing_subscriber::registry()
                .with(filter.clone())
                .with(layer)
                .try_init()
                .is_ok()
            {
                return;
            }
        } else if let Err(err) = result {
            eprintln!("idv-common: create log appender failed: {err}");
        }
    }

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .try_init();
}
