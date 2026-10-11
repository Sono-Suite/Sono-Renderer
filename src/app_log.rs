//! Bounded process-wide log capture used by the optional hosted admin UI.
//!
//! The logger is installed only when the admin service is enabled. Render-job
//! lifecycle history remains owned by the job manager; these records cover
//! process diagnostics emitted through the `log` facade during this process.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const PROCESS_LOG_CAPACITY: usize = 2_000;
const MAX_TARGET_BYTES: usize = 256;
const MAX_MESSAGE_BYTES: usize = 4_096;

static GLOBAL_STORE: OnceLock<ProcessLogStore> = OnceLock::new();
static LOGGER_INSTALL: OnceLock<bool> = OnceLock::new();
static SECRET_ENV_VALUES: OnceLock<Vec<String>> = OnceLock::new();
static LOGGER: ProcessLogger = ProcessLogger;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLogRecord {
    pub sequence: u64,
    pub unix_time_ms: u128,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessLogStore {
    inner: Arc<Mutex<VecDeque<ProcessLogRecord>>>,
    next_sequence: Arc<AtomicU64>,
}

impl ProcessLogStore {
    /// Records a sanitized, size-bounded event. Intended for the process
    /// logger and focused integration tests.
    pub fn record(&self, level: &str, target: &str, message: &str) {
        let mut records = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record = make_record(
            self.next_sequence.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            level,
            target,
            message,
            secret_env_values(),
        );
        if records.len() == PROCESS_LOG_CAPACITY {
            records.pop_front();
        }
        records.push_back(record);
    }

    /// Returns the newest `limit` events in chronological order.
    pub fn recent(&self, limit: usize) -> Vec<ProcessLogRecord> {
        let records = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let start = records.len().saturating_sub(limit);
        records.iter().skip(start).cloned().collect()
    }
}

pub fn global_store() -> ProcessLogStore {
    GLOBAL_STORE.get_or_init(ProcessLogStore::default).clone()
}

/// Installs bounded in-memory capture for standard `log` events. A different
/// logger installed by an embedding host is left intact; admin itself remains
/// available, but cannot promise to collect that host's process log stream.
pub fn initialize() -> bool {
    *LOGGER_INSTALL.get_or_init(|| {
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
            true
        } else {
            false
        }
    })
}

struct ProcessLogger;

impl log::Log for ProcessLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            global_store().record(
                &record.level().to_string().to_ascii_lowercase(),
                record.target(),
                &record.args().to_string(),
            );
        }
    }

    fn flush(&self) {}
}

pub(crate) fn redact(value: &str) -> String {
    redact_with_secrets(value, secret_env_values())
}

fn secret_env_values() -> &'static [String] {
    SECRET_ENV_VALUES.get_or_init(|| {
        std::env::vars_os()
            .filter_map(|(key, secret)| {
                let key = key.to_string_lossy().to_ascii_lowercase();
                let is_secret = [
                    "token",
                    "secret",
                    "password",
                    "webhook",
                    "credential",
                    "private_key",
                    "api_key",
                    "access_key",
                    "secret_key",
                ]
                .iter()
                .any(|part| key.contains(part));
                let secret = secret.to_string_lossy().into_owned();
                (is_secret && !secret.is_empty()).then_some(secret)
            })
            .collect::<Vec<_>>()
    })
}

pub(crate) fn redact_with_secrets(value: &str, secrets: &[String]) -> String {
    let mut value = value.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            value = value.replace(secret, "[redacted]");
        }
    }
    value
}

fn bounded_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let mut end = maximum_bytes.saturating_sub('…'.len_utf8());
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}…", &value[..end])
}

fn make_record(
    sequence: u64,
    unix_time_ms: u128,
    level: &str,
    target: &str,
    message: &str,
    secrets: &[String],
) -> ProcessLogRecord {
    ProcessLogRecord {
        sequence,
        unix_time_ms,
        level: bounded_text(&redact_with_secrets(level, secrets), 16),
        target: bounded_text(&redact_with_secrets(target, secrets), MAX_TARGET_BYTES),
        message: bounded_text(&redact_with_secrets(message, secrets), MAX_MESSAGE_BYTES),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_log_store_is_bounded_and_returns_newest_in_order() {
        let store = ProcessLogStore::default();
        for index in 0..PROCESS_LOG_CAPACITY + 3 {
            store.record("info", "test", &index.to_string());
        }
        let records = store.recent(2);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].message, (PROCESS_LOG_CAPACITY + 1).to_string());
        assert_eq!(records[1].message, (PROCESS_LOG_CAPACITY + 2).to_string());
        assert_eq!(
            store.recent(PROCESS_LOG_CAPACITY + 10).len(),
            PROCESS_LOG_CAPACITY
        );
    }

    #[test]
    fn process_log_store_redacts_known_secret_values() {
        let record = make_record(
            0,
            1,
            "warn",
            "test",
            "token=sensitive",
            &["sensitive".to_owned()],
        );
        assert_eq!(record.message, "token=[redacted]");
    }

    #[test]
    fn process_log_store_bounds_untrusted_unicode_text_at_character_boundaries() {
        let store = ProcessLogStore::default();
        store.record("info", "t", &"界".repeat(2_000));
        let message = &store.recent(1)[0].message;
        assert!(message.len() <= MAX_MESSAGE_BYTES);
        assert!(message.is_char_boundary(message.len()));
        assert!(message.ends_with('…'));
    }
}
