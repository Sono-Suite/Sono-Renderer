//! Named, persistent snapshots of the shared user-facing renderer config.

use crate::render::RenderConfig;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
const PROFILE_FORMAT: &str = "sono-renderer-config-profile";
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30);
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigProfile {
    pub id: String,
    pub name: String,
    pub created_unix_time_ms: u128,
    pub updated_unix_time_ms: u128,
    pub config: RenderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportedConfigProfile {
    pub name: String,
    pub config: RenderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfigProfileExport {
    pub format: String,
    pub schema_version: u32,
    pub profile: ExportedConfigProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDocument {
    schema_version: u32,
    #[serde(default)]
    active_profile_id: Option<String>,
    #[serde(default)]
    profiles: Vec<ConfigProfile>,
}

impl Default for ProfileDocument {
    fn default() -> Self {
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            active_profile_id: None,
            profiles: Vec::new(),
        }
    }
}

/// File-backed profile catalog shared by the desktop and CLI.
#[derive(Debug, Clone)]
pub struct ConfigProfileManager {
    path: PathBuf,
}

impl ConfigProfileManager {
    pub fn open_default() -> Result<Self> {
        Self::open_at(default_profile_store_path())
    }

    pub fn open_at(path: impl Into<PathBuf>) -> Result<Self> {
        let manager = Self { path: path.into() };
        manager.read(|_| Ok(()))?;
        Ok(manager)
    }

    pub fn store_path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Result<Vec<ConfigProfile>> {
        self.read(|document| {
            let mut profiles = document.profiles.clone();
            profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            Ok(profiles)
        })
    }

    pub fn get(&self, selector: &str) -> Result<ConfigProfile> {
        self.read(|document| Ok(find_profile(document, selector).cloned()))?
            .with_context(|| format!("configuration profile {selector:?} was not found"))
    }

    pub fn resolve(&self, selector: &str) -> Result<RenderConfig> {
        let profile = self.get(selector)?;
        profile
            .config
            .validate()
            .with_context(|| format!("configuration profile {:?} is invalid", profile.name))?;
        Ok(profile.config)
    }

    pub fn active(&self) -> Result<Option<ConfigProfile>> {
        self.read(|document| {
            Ok(document
                .active_profile_id
                .as_deref()
                .and_then(|id| document.profiles.iter().find(|profile| profile.id == id))
                .cloned())
        })
    }

    pub fn select(&self, selector: &str) -> Result<ConfigProfile> {
        self.mutate(|document| {
            let profile = find_profile(document, selector)
                .with_context(|| format!("configuration profile {selector:?} was not found"))?
                .clone();
            document.active_profile_id = Some(profile.id.clone());
            Ok(profile)
        })
    }

    pub fn select_current(&self) -> Result<()> {
        self.mutate(|document| {
            document.active_profile_id = None;
            Ok(())
        })
    }

    pub fn create(&self, name: &str, config: RenderConfig) -> Result<ConfigProfile> {
        let name = validate_name(name)?;
        let config = normalize_profile_config(config)?;
        self.mutate(|document| {
            ensure_unique_name(document, &name, None)?;
            let now = unix_time_ms();
            let id = unique_profile_id(document);
            let profile = ConfigProfile {
                id,
                name,
                created_unix_time_ms: now,
                updated_unix_time_ms: now,
                config,
            };
            document.profiles.push(profile.clone());
            Ok(profile)
        })
    }

    pub fn update(&self, selector: &str, config: RenderConfig) -> Result<ConfigProfile> {
        let config = normalize_profile_config(config)?;
        self.mutate(|document| {
            let index = profile_index(document, selector)?;
            document.profiles[index].config = config;
            document.profiles[index].updated_unix_time_ms = unix_time_ms();
            Ok(document.profiles[index].clone())
        })
    }

    pub fn duplicate(&self, selector: &str, name: &str) -> Result<ConfigProfile> {
        let name = validate_name(name)?;
        self.mutate(|document| {
            ensure_unique_name(document, &name, None)?;
            let source = find_profile(document, selector)
                .with_context(|| format!("configuration profile {selector:?} was not found"))?
                .clone();
            let now = unix_time_ms();
            let profile = ConfigProfile {
                id: unique_profile_id(document),
                name,
                created_unix_time_ms: now,
                updated_unix_time_ms: now,
                config: source.config,
            };
            document.profiles.push(profile.clone());
            Ok(profile)
        })
    }

    pub fn rename(&self, selector: &str, new_name: &str) -> Result<ConfigProfile> {
        let new_name = validate_name(new_name)?;
        self.mutate(|document| {
            let index = profile_index(document, selector)?;
            let id = document.profiles[index].id.clone();
            ensure_unique_name(document, &new_name, Some(&id))?;
            document.profiles[index].name = new_name;
            document.profiles[index].updated_unix_time_ms = unix_time_ms();
            Ok(document.profiles[index].clone())
        })
    }

    pub fn reset(&self, selector: &str) -> Result<ConfigProfile> {
        self.update(selector, RenderConfig::default())
    }

    pub fn delete(&self, selector: &str) -> Result<ConfigProfile> {
        self.mutate(|document| {
            let index = profile_index(document, selector)?;
            let removed = document.profiles.remove(index);
            if document.active_profile_id.as_deref() == Some(&removed.id) {
                document.active_profile_id = None;
            }
            Ok(removed)
        })
    }

    pub fn export(&self, selector: &str, path: impl AsRef<Path>) -> Result<()> {
        let profile = self.get(selector)?;
        let mut config = profile.config;
        let path = path.as_ref();
        let base = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| {
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new("."))
                    .to_path_buf()
            });
        config.relativize_config_paths(&base);
        scrub_secret_references(&mut config);
        let export = ConfigProfileExport {
            format: PROFILE_FORMAT.into(),
            schema_version: PROFILE_SCHEMA_VERSION,
            profile: ExportedConfigProfile {
                name: profile.name,
                config,
            },
        };
        write_json_atomic(path, &export)
            .with_context(|| format!("exporting configuration profile to {}", path.display()))
    }

    pub fn import(
        &self,
        path: impl AsRef<Path>,
        name_override: Option<&str>,
    ) -> Result<ConfigProfile> {
        let path = path.as_ref();
        let bytes = fs::read(path)
            .with_context(|| format!("reading configuration profile export {}", path.display()))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing configuration profile export {}", path.display()))?;
        reject_unknown_config_fields(
            value.get("profile").and_then(|value| value.get("config")),
            "profile.config",
        )?;
        let mut export: ConfigProfileExport = serde_json::from_value(value)
            .with_context(|| format!("parsing configuration profile export {}", path.display()))?;
        if export.format != PROFILE_FORMAT {
            bail!(
                "unsupported configuration profile format {:?}",
                export.format
            );
        }
        if export.schema_version != PROFILE_SCHEMA_VERSION {
            bail!(
                "unsupported configuration profile schema version {}; this renderer supports version {}",
                export.schema_version,
                PROFILE_SCHEMA_VERSION
            );
        }
        let name = validate_name(name_override.unwrap_or(&export.profile.name))?;
        let base = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| {
                path.parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });
        export.profile.config.resolve_config_paths(&base);
        let config = normalize_profile_config(export.profile.config)?;
        self.create(&name, config)
    }

    fn read<T>(&self, f: impl FnOnce(&ProfileDocument) -> Result<T>) -> Result<T> {
        let _lock = StoreLock::acquire(&self.path)?;
        let document = load_document(&self.path)?;
        f(&document)
    }

    fn mutate<T>(&self, f: impl FnOnce(&mut ProfileDocument) -> Result<T>) -> Result<T> {
        let _lock = StoreLock::acquire(&self.path)?;
        let mut document = load_document(&self.path)?;
        let result = f(&mut document)?;
        save_document(&self.path, &document)?;
        Ok(result)
    }
}

