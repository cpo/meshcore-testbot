//! Environment-driven settings (`MESHCORE_*`).

use std::env;
use std::sync::OnceLock;

pub const DEFAULT_REPLY_TEXT: &str = "Set DEFAULT_REPLY_TEXT please";

static REPLY_TEXT: OnceLock<String> = OnceLock::new();

/// Configured by `MESHCORE_REPLY_TEXT` (non-empty after trim); default [`DEFAULT_REPLY_TEXT`].
pub fn reply_location_text() -> &'static str {
    REPLY_TEXT
        .get_or_init(|| {
            env::var("MESHCORE_REPLY_TEXT")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_REPLY_TEXT.to_string())
        })
        .as_str()
}

pub fn meshcore_log_all_enabled() -> bool {
    env::var("MESHCORE_LOGALL")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
}

/// Channel reply/trigger logic runs only when set to a non-empty value (default: off; visor always runs).
pub fn is_bot_enabled() -> bool {
    env::var("MESHCORE_BOT_ENABLED")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
}

pub fn poll_interval_secs() -> u64 {
    env::var("MESHCORE_POLL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3)
}

/// Interval voor kaart-API-contactlijst (standaard 5 minuten).
pub fn contact_resync_interval_secs() -> u64 {
    env::var("MESHCORE_CONTACT_SYNC_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&s| s > 0)
        .unwrap_or(300)
}
