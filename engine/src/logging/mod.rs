use std::fs;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,engine=debug"));

    // Ensure logs/ directory exists relative to the workspace root.
    // When running from engine/, the workspace root is one level up.
    let log_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("logs");
    let _ = fs::create_dir_all(&log_dir);

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("engine")
        .filename_suffix("log")
        .build(&log_dir)
        .expect("Failed to create log file appender");

    let file_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_current_span(true)
        .with_span_list(true)
        .flatten_event(true)
        .with_writer(file_appender)
        .with_ansi(false);

    let stdout_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_current_span(true)
        .with_span_list(true)
        .flatten_event(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stdout_layer)
        .init();
}

pub fn init_test() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::sink)
        .try_init();
}
