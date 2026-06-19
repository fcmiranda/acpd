// watcher.rs — Filesystem monitoring and log scanning for logwatchd.
//
// Assumptions:
// - Uses the `notify` crate (inotify on Linux) for efficient filesystem events.
// - Tracks file read positions so we only scan new content (tail-like behavior).
// - Matches file names against configured glob patterns before scanning.
// - Compiles error patterns into a RegexSet for efficient multi-pattern matching.
// - Runs as a long-lived task that respects the shutdown watch channel.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use regex::RegexSet;
use tokio::sync::watch;

use crate::config::SharedConfig;
use crate::notifier::{self, RateLimiter, Urgency};

/// State for each tracked log file: the byte offset we've read up to.
struct FileState {
    offset: u64,
}

/// Run the filesystem watcher loop.
///
/// This function:
/// 1. Sets up an inotify watcher on the configured directory.
/// 2. Receives file events via a channel.
/// 3. For create/modify events on matching files, scans new content for error patterns.
/// 4. Sends desktop notifications when errors are found (subject to rate limiting).
/// 5. Exits cleanly when the shutdown signal is received.
pub async fn run_watcher(
    config: SharedConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    // Load initial config snapshot
    let cfg = config.load();

    // Compile error patterns into a RegexSet for efficient matching
    let error_regexes = RegexSet::new(&cfg.error_patterns)?;
    // We'll rebuild this on config reload
    let error_regexes = Arc::new(tokio::sync::RwLock::new(error_regexes));

    // Build glob matchers for file patterns
    let file_matchers: Vec<glob::Pattern> = cfg
        .file_patterns
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    let file_matchers = Arc::new(tokio::sync::RwLock::new(file_matchers));

    // Track read positions for each file
    let file_states: Arc<tokio::sync::Mutex<HashMap<PathBuf, FileState>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    // Set up the rate limiter
    let rate_limiter = Arc::new(RateLimiter::new(cfg.rate_limit_per_minute));

    // Create a channel for notify events
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<notify::Result<Event>>(256);

    // Create the filesystem watcher
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(
        move |res: notify::Result<Event>| {
            // Send event to async channel; ignore errors if channel is full/closed
            let _ = event_tx.blocking_send(res);
        },
    )?;

    let watch_dir = cfg.watch_dir.clone();
    watcher.watch(Path::new(&watch_dir), RecursiveMode::Recursive)?;
    tracing::info!(dir = %watch_dir, "started watching directory for log files");

    // Notification settings
    let app_name = cfg.notification.app_name.clone();
    let urgency = Urgency::from(cfg.notification.urgency.as_str());
    let timeout_ms = (cfg.notification.timeout_secs * 1000) as i32;

    // Watchdog interval — ping systemd watchdog periodically
    let mut watchdog_interval = tokio::time::interval(
        tokio::time::Duration::from_secs(25), // Must be < WatchdogSec/2 (60s/2 = 30s)
    );

    loop {
        tokio::select! {
            // Shutdown signal
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    tracing::info!("watcher received shutdown signal");
                    break;
                }
            }

            // Watchdog ping
            _ = watchdog_interval.tick() => {
                sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]).ok();
            }

            // Filesystem event
            Some(event_result) = event_rx.recv() => {
                match event_result {
                    Ok(event) => {
                        // We care about file creation and modification
                        match event.kind {
                            EventKind::Create(_) | EventKind::Modify(_) => {
                                for path in &event.paths {
                                    let matchers = file_matchers.read().await;
                                    if matches_any_pattern(path, &matchers) {
                                        let regexes = error_regexes.read().await;
                                        if let Err(e) = scan_file(
                                            path,
                                            &file_states,
                                            &regexes,
                                            &rate_limiter,
                                            &app_name,
                                            urgency,
                                            timeout_ms,
                                        )
                                        .await
                                        {
                                            tracing::warn!(
                                                path = %path.display(),
                                                error = %e,
                                                "failed to scan file"
                                            );
                                        }
                                    }
                                }
                            }
                            _ => {} // Ignore other event types (delete, rename, etc.)
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "filesystem watcher error");
                    }
                }
            }
        }
    }

    // Drop the watcher to stop watching
    drop(watcher);
    tracing::info!("filesystem watcher stopped");
    Ok(())
}

/// Check if a file path matches any of the configured glob patterns.
fn matches_any_pattern(path: &Path, patterns: &[glob::Pattern]) -> bool {
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };
    patterns.iter().any(|p| p.matches(file_name))
}

/// Scan a file from the last-known offset for new lines matching error patterns.
///
/// Only reads content appended since the last scan (tail semantics).
/// Sends a desktop notification for each matching line (subject to rate limit).
async fn scan_file(
    path: &Path,
    file_states: &Arc<tokio::sync::Mutex<HashMap<PathBuf, FileState>>>,
    error_regexes: &RegexSet,
    rate_limiter: &RateLimiter,
    app_name: &str,
    urgency: Urgency,
    timeout_ms: i32,
) -> Result<()> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;

    // Get or create file state
    let mut states = file_states.lock().await;
    let state = states
        .entry(path.to_path_buf())
        .or_insert(FileState { offset: 0 });

    // If the file was truncated (e.g. log rotation), reset offset
    if metadata.len() < state.offset {
        tracing::debug!(path = %path.display(), "file was truncated, resetting offset");
        state.offset = 0;
    }

    // Skip if no new content
    if metadata.len() == state.offset {
        return Ok(());
    }

    let current_offset = state.offset;
    // Update offset now to avoid re-reading on concurrent events
    state.offset = metadata.len();
    drop(states); // Release lock before doing I/O

    // Read new content from the last offset
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(current_offset))?;

    let mut line = String::new();
    while reader.read_line(&mut line)? > 0 {
        if error_regexes.is_match(&line) {
            let trimmed = line.trim().to_string();
            let display_path = path.display().to_string();

            tracing::warn!(
                file = %display_path,
                line = %trimmed,
                "error pattern detected"
            );

            // Rate-limited notification
            if rate_limiter.check() {
                let summary = format!("Error in {}", path.file_name().unwrap_or_default().to_string_lossy());
                let body = format!("{}\n\n{}", display_path, trimmed);
                notifier::send_notification(
                    app_name,
                    &summary,
                    &body,
                    urgency,
                    timeout_ms,
                )
                .await?;
            } else {
                tracing::debug!("notification rate limit exceeded, suppressing");
            }
        }
        line.clear();
    }

    Ok(())
}
