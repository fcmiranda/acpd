// notifier.rs — Desktop notification sender for logwatchd.
//
// Assumptions:
// - Uses the freedesktop.org Desktop Notifications Specification over D-Bus.
// - Falls back gracefully if D-Bus is unavailable (logs a warning instead of crashing).
// - Rate limiting prevents notification storms when many errors appear at once.
// - The daemon may run as a system service, so D-Bus session access might require
//   DBUS_SESSION_BUS_ADDRESS to be set in the systemd unit file.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;

/// Rate limiter that tracks notification timestamps in a sliding window.
pub struct RateLimiter {
    max_per_minute: u32,
    timestamps: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            max_per_minute,
            timestamps: Mutex::new(VecDeque::new()),
        }
    }

    /// Returns true if a notification is allowed under the rate limit.
    pub fn check(&self) -> bool {
        if self.max_per_minute == 0 {
            return true; // Unlimited
        }

        let mut timestamps = self.timestamps.lock().unwrap();
        let now = Instant::now();
        let one_minute_ago = now - std::time::Duration::from_secs(60);

        // Remove timestamps older than 1 minute
        while timestamps.front().map_or(false, |&t| t < one_minute_ago) {
            timestamps.pop_front();
        }

        if timestamps.len() < self.max_per_minute as usize {
            timestamps.push_back(now);
            true
        } else {
            false
        }
    }
}

/// Notification urgency levels matching the freedesktop spec.
#[derive(Debug, Clone, Copy)]
pub enum Urgency {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

impl From<&str> for Urgency {
    fn from(s: &str) -> Self {
        match s {
            "low" => Urgency::Low,
            "normal" => Urgency::Normal,
            "critical" => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }
}

/// Send a desktop notification via D-Bus using the org.freedesktop.Notifications interface.
///
/// This uses zbus to make an async D-Bus method call. If D-Bus is not available,
/// the error is logged but does not crash the daemon.
pub async fn send_notification(
    app_name: &str,
    summary: &str,
    body: &str,
    urgency: Urgency,
    timeout_ms: i32,
) -> Result<()> {
    // Connect to the session bus
    let connection = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to connect to D-Bus session bus; notification not sent"
            );
            return Ok(());
        }
    };

    // Call org.freedesktop.Notifications.Notify
    // Signature: Notify(app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout)
    let msg = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                app_name,
                0u32,          // replaces_id — 0 means don't replace
                "",            // app_icon — empty for default
                summary,
                body,
                &[] as &[&str],  // actions
                &std::collections::HashMap::from([
                    ("urgency", zbus::zvariant::Value::from(urgency as u8)),
                ]),
                timeout_ms,
            ),
        )
        .await;

    match msg {
        Ok(_) => {
            tracing::debug!(summary, "desktop notification sent");
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                summary,
                "failed to send desktop notification"
            );
        }
    }

    Ok(())
}
