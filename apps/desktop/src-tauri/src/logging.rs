//! Structured logging: JSON lines in `<data root>\logs\memora.log.YYYY-MM-DD`, human-readable on stderr.
//! Level via `MEMORA_LOG` (EnvFilter syntax), default `info`.

use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Returns the active log folder (if file logging started) and the guard that flushes it on drop.
/// Without a data root (first run) only stderr is used, so nothing is written to disk.
pub fn init(log_dir: Option<PathBuf>) -> (Option<PathBuf>, Option<WorkerGuard>) {
    let filter = || EnvFilter::try_from_env("MEMORA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr = fmt::layer().with_writer(std::io::stderr);
    match log_dir.filter(|d| std::fs::create_dir_all(d).is_ok()) {
        Some(dir) => {
            let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "memora.log"));
            tracing_subscriber::registry()
                .with(filter())
                .with(stderr)
                .with(fmt::layer().json().with_current_span(false).with_writer(writer))
                .init();
            (Some(dir), Some(guard))
        }
        None => {
            tracing_subscriber::registry().with(filter()).with(stderr).init();
            (None, None)
        }
    }
}
