//! Durable history for completed, failed, and cancelled render attempts.
//!
//! Queue state remains owned by `job_manager`; this module stores immutable,
//! redacted snapshots of terminal attempts as one atomic JSON file per attempt.

use crate::render::RenderConfig;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const HISTORY_FORMAT: &str = "sono-render-history";
pub const HISTORY_SCHEMA_VERSION: u32 = 1;
pub const RETENTION_FORMAT: &str = "sono-render-history-retention";
pub const RETENTION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_MAX_ENTRIES: usize = 1_000;
const SETTINGS_FILE: &str = "retention.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryInputIdentity {
    pub role: String,
    pub path: PathBuf,
    pub exists: bool,
    pub size_bytes: Option<u64>,
    pub modified_unix_time_ms: Option<u128>,
}

impl Default for HistoryInputIdentity {
    fn default() -> Self {
        Self {
            role: String::new(),
            path: PathBuf::new(),
            exists: false,
            size_bytes: None,
            modified_unix_time_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryOutputMetadata {
    pub path: Option<PathBuf>,
    pub exists: bool,
    pub size_bytes: Option<u64>,
}

impl Default for HistoryOutputMetadata {
    fn default() -> Self {
        Self {
            path: None,
            exists: false,
            size_bytes: None,
        }
    }
}

/// A safe, self-contained description of one execution attempt. `extra`
/// preserves fields written by newer minor schemas when an older client pins
/// or exports the record. The configuration contains credential references,
/// never credential values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderHistoryRecord {
    pub format: String,
    pub schema_version: u32,
    pub id: String,
    pub job_id: String,
    pub attempt: u32,
    pub project_path: Option<PathBuf>,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub original_job_id: Option<String>,
    pub retried_from_history_id: Option<String>,
    pub rerendered_from_history_id: Option<String>,
    #[serde(default)]
    pub reproduced_from_manifest_sha256: Option<String>,
    pub renderer_version: String,
    pub operating_system: String,
    pub architecture: String,
    pub execution_backend: String,
    pub status: HistoryStatus,
    pub started_unix_time_ms: Option<u128>,
    pub completed_unix_time_ms: u128,
    pub config_snapshot: RenderConfig,
    pub input_identities: Vec<HistoryInputIdentity>,
    pub output: HistoryOutputMetadata,
    #[serde(default)]
    pub rgb_sequence_sha1: Option<String>,
    #[serde(default)]
    pub sfx_event_sha1: Option<String>,
    #[serde(default)]
    pub sfx_pcm_sha1: Option<String>,
    pub performance_record_id: Option<String>,
    pub diagnostic_record_id: Option<String>,
    pub effective_backend_status: Option<String>,
    pub cache_state: Option<String>,
    pub fallback: Option<String>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
    pub pinned: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for RenderHistoryRecord {
    fn default() -> Self {
        Self {
            format: HISTORY_FORMAT.into(),
            schema_version: 0,
            id: String::new(),
            job_id: String::new(),
            attempt: 0,
            project_path: None,
            profile_id: None,
            profile_name: None,
            original_job_id: None,
            retried_from_history_id: None,
            rerendered_from_history_id: None,
            reproduced_from_manifest_sha256: None,
            renderer_version: String::new(),
            operating_system: String::new(),
            architecture: String::new(),
            execution_backend: String::new(),
            status: HistoryStatus::Failed,
            started_unix_time_ms: None,
            completed_unix_time_ms: 0,
            config_snapshot: RenderConfig::default(),
            input_identities: Vec::new(),
            output: HistoryOutputMetadata::default(),
            rgb_sequence_sha1: None,
            sfx_event_sha1: None,
            sfx_pcm_sha1: None,
            performance_record_id: None,
            diagnostic_record_id: None,
            effective_backend_status: None,
            cache_state: None,
            fallback: None,
            warnings: Vec::new(),
            error: None,
            pinned: false,
            extra: BTreeMap::new(),
        }
    }
}

impl RenderHistoryRecord {
    pub fn validate(&self) -> Result<()> {
        if self.format != HISTORY_FORMAT {
            bail!("unsupported render history format {:?}", self.format);
        }
        if self.schema_version != HISTORY_SCHEMA_VERSION {
            bail!(
                "unsupported render history schema version {}",
                self.schema_version
            );
        }
        validate_id(&self.id)?;
        if self.job_id.is_empty() {
            bail!("render history entry has an empty job ID");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryRetentionPolicy {
    pub format: String,
    pub schema_version: u32,
    pub max_entries: usize,
    pub max_age_days: Option<u64>,
    pub keep_failures: bool,
    pub keep_pinned: bool,
}

impl Default for HistoryRetentionPolicy {
    fn default() -> Self {
        Self {
            format: RETENTION_FORMAT.into(),
            schema_version: RETENTION_SCHEMA_VERSION,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_age_days: None,
            keep_failures: true,
            keep_pinned: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub text: Option<String>,
    pub status: Option<HistoryStatus>,
    pub engine: Option<String>,
    pub project: Option<String>,
    pub backend: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub entries: Vec<RenderHistoryRecord>,
    pub issues: Vec<HistoryIssue>,
}

#[derive(Debug, Clone)]
pub struct RenderHistoryStore {
    root: PathBuf,
}

impl RenderHistoryStore {
    pub fn open_default() -> Result<Self> {
        Self::open_at(default_store_path())
    }

    pub fn open_at(path: impl Into<PathBuf>) -> Result<Self> {
        let root = path.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating render history {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save_immutable(&self, record: &RenderHistoryRecord) -> Result<()> {
        record.validate()?;
        let path = self.record_path(&record.id)?;
        if path.exists() {
            return Ok(());
        }
        let bytes = serde_json::to_vec_pretty(record)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".history-")
            .suffix(".tmp")
            .tempfile_in(&self.root)
            .context("creating atomic render history entry")?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {}
            Err(_error) if path.exists() => {}
            Err(error) => {
                return Err(error.error)
                    .with_context(|| format!("publishing render history entry {}", path.display()))
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<RenderHistoryRecord> {
        let path = self.record_path(id)?;
        let bytes =
            fs::read(&path).with_context(|| format!("reading history entry {}", path.display()))?;
        decode_record(&bytes).with_context(|| format!("decoding history entry {}", path.display()))
    }

    pub fn list(&self, query: &HistoryQuery) -> HistoryPage {
        let mut entries = Vec::new();
        let mut issues = Vec::new();
        let read = match fs::read_dir(&self.root) {
            Ok(read) => read,
            Err(error) => {
                return HistoryPage {
                    entries,
                    issues: vec![HistoryIssue {
                        path: self.root.clone(),
                        message: error.to_string(),
                    }],
                }
            }
        };
        let settings = self.root.join(SETTINGS_FILE);
        if settings.exists() {
            if let Err(error) = self.retention() {
                issues.push(HistoryIssue {
                    path: settings,
                    message: format!("{error:#}"),
                });
            }
        }
        for item in read {
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    issues.push(HistoryIssue {
                        path: self.root.clone(),
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            let path = item.path();
            if path.extension().is_none_or(|extension| extension != "json")
                || path.file_name().is_some_and(|name| name == SETTINGS_FILE)
            {
                continue;
            }
            let decoded = fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| decode_record(&bytes));
            match decoded {
                Ok(entry) if matches_query(&entry, query) => entries.push(entry),
                Ok(_) => {}
                Err(error) => issues.push(HistoryIssue {
                    path,
                    message: format!("{error:#}"),
                }),
            }
        }
        entries.sort_by(|left, right| {
            (right.completed_unix_time_ms, &right.id).cmp(&(left.completed_unix_time_ms, &left.id))
        });
        entries.truncate(query.limit.unwrap_or(usize::MAX));
        HistoryPage { entries, issues }
    }

    pub fn retention(&self) -> Result<HistoryRetentionPolicy> {
        let path = self.root.join(SETTINGS_FILE);
        if !path.exists() {
            return Ok(HistoryRetentionPolicy::default());
        }
        let policy: HistoryRetentionPolicy = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("decoding history retention settings {}", path.display()))?;
        if policy.format != RETENTION_FORMAT || policy.schema_version != RETENTION_SCHEMA_VERSION {
            bail!(
                "unsupported render history retention schema {}",
                policy.schema_version
            );
        }
        if !(1..=100_000).contains(&policy.max_entries) {
            bail!("history max_entries must be in 1..=100000");
        }
        Ok(policy)
    }

    pub fn set_retention(&self, policy: &HistoryRetentionPolicy) -> Result<usize> {
        if !(1..=100_000).contains(&policy.max_entries) {
            bail!("history max_entries must be in 1..=100000");
        }
        let path = self.root.join(SETTINGS_FILE);
        write_atomic_replace(&path, &serde_json::to_vec_pretty(policy)?)?;
        self.prune()
    }

    pub fn pin(&self, id: &str, pinned: bool) -> Result<RenderHistoryRecord> {
        let mut record = self.get(id)?;
        record.pinned = pinned;
        let path = self.record_path(id)?;
        write_atomic_replace(&path, &serde_json::to_vec_pretty(&record)?)?;
        Ok(record)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.record_path(id)?;
        fs::remove_file(&path).with_context(|| format!("deleting history entry {id}"))
    }

    pub fn clear(&self, keep_failures: bool) -> Result<usize> {
        let page = self.list(&HistoryQuery::default());
        let mut removed = 0;
        for entry in page.entries {
            if entry.pinned || keep_failures && entry.status == HistoryStatus::Failed {
                continue;
            }
            self.delete(&entry.id)?;
            removed += 1;
        }
        Ok(removed)
    }

    pub fn prune(&self) -> Result<usize> {
        let policy = self.retention().unwrap_or_default();
        let page = self.list(&HistoryQuery::default());
        let now = unix_time_ms();
        let cutoff = policy
            .max_age_days
            .map(|days| now.saturating_sub(days as u128 * 86_400_000));
        let protected = page
            .entries
            .iter()
            .filter(|entry| {
                policy.keep_pinned && entry.pinned
                    || policy.keep_failures && entry.status == HistoryStatus::Failed
            })
            .map(|entry| entry.id.clone())
            .collect::<std::collections::HashSet<_>>();
        let ordinary = page
            .entries
            .iter()
            .filter(|entry| !protected.contains(&entry.id))
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let allowed_ordinary = policy.max_entries.saturating_sub(protected.len());
        let mut removed = 0;
        for entry in &page.entries {
            if protected.contains(&entry.id) {
                continue;
            }
            let too_old = cutoff.is_some_and(|cutoff| entry.completed_unix_time_ms < cutoff);
            let too_many = ordinary
                .iter()
                .position(|candidate| candidate == &entry.id)
                .is_some_and(|index| index >= allowed_ordinary);
            if too_old || too_many {
                self.delete(&entry.id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn record_path(&self, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }
}

pub fn history_id(job_id: &str, attempt: u32) -> String {
    let safe = job_id
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || byte == b'-' {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("history-{}-attempt-{attempt}", safe.trim_matches('-'))
}

pub fn sanitize_config_snapshot(config: &RenderConfig) -> RenderConfig {
    let mut snapshot = config.clone();
    let key = snapshot.notifications.discord.bot_token_env.as_str();
    let valid_reference = !key.is_empty()
        && key.chars().enumerate().all(|(index, ch)| {
            ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
        });
    if !valid_reference {
        snapshot.notifications.discord.bot_token_env =
            crate::notifications::DiscordNotificationConfig::default().bot_token_env;
    }
    snapshot
}

pub fn missing_inputs(config: &RenderConfig) -> Vec<(String, PathBuf)> {
    input_paths(config)
        .into_iter()
        .filter(|(role, path)| {
            let enabled = match role.as_str() {
                "music" => config.layers.bgm,
                "MV" => config.mv_enabled,
                _ => true,
            };
            enabled && !path.is_file()
        })
        .collect()
}

pub fn save_history_export_atomic(path: &Path, record: &RenderHistoryRecord) -> Result<()> {
    record.validate()?;
    if path.exists() {
        bail!(
            "refusing to replace existing history export {}",
            path.display()
        );
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(record)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".history-export-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    match temp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(_error) if path.exists() => bail!(
            "refusing to replace existing history export {}",
            path.display()
        ),
        Err(error) => Err(error.error)
            .with_context(|| format!("publishing history export {}", path.display())),
    }
}

pub(crate) fn input_identities(config: &RenderConfig) -> Vec<HistoryInputIdentity> {
    input_paths(config)
        .into_iter()
        .map(|(role, path)| {
            let metadata = fs::metadata(&path).ok();
            let modified_unix_time_ms = metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis());
            HistoryInputIdentity {
                role,
                path,
                exists: metadata.is_some(),
                size_bytes: metadata.map(|metadata| metadata.len()),
                modified_unix_time_ms,
            }
        })
        .collect()
}

fn input_paths(config: &RenderConfig) -> Vec<(String, PathBuf)> {
    let mut inputs = vec![
        ("engine".into(), config.engine.clone()),
        ("resources".into(), config.resources.clone()),
        ("level".into(), config.level.clone()),
    ];
    for (role, path) in [
        ("music", config.music.as_ref()),
        ("MV", config.mv.as_ref()),
        (
            "particle resources",
            config.resource_overrides.particles.as_ref(),
        ),
        (
            "effects resources",
            config.resource_overrides.effects.as_ref(),
        ),
        (
            "background resources",
            config.resource_overrides.background.as_ref(),
        ),
    ] {
        if let Some(path) = path {
            inputs.push((role.into(), path.clone()));
        }
    }
    inputs
}

fn matches_query(record: &RenderHistoryRecord, query: &HistoryQuery) -> bool {
    if query.status.is_some_and(|status| record.status != status) {
        return false;
    }
    if query.engine.as_ref().is_some_and(|engine| {
        !record
            .config_snapshot
            .engine
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains(&engine.to_ascii_lowercase())
    }) {
        return false;
    }
    if query.project.as_ref().is_some_and(|project| {
        !record.project_path.as_ref().is_some_and(|path| {
            path.to_string_lossy()
                .to_ascii_lowercase()
                .contains(&project.to_ascii_lowercase())
        })
    }) {
        return false;
    }
    if query.backend.as_ref().is_some_and(|backend| {
        !record
            .execution_backend
            .to_ascii_lowercase()
            .contains(&backend.to_ascii_lowercase())
    }) {
        return false;
    }
    let Some(text) = query.text.as_deref().filter(|text| !text.trim().is_empty()) else {
        return true;
    };
    let text = text.to_ascii_lowercase();
    [
        record.id.as_str(),
        record.job_id.as_str(),
        record.profile_name.as_deref().unwrap_or(""),
        record.execution_backend.as_str(),
        record.config_snapshot.engine.to_string_lossy().as_ref(),
        record.config_snapshot.level.to_string_lossy().as_ref(),
        record
            .output
            .path
            .as_ref()
            .map(|path| path.to_string_lossy())
            .as_deref()
            .unwrap_or(""),
        record.error.as_deref().unwrap_or(""),
    ]
    .iter()
    .any(|candidate| candidate.to_ascii_lowercase().contains(&text))
}

fn decode_record(bytes: &[u8]) -> Result<RenderHistoryRecord> {
    let mut record: RenderHistoryRecord = serde_json::from_slice(bytes)?;
    if record.format != HISTORY_FORMAT {
        bail!("unsupported render history format {:?}", record.format);
    }
    // Version zero was the initial prototype shape; absent additive fields
    // receive defaults and are promoted without dropping flattened fields.
    if record.schema_version == 0 {
        record.schema_version = HISTORY_SCHEMA_VERSION;
    }
    record.validate()?;
    Ok(record)
}

fn write_atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("history path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".history-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("atomically replacing {}", path.display()))?;
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 180
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("invalid render history identifier");
    }
    Ok(())
}

pub(crate) fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn default_store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_HISTORY_DIR") {
        return PathBuf::from(path);
    }
    crate::store_paths::default_history_store_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn durable_config(config: &RenderConfig) -> RenderConfig {
        serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap()
    }

    fn record(id: &str) -> RenderHistoryRecord {
        RenderHistoryRecord {
            id: id.into(),
            job_id: "job-a".into(),
            schema_version: HISTORY_SCHEMA_VERSION,
            completed_unix_time_ms: 10,
            ..Default::default()
        }
    }

    #[test]
    fn records_round_trip_and_keep_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = RenderHistoryStore::open_at(dir.path()).unwrap();
        let mut entry = record("h-1");
        entry
            .extra
            .insert("future_note".into(), Value::String("kept".into()));
        store.save_immutable(&entry).unwrap();
        let loaded = store.get("h-1").unwrap();
        assert_eq!(loaded.id, entry.id);
        assert_eq!(loaded.extra, entry.extra);
        assert_eq!(
            durable_config(&loaded.config_snapshot),
            durable_config(&entry.config_snapshot)
        );
        let stored: Value =
            serde_json::from_slice(&fs::read(store.root().join("h-1.json")).unwrap()).unwrap();
        assert!(stored
            .pointer("/config_snapshot/ui/profile_directory")
            .is_none());
    }

    #[test]
    fn corrupt_entries_are_reported_without_hiding_valid_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = RenderHistoryStore::open_at(dir.path()).unwrap();
        store.save_immutable(&record("h-good")).unwrap();
        fs::write(dir.path().join("h-bad.json"), b"{").unwrap();
        let page = store.list(&HistoryQuery::default());
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.issues.len(), 1);
    }

    #[test]
    fn schema_zero_is_migrated_and_future_schema_is_isolated_as_an_issue() {
        let mut old = serde_json::to_value(record("h-old")).unwrap();
        old["schema_version"] = Value::from(0);
        assert_eq!(
            decode_record(&serde_json::to_vec(&old).unwrap())
                .unwrap()
                .schema_version,
            HISTORY_SCHEMA_VERSION
        );
        let dir = tempfile::tempdir().unwrap();
        let store = RenderHistoryStore::open_at(dir.path()).unwrap();
        let mut future = record("h-future");
        future.schema_version = HISTORY_SCHEMA_VERSION + 1;
        fs::write(
            dir.path().join("h-future.json"),
            serde_json::to_vec(&future).unwrap(),
        )
        .unwrap();
        assert_eq!(store.list(&HistoryQuery::default()).issues.len(), 1);
    }

    #[test]
    fn secret_like_token_reference_is_removed_and_retention_keeps_failures_and_pins() {
        let mut config = RenderConfig::default();
        config.notifications.discord.bot_token_env = "eyJhbGciOi.secret.value".into();
        assert_eq!(
            sanitize_config_snapshot(&config)
                .notifications
                .discord
                .bot_token_env,
            crate::notifications::DiscordNotificationConfig::default().bot_token_env
        );
        let dir = tempfile::tempdir().unwrap();
        let store = RenderHistoryStore::open_at(dir.path()).unwrap();
        let mut a = record("h-a");
        a.completed_unix_time_ms = 3;
        a.status = HistoryStatus::Completed;
        let mut b = record("h-b");
        b.completed_unix_time_ms = 2;
        b.status = HistoryStatus::Failed;
        let mut c = record("h-c");
        c.completed_unix_time_ms = 1;
        c.pinned = true;
        for entry in [&a, &b, &c] {
            store.save_immutable(entry).unwrap();
        }
        let removed = store
            .set_retention(&HistoryRetentionPolicy {
                max_entries: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(store.list(&HistoryQuery::default()).entries.len(), 2);
    }

    #[test]
    fn text_and_status_filters_search_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = RenderHistoryStore::open_at(dir.path()).unwrap();
        let mut entry = record("h-1");
        entry.profile_name = Some("Studio".into());
        entry.status = HistoryStatus::Completed;
        store.save_immutable(&entry).unwrap();
        assert_eq!(
            store
                .list(&HistoryQuery {
                    text: Some("studio".into()),
                    status: Some(HistoryStatus::Completed),
                    ..Default::default()
                })
                .entries
                .len(),
            1
        );
        assert_eq!(
            store
                .list(&HistoryQuery {
                    status: Some(HistoryStatus::Failed),
                    ..Default::default()
                })
                .entries
                .len(),
            0
        );
    }
}
