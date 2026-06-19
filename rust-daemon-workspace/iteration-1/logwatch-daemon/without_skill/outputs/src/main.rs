//! logwatchd - A daemon that monitors directories for new log files
//! and sends desktop notifications when error patterns are found.
//!
//! Features:
//! - Filesystem watching via inotify (Linux) / kqueue (macOS)
//! - TOML-based configuration with hot-reload via SIGHUP
//! - Graceful shutdown on SIGTERM/SIGINT
//! - Desktop notifications with rate limiting
//! - Structured logging via the `tracing` crate
//! - PID file management

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use serde::Deserialize;
use tokio::sync::{mpsc, RwLock};

use futures::StreamExt;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook_tokio::Signals;

use tracing::{debug, error, info, warn};

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// Top-level configuration loaded from the TOML file.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub general: GeneralConfig,
    pub patterns: PatternConfig,
    pub notifications: NotificationConfig,
    pub file_filter: FileFilterConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralConfig {
    pub watch_dir: PathBuf,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_pid_file")]
    pub pid_file: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_log_format")]
    pub log_format: String,
}

fn default_poll_interval() -> u64 {
    5
}
fn default_pid_file() -> PathBuf {
    PathBuf::from("/run/logwatchd.pid")
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_format() -> String {
    "pretty".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct PatternConfig {
    pub entries: Vec<PatternEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PatternEntry {
    pub name: String,
    pub regex: String,
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_severity() -> String {
    "normal".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_app_name")]
    pub app_name: String,
    #[serde(default = "default_max_per_minute")]
    pub max_per_minute: u32,
}

fn default_enabled() -> bool {
    true
}
fn default_timeout_ms() -> u64 {
    10_000
}
fn default_app_name() -> String {
    "logwatchd".to_string()
}
fn default_max_per_minute() -> u32 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileFilterConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

// ---------------------------------------------------------------------------
// Compiled patterns
// ---------------------------------------------------------------------------

/// A compiled pattern ready for matching.
#[derive(Debug, Clone)]
struct CompiledPattern {
    name: String,
    regex: Regex,
    severity: String,
}

fn compile_patterns(entries: &[PatternEntry]) -> Vec<CompiledPattern> {
    entries
        .iter()
        .filter_map(|e| {
            match Regex::new(&e.regex) {
                Ok(re) => Some(CompiledPattern {
                    name: e.name.clone(),
                    regex: re,
                    severity: e.severity.clone(),
                }),
                Err(err) => {
                    error!(pattern = %e.name, error = %err, "Failed to compile regex pattern, skipping");
                    None
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

/// Simple sliding-window rate limiter per minute.
struct RateLimiter {
    max_per_minute: u32,
    timestamps: Vec<Instant>,
}

impl RateLimiter {
    fn new(max_per_minute: u32) -> Self {
        Self {
            max_per_minute,
            timestamps: Vec::new(),
        }
    }

    /// Returns `true` if the notification is allowed.
    fn allow(&mut self) -> bool {
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);
        self.timestamps.retain(|t| *t >= one_minute_ago);
        if (self.timestamps.len() as u32) < self.max_per_minute {
            self.timestamps.push(now);
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration loading
// ---------------------------------------------------------------------------

fn load_config(path: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

// ---------------------------------------------------------------------------
// PID file management
// ---------------------------------------------------------------------------

fn write_pid_file(path: &Path) -> io::Result<()> {
    let pid = std::process::id();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, pid.to_string())?;
    info!(pid = pid, path = %path.display(), "Wrote PID file");
    Ok(())
}

fn remove_pid_file(path: &Path) {
    if path.exists() {
        if let Err(e) = fs::remove_file(path) {
            warn!(error = %e, path = %path.display(), "Failed to remove PID file");
        } else {
            info!(path = %path.display(), "Removed PID file");
        }
    }
}

// ---------------------------------------------------------------------------
// File filter
// ---------------------------------------------------------------------------

fn matches_glob(pattern: &str, filename: &str) -> bool {
    // Simple glob matching: support * and ? wildcards
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let filename_chars: Vec<char> = filename.chars().collect();
    glob_match(&pattern_chars, &filename_chars, 0, 0)
}

fn glob_match(pat: &[char], name: &[char], pi: usize, ni: usize) -> bool {
    if pi == pat.len() && ni == name.len() {
        return true;
    }
    if pi == pat.len() {
        return false;
    }
    if pat[pi] == '*' {
        // Match zero or more characters
        for i in ni..=name.len() {
            if glob_match(pat, name, pi + 1, i) {
                return true;
            }
        }
        false
    } else if ni < name.len() && (pat[pi] == '?' || pat[pi] == name[ni]) {
        glob_match(pat, name, pi + 1, ni + 1)
    } else {
        false
    }
}

fn should_watch_file(filename: &str, filter: &FileFilterConfig) -> bool {
    // Check exclude first
    for pattern in &filter.exclude {
        if matches_glob(pattern, filename) {
            return false;
        }
    }
    // If include patterns exist, file must match at least one
    if filter.include.is_empty() {
        return true;
    }
    for pattern in &filter.include {
        if matches_glob(pattern, filename) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Notification sending
// ---------------------------------------------------------------------------

fn send_notification(
    app_name: &str,
    pattern_name: &str,
    severity: &str,
    file: &str,
    matched_line: &str,
    timeout_ms: u64,
) {
    let urgency = match severity {
        "critical" => notify_rust::Urgency::Critical,
        "low" => notify_rust::Urgency::Low,
        _ => notify_rust::Urgency::Normal,
    };

    let summary = format!("[{}] Pattern '{}' matched", severity.to_uppercase(), pattern_name);
    let body = format!("File: {}\n{}", file, truncate_line(matched_line, 200));

    let result = notify_rust::Notification::new()
        .appname(app_name)
        .summary(&summary)
        .body(&body)
        .urgency(urgency)
        .timeout(if timeout_ms == 0 {
            notify_rust::Timeout::Never
        } else {
            notify_rust::Timeout::Milliseconds(timeout_ms as u32)
        })
        .show();

    match result {
        Ok(_) => debug!(pattern = %pattern_name, file = %file, "Desktop notification sent"),
        Err(e) => warn!(error = %e, "Failed to send desktop notification"),
    }
}

fn truncate_line(line: &str, max_len: usize) -> String {
    if line.len() > max_len {
        format!("{}…", &line[..max_len])
    } else {
        line.to_string()
    }
}

// ---------------------------------------------------------------------------
// Log file scanner
// ---------------------------------------------------------------------------

/// Tracks file offsets so we only scan new content.
struct FileTracker {
    offsets: HashMap<PathBuf, u64>,
}

impl FileTracker {
    fn new() -> Self {
        Self {
            offsets: HashMap::new(),
        }
    }

    /// Scan a file from the last known offset and return newly matched lines.
    fn scan_file(
        &mut self,
        path: &Path,
        patterns: &[CompiledPattern],
    ) -> Vec<(String, String, String)> {
        // Returns Vec<(pattern_name, severity, matched_line)>
        let mut matches = Vec::new();

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to open file for scanning");
                return matches;
            }
        };

        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to read file metadata");
                return matches;
            }
        };

        let file_len = metadata.len();
        let last_offset = self.offsets.get(path).copied().unwrap_or(0);

        // If the file was truncated/rotated, start from the beginning
        let start_offset = if file_len < last_offset { 0 } else { last_offset };

        let mut reader = io::BufReader::new(&file);
        if let Err(e) = reader.seek(SeekFrom::Start(start_offset)) {
            warn!(path = %path.display(), error = %e, "Failed to seek in file");
            return matches;
        }

        let mut current_offset = start_offset;
        let mut line_buf = String::new();

        loop {
            line_buf.clear();
            match reader.read_line(&mut line_buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    current_offset += n as u64;
                    let trimmed = line_buf.trim_end();
                    for pattern in patterns {
                        if pattern.regex.is_match(trimmed) {
                            matches.push((
                                pattern.name.clone(),
                                pattern.severity.clone(),
                                trimmed.to_string(),
                            ));
                            break; // Only report first matching pattern per line
                        }
                    }
                }
                Err(e) => {
                    // Non-UTF8 lines or other read errors
                    debug!(path = %path.display(), error = %e, "Error reading line, skipping");
                    break;
                }
            }
        }

        self.offsets.insert(path.to_path_buf(), current_offset);
        matches
    }
}

// ---------------------------------------------------------------------------
// Logging initialization
// ---------------------------------------------------------------------------

fn init_logging(config: &GeneralConfig) {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    match config.log_format.as_str() {
        "json" => {
            fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(true)
                .init();
        }
        _ => {
            fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_thread_ids(true)
                .init();
        }
    }
}

// ---------------------------------------------------------------------------
// CLI argument parsing (minimal)
// ---------------------------------------------------------------------------

fn parse_args() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();
    let mut config_path = PathBuf::from("/etc/logwatchd/config.toml");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                if i + 1 < args.len() {
                    config_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("Error: --config requires a path argument");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("logwatchd - Log file monitoring daemon");
                println!();
                println!("USAGE:");
                println!("    logwatchd [OPTIONS]");
                println!();
                println!("OPTIONS:");
                println!("    -c, --config <PATH>    Path to config file [default: /etc/logwatchd/config.toml]");
                println!("    -h, --help             Print this help message");
                println!("    -V, --version          Print version information");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("logwatchd {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
    }

    config_path
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let config_path = parse_args();

    // Load initial configuration
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration from {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };

    // Initialize logging
    init_logging(&config.general);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %config_path.display(),
        watch_dir = %config.general.watch_dir.display(),
        "logwatchd starting"
    );

    // Write PID file
    if let Err(e) = write_pid_file(&config.general.pid_file) {
        warn!(error = %e, "Failed to write PID file, continuing anyway");
    }
    let pid_file_path = config.general.pid_file.clone();

    // Shared config behind an RwLock for hot-reload
    let shared_config = Arc::new(RwLock::new(config));

    // Channel for filesystem events
    let (fs_tx, mut fs_rx) = mpsc::channel::<Event>(256);

    // Set up filesystem watcher
    let watcher_config = shared_config.clone();
    let watcher_result = {
        let tx = fs_tx.clone();
        let config_guard = watcher_config.read().await;
        let watch_dir = config_guard.general.watch_dir.clone();
        drop(config_guard);

        let mut watcher: RecommendedWatcher = notify::recommended_watcher(
            move |res: Result<Event, notify::Error>| {
                match res {
                    Ok(event) => {
                        if let Err(e) = tx.blocking_send(event) {
                            eprintln!("Failed to send filesystem event: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Filesystem watcher error: {}", e);
                    }
                }
            },
        )?;

        if watch_dir.exists() {
            watcher.watch(&watch_dir, RecursiveMode::Recursive)?;
            info!(path = %watch_dir.display(), "Watching directory");
        } else {
            error!(path = %watch_dir.display(), "Watch directory does not exist");
            remove_pid_file(&pid_file_path);
            std::process::exit(1);
        }

        Ok::<RecommendedWatcher, notify::Error>(watcher)
    };

    let _watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            error!(error = %e, "Failed to initialize filesystem watcher");
            remove_pid_file(&pid_file_path);
            std::process::exit(1);
        }
    };

    // Register signal handlers
    let mut signals = Signals::new([SIGTERM, SIGINT, SIGHUP])
        .expect("Failed to register signal handlers");

    // Compile initial patterns
    {
        let cfg = shared_config.read().await;
        let compiled = compile_patterns(&cfg.patterns.entries);
        info!(count = compiled.len(), "Compiled error patterns");
    }

    // File tracker for offset management
    let file_tracker = Arc::new(RwLock::new(FileTracker::new()));
    let rate_limiter = Arc::new(RwLock::new(RateLimiter::new(
        shared_config.read().await.notifications.max_per_minute,
    )));

    info!("logwatchd is ready and processing events");

    // Main event loop
    loop {
        tokio::select! {
            // Handle filesystem events
            Some(event) = fs_rx.recv() => {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        let cfg = shared_config.read().await;
                        let patterns = compile_patterns(&cfg.patterns.entries);
                        let filter = cfg.file_filter.clone();
                        let notif_config = cfg.notifications.clone();
                        drop(cfg);

                        for path in &event.paths {
                            // Check if the path is a file and matches our filters
                            if !path.is_file() {
                                continue;
                            }

                            let filename = match path.file_name().and_then(|n| n.to_str()) {
                                Some(name) => name.to_string(),
                                None => continue,
                            };

                            if !should_watch_file(&filename, &filter) {
                                debug!(file = %filename, "Skipping file (filtered out)");
                                continue;
                            }

                            debug!(file = %path.display(), "Scanning file for patterns");

                            let matches = {
                                let mut tracker = file_tracker.write().await;
                                tracker.scan_file(path, &patterns)
                            };

                            for (pattern_name, severity, matched_line) in matches {
                                info!(
                                    pattern = %pattern_name,
                                    severity = %severity,
                                    file = %path.display(),
                                    "Pattern matched"
                                );

                                if notif_config.enabled {
                                    let mut limiter = rate_limiter.write().await;
                                    if limiter.allow() {
                                        send_notification(
                                            &notif_config.app_name,
                                            &pattern_name,
                                            &severity,
                                            &path.display().to_string(),
                                            &matched_line,
                                            notif_config.timeout_ms,
                                        );
                                    } else {
                                        warn!("Rate limit exceeded, notification suppressed");
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        // Ignore other event types (Remove, Access, etc.)
                    }
                }
            }

            // Handle signals
            Some(signal) = signals.next() => {
                match signal {
                    SIGTERM | SIGINT => {
                        info!(signal = signal, "Received shutdown signal, shutting down gracefully");
                        break;
                    }
                    SIGHUP => {
                        info!("Received SIGHUP, reloading configuration");
                        match load_config(&config_path) {
                            Ok(new_config) => {
                                let compiled = compile_patterns(&new_config.patterns.entries);
                                info!(
                                    patterns = compiled.len(),
                                    watch_dir = %new_config.general.watch_dir.display(),
                                    "Configuration reloaded successfully"
                                );

                                // Update rate limiter
                                {
                                    let mut limiter = rate_limiter.write().await;
                                    *limiter = RateLimiter::new(new_config.notifications.max_per_minute);
                                }

                                // Update shared config
                                {
                                    let mut cfg = shared_config.write().await;
                                    *cfg = new_config;
                                }
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to reload configuration, keeping current config");
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    // Graceful shutdown
    info!("Shutting down logwatchd");
    remove_pid_file(&pid_file_path);
    info!("logwatchd stopped");
}