pub fn default_profile_store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_PROFILE_STORE") {
        return PathBuf::from(path);
    }
    crate::store_paths::config_root().join("profiles.json")
}

fn normalize_profile_config(mut config: RenderConfig) -> Result<RenderConfig> {
    config.ui.migrate_legacy_empty_default();
    config.validate()?;
    scrub_secret_references(&mut config);
    Ok(config)
}

fn scrub_secret_references(config: &mut RenderConfig) {
    let key = config.notifications.discord.bot_token_env.as_str();
    let valid = !key.is_empty()
        && key.chars().enumerate().all(|(index, ch)| {
            ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
        });
    if !valid {
        config.notifications.discord.bot_token_env =
            crate::notifications::DiscordNotificationConfig::default().bot_token_env;
    }
}

fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("profile name cannot be empty");
    }
    if name.chars().count() > 64 {
        bail!("profile name must be 64 characters or fewer");
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        bail!("profile name cannot contain control characters or path separators");
    }
    Ok(name.to_owned())
}

fn ensure_unique_name(
    document: &ProfileDocument,
    name: &str,
    except_id: Option<&str>,
) -> Result<()> {
    if document.profiles.iter().any(|profile| {
        Some(profile.id.as_str()) != except_id
            && (profile.name.eq_ignore_ascii_case(name) || profile.id.eq_ignore_ascii_case(name))
    }) {
        bail!("a configuration profile name or identifier {name:?} is already in use");
    }
    Ok(())
}

fn find_profile<'a>(document: &'a ProfileDocument, selector: &str) -> Option<&'a ConfigProfile> {
    document
        .profiles
        .iter()
        .find(|profile| profile.id == selector)
        .or_else(|| {
            document
                .profiles
                .iter()
                .find(|profile| profile.name.eq_ignore_ascii_case(selector.trim()))
        })
}

