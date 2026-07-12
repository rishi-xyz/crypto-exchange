//! Structured logging initialization using [`tracing`].
//!
//! Provides two entry points:
//!
//! - [`logging::init()`](crate::logging::init) — production setup: dual-output (JSON to stdout + daily rolling log file)
//! - [`logging::init_test()`](crate::logging::init_test) — test setup: all output suppressed to `sink`
//!
//! # Output Format
//!
//! All logs are JSON-formatted for machine parsing. Each line includes:
//! `timestamp`, `level`, `message`, `target`, `filename`, `line_number`,
//! and structured `span` data from `#[instrument]` annotations.
//!
//! # Log Files
//!
//! Logs are written to `logs/engine.YYYY-MM-DD.log` in the workspace root,
//! rotated daily via [`tracing-appender`](https://docs.rs/tracing-appender).
//!
//! # Configuration
//!
//! The log level is controlled by the `RUST_LOG` environment variable.
//! Defaults to `info,engine=debug` (engine crate at debug level, everything else at info).

use std::fs;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes the tracing subscriber for production use.
///
/// Sets up two JSON-output layers:
/// 1. **stdout** — for container/orchestrator log collection
/// 2. **File** — daily rolling file at `logs/engine.YYYY-MM-DD.log`
///
/// The log level is read from `RUST_LOG` (e.g. `RUST_LOG=debug`).
/// Falls back to `info,engine=debug` if unset.
///
/// # Panics
///
/// Panics if the tracing subscriber fails to initialize (typically called twice).
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

/// Initializes tracing for tests with all output suppressed.
///
/// Uses `std::io::sink` as the writer so no log output appears during test runs.
/// Respects `RUST_LOG` for filter level, but output is discarded regardless.
/// Safe to call multiple times (uses `try_init`).
pub fn init_test() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::sink)
        .try_init();
}