fn profile_index(document: &ProfileDocument, selector: &str) -> Result<usize> {
    document
        .profiles
        .iter()
        .position(|profile| profile.id == selector)
        .or_else(|| {
            document
                .profiles
                .iter()
                .position(|profile| profile.name.eq_ignore_ascii_case(selector.trim()))
        })
        .with_context(|| format!("configuration profile {selector:?} was not found"))
}

fn unique_profile_id(document: &ProfileDocument) -> String {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    loop {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = format!("profile-{:x}-{nanos:x}-{counter:x}", std::process::id());
        if document.profiles.iter().all(|profile| {
            profile.id != candidate && !profile.name.eq_ignore_ascii_case(&candidate)
        }) {
            return candidate;
        }
    }
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn load_document(path: &Path) -> Result<ProfileDocument> {
    if !path.exists() {
        return Ok(ProfileDocument::default());
    }
    let bytes =
        fs::read(path).with_context(|| format!("reading profile store {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing profile store {}", path.display()))?;
    if let Some(profiles) = value.get("profiles").and_then(serde_json::Value::as_array) {
        for (index, profile) in profiles.iter().enumerate() {
            reject_unknown_config_fields(
                profile.get("config"),
                &format!("profiles[{index}].config"),
            )?;
        }
    }
    let document: ProfileDocument = serde_json::from_value(value)
        .with_context(|| format!("parsing profile store {}", path.display()))?;
    if document.schema_version != PROFILE_SCHEMA_VERSION {
        bail!(
            "unsupported profile store schema version {}; expected {}",
            document.schema_version,
            PROFILE_SCHEMA_VERSION
        );
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for profile in &document.profiles {
        if profile.id.trim().is_empty() || !ids.insert(profile.id.as_str()) {
            bail!("profile store contains an empty or duplicate profile id");
        }
        validate_name(&profile.name)?;
        if !names.insert(profile.name.to_lowercase()) {
            bail!("profile store contains ambiguous duplicate names");
        }
        profile
            .config
            .validate()
            .with_context(|| format!("stored profile {:?} is invalid", profile.name))?;
    }
    if document
        .active_profile_id
        .as_ref()
        .is_some_and(|active| !ids.contains(active.as_str()))
    {
        bail!("profile store points to a missing active profile");
    }
    Ok(document)
}

fn reject_unknown_config_fields(value: Option<&serde_json::Value>, location: &str) -> Result<()> {
    let object = value
        .and_then(serde_json::Value::as_object)
        .with_context(|| format!("{location} must be a renderer configuration object"))?;
    let known = serde_json::to_value(RenderConfig::default())?;
    let known = known
        .as_object()
        .expect("RenderConfig serializes as an object")
        .keys()
        .collect::<std::collections::HashSet<_>>();
    if let Some(field) = object.keys().find(|key| !known.contains(key)) {
        bail!(
            "unknown field {field:?} in {location}; refusing to silently discard profile settings"
        );
    }
    Ok(())
}

fn save_document(path: &Path, document: &ProfileDocument) -> Result<()> {
    write_json_atomic(path, document)
        .with_context(|| format!("saving profile store {}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating profile directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("atomically replacing {}", path.display()))?;
    Ok(())
}

struct StoreLock {
    path: PathBuf,
}

impl StoreLock {
    fn acquire(store: &Path) -> Result<Self> {
        let parent = store
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let mut lock_name = store.as_os_str().to_os_string();
        lock_name.push(".lock");
        let path = PathBuf::from(lock_name);
        let deadline = Instant::now() + LOCK_WAIT_LIMIT;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > LOCK_STALE_AFTER);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "timed out waiting for profile store lock {}",
                            path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("locking {}", path.display()))
                }
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn durable_config(config: &RenderConfig) -> RenderConfig {
        serde_json::from_value(serde_json::to_value(config).unwrap()).unwrap()
    }

    fn assert_durable_config_eq(left: &RenderConfig, right: &RenderConfig) {
        assert_eq!(durable_config(left), durable_config(right));
    }

    fn manager(temp: &TempDir) -> ConfigProfileManager {
        ConfigProfileManager::open_at(temp.path().join("profiles.json")).unwrap()
    }

    #[test]
    fn profile_crud_selection_reset_and_snapshot_round_trip() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        let mut config = RenderConfig::default();
        config.width = 1280;
        config.execution_mode = crate::sono_gcc::WatchExecutionMode::SonoGcc;
        let created = manager.create("  Fast Preview  ", config.clone()).unwrap();
        assert!(created.id.starts_with("profile-"));
        assert_eq!(created.name, "Fast Preview");
        assert_durable_config_eq(&manager.resolve(&created.id).unwrap(), &config);
        assert!(serde_json::to_value(&created.config)
            .unwrap()
            .pointer("/ui/profile_directory")
            .is_none());
        assert_eq!(manager.select("fast preview").unwrap().id, created.id);
        assert_eq!(manager.active().unwrap().unwrap().id, created.id);

        let duplicate = manager.duplicate(&created.id, "Fast Preview Copy").unwrap();
        assert_ne!(duplicate.id, created.id);
        assert_durable_config_eq(&duplicate.config, &config);
        let renamed = manager.rename(&duplicate.id, "Draft Preview").unwrap();
        assert_eq!(renamed.name, "Draft Preview");
        let edited = manager
            .update(&created.id, RenderConfig::default())
            .unwrap();
        assert_eq!(edited.config, RenderConfig::default());
        assert_eq!(
            manager.reset(&duplicate.id).unwrap().config,
            RenderConfig::default()
        );
        assert!(manager.delete(&created.id).is_ok());
        assert!(manager.active().unwrap().is_none());
    }

    #[test]
    fn names_are_unambiguous_and_validated() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        manager.create("YouTube", RenderConfig::default()).unwrap();
        assert!(manager.create("youtube", RenderConfig::default()).is_err());
        for name in ["", "  ", "bad/name", "bad\\name", "bad\nname"] {
            assert!(manager.create(name, RenderConfig::default()).is_err());
        }
        assert!(manager.get("missing").is_err());
    }

    #[test]
    fn import_export_round_trip_is_versioned_and_never_exports_credentials() {
        let temp = TempDir::new().unwrap();
        let manager = manager(&temp);
        let mut config = RenderConfig::default();
        let engine_path = temp.path().join("engine.zip");
        fs::write(&engine_path, b"fixture").unwrap();
        config.engine = fs::canonicalize(&engine_path).unwrap();
        config.notifications.discord.enabled = true;
        config.notifications.discord.bot_token_env = "raw.bot.token.secret".into();
        let profile = manager.create("Discord Preview", config).unwrap();
        assert_eq!(
            profile.config.notifications.discord.bot_token_env,
            crate::notifications::DiscordNotificationConfig::default().bot_token_env
        );
        let export_path = temp.path().join("portable.json");
        manager.export(&profile.id, &export_path).unwrap();
        let exported = fs::read_to_string(&export_path).unwrap();
        assert!(!exported.contains("raw.bot.token.secret"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&exported).unwrap()["profile"]["config"]
                ["engine"],
            "engine.zip"
        );
        let imported = manager
            .import(&export_path, Some("Imported Preview"))
            .unwrap();
        assert_eq!(imported.name, "Imported Preview");
        assert_durable_config_eq(&imported.config, &profile.config);
        let document: ConfigProfileExport = serde_json::from_str(&exported).unwrap();
        assert_eq!(document.schema_version, PROFILE_SCHEMA_VERSION);

        let mut unknown: serde_json::Value = serde_json::from_str(&exported).unwrap();
        unknown["unexpected"] = serde_json::json!(true);
        fs::write(temp.path().join("unknown.json"), unknown.to_string()).unwrap();
        assert!(format!(
            "{:#}",
            manager
                .import(temp.path().join("unknown.json"), None)
                .unwrap_err()
        )
        .contains("unknown field"));

        let mut unknown_config: serde_json::Value = serde_json::from_str(&exported).unwrap();
        unknown_config["profile"]["config"]["future_render_mode"] = serde_json::json!("fast");
        fs::write(
            temp.path().join("unknown-config.json"),
            unknown_config.to_string(),
        )
        .unwrap();
        assert!(format!(
            "{:#}",
            manager
                .import(temp.path().join("unknown-config.json"), None)
                .unwrap_err()
        )
        .contains("future_render_mode"));

        let mut future = document;
        future.schema_version += 1;
        fs::write(
            temp.path().join("future.json"),
            serde_json::to_vec(&future).unwrap(),
        )
        .unwrap();
        assert!(manager
            .import(temp.path().join("future.json"), None)
            .is_err());
        fs::write(
            temp.path().join("missing.json"),
            r#"{"format":"sono-renderer-config-profile","schema_version":1}"#,
        )
        .unwrap();
        assert!(manager
            .import(temp.path().join("missing.json"), None)
            .is_err());
    }

    #[test]
    fn active_selection_survives_restart_and_deleted_selection_clears() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("profiles.json");
        let manager = ConfigProfileManager::open_at(&path).unwrap();
        let profile = manager
            .create("Persistent", RenderConfig::default())
            .unwrap();
        manager.select(&profile.id).unwrap();
        let reopened = ConfigProfileManager::open_at(&path).unwrap();
        assert_eq!(reopened.active().unwrap().unwrap().id, profile.id);
        reopened.delete(&profile.id).unwrap();
        assert!(manager.active().unwrap().is_none());
    }
}
