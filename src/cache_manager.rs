//! Shared inspection and lifecycle controls for regenerable renderer caches.
//!
//! Durable settings, profiles, queued jobs, renders, and retained diagnostics
//! are deliberately outside this manager. Persistent cache operations are
//! delegated to the producer so the CLI and GUI share the same safety rules.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
pub struct CacheInventory {
    pub scanned_unix_ms: u128,
    pub total_disk_bytes: u64,
    /// Aggregate regenerable disk caches. `disk_entries` remains the
    /// individually inspectable Sono-GCC store; smaller producer-owned
    /// formats are summarized here instead of pretending they share its
    /// entry schema.
    pub regenerable_disk: Vec<DiskCacheInfo>,
    pub disk_entries: Vec<CacheEntry>,
    pub memory: Vec<MemoryCacheInfo>,
    pub durable_data_note: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskCacheInfo {
    pub category: &'static str,
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheEntry {
    /// Store-qualified ID accepted by `cache inspect/remove`.
    pub id: String,
    pub category: &'static str,
    /// A graph digest is available; a user-facing engine/project name is not
    /// retained by the compiler cache today.
    pub associated_data: String,
    pub size_bytes: u64,
    pub created_unix_ms: Option<u128>,
    pub last_used_unix_ms: Option<u128>,
    pub schema: Option<u32>,
    pub target: Option<String>,
    pub status: CacheEntryStatus,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CacheEntryStatus {
    Unverified,
    Valid,
    Incompatible,
    Corrupt,
    Unreadable,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryCacheInfo {
    pub category: &'static str,
    pub entries: usize,
    pub retained_bytes: Option<u64>,
    pub limit_bytes: Option<u64>,
    pub notes: &'static str,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CacheOperationReport {
    pub removed_entries: usize,
    pub removed_bytes: u64,
    pub skipped_active: usize,
    pub cleared_memory_entries: usize,
    pub cleared_tempo_analysis_entries: usize,
    pub cleared_transcription_entries: usize,
}

pub fn inventory() -> Result<CacheInventory> {
    let disk_entries = crate::sono_gcc::inspect_persistent_cache()?;
    let gcc_disk_bytes = disk_entries
        .iter()
        .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
    let tempo = crate::tempo_analysis::cache_info();
    let transcription = crate::subtitles::cache_info()?;
    let total_disk_bytes = gcc_disk_bytes
        .saturating_add(tempo.bytes)
        .saturating_add(transcription.bytes);
    let (sfx_entries, sfx_bytes, sfx_limit) = crate::audio::decoded_sfx_cache_info();
    let (atlas_entries, atlas_bytes, atlas_limit) = crate::gpu_render::atlas_cache_info();
    let (gcc_entries, gcc_bytes) = crate::sono_gcc::process_cache_info();
    Ok(CacheInventory {
        scanned_unix_ms: now_unix_ms(),
        total_disk_bytes,
        regenerable_disk: vec![
            DiskCacheInfo {
                category: "Sono-GCC compiled programs",
                entries: disk_entries.len(),
                bytes: gcc_disk_bytes,
            },
            DiskCacheInfo {
                category: "Chart tempo analysis",
                entries: tempo.entries,
                bytes: tempo.bytes,
            },
            DiskCacheInfo {
                category: "Speech transcription results",
                entries: transcription.entries,
                bytes: transcription.bytes,
            },
        ],
        disk_entries,
        memory: vec![
            MemoryCacheInfo {
                category: "Decoded SFX PCM",
                entries: sfx_entries,
                retained_bytes: Some(sfx_bytes),
                limit_bytes: Some(sfx_limit),
                notes: "Content-keyed LRU; active audio clips remain alive until their render releases them.",
            },
            MemoryCacheInfo {
                category: "WGPU source atlases",
                entries: atlas_entries,
                retained_bytes: Some(atlas_bytes),
                limit_bytes: Some(atlas_limit),
                notes: "Device-local texture cache; clearing drops cache references, not in-flight frame references.",
            },
            MemoryCacheInfo {
                category: "Sono-GCC loaded programs",
                entries: gcc_entries,
                retained_bytes: gcc_bytes,
                limit_bytes: None,
                notes: "At most four process-local programs; active render references remain alive.",
            },
        ],
        durable_data_note: "This manager only removes generated cache data. It never scans or removes profiles, settings, queued-job history, source assets, rendered videos, Cargo target data, or retained artifacts/ diagnostics.",
    })
}

pub fn inspect(id: &str) -> Result<CacheEntry> {
    inventory()?
        .disk_entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow::anyhow!("cache entry {id:?} was not found"))
}

/// Hash and structurally validate every persistent entry. This is explicit
/// work and is never run on renderer startup or in the render hot path.
pub fn validate_all() -> Result<Vec<CacheEntry>> {
    crate::sono_gcc::validate_persistent_cache()
}

pub fn validate(id: &str) -> Result<CacheEntry> {
    let (store, key) = parse_entry_id(id)?;
    crate::sono_gcc::validate_persistent_cache_entry(store, key)?
        .ok_or_else(|| anyhow::anyhow!("cache entry {id:?} was not found"))
}

pub fn remove(id: &str) -> Result<CacheOperationReport> {
    let (store, key) = parse_entry_id(id)?;
    if crate::sono_gcc::evict_process_cache_entry(key)? {
        bail!("cache entry {id:?} is in active use and was not removed");
    }
    let root = crate::sono_gcc::persistent_cache_roots()
        .into_iter()
        .find_map(|(candidate, root)| (candidate == store).then_some(root));
    let Some(root) = root else {
        bail!("persistent Sono-GCC cache store {store:?} is unavailable on this platform");
    };
    let (removed, bytes, active) = remove_gcc_entry_at(&root, key)?;
    if active {
        bail!("cache entry {id:?} is in active use and was not removed");
    }
    if !removed {
        bail!("cache entry {id:?} was not found");
    }
    Ok(CacheOperationReport {
        removed_entries: 1,
        removed_bytes: bytes,
        ..CacheOperationReport::default()
    })
}

pub fn rebuild(id: &str) -> Result<crate::sono_gcc::SonoGccPrecompileReport> {
    let (store, key) = parse_entry_id(id)?;
    let nodes = crate::sono_gcc::load_cached_watch_graph(store, key)?;
    remove(id)?;
    crate::sono_gcc::precompile(&nodes)
}

/// Remove all inactive Sono-GCC entries. Active entries are reported and left
/// untouched. `include_memory` also evicts safe process-cache references.
pub fn clear(include_memory: bool) -> Result<CacheOperationReport> {
    let entries = inventory()?.disk_entries;
    let mut report = CacheOperationReport::default();
    for entry in entries {
        match remove(&entry.id) {
            Ok(removed) => {
                report.removed_entries += removed.removed_entries;
                report.removed_bytes = report.removed_bytes.saturating_add(removed.removed_bytes);
            }
            Err(error) if error.to_string().contains("active use") => {
                report.skipped_active += 1;
            }
            Err(error) => return Err(error),
        }
    }
    let analysis = crate::tempo_analysis::clear_cache()?;
    report.cleared_tempo_analysis_entries = analysis.entries;
    report.removed_entries = report.removed_entries.saturating_add(analysis.entries);
    report.removed_bytes = report.removed_bytes.saturating_add(analysis.bytes);
    let transcription = crate::subtitles::clear_cache()?;
    report.cleared_transcription_entries = transcription.entries;
    report.removed_entries = report.removed_entries.saturating_add(transcription.entries);
    report.removed_bytes = report.removed_bytes.saturating_add(transcription.bytes);
    if include_memory {
        report.cleared_memory_entries = crate::sono_gcc::clear_idle_process_cache()
            + crate::audio::clear_decoded_sfx_cache()
            + crate::gpu_render::clear_atlas_cache();
    }
    Ok(report)
}

fn remove_gcc_entry_at(root: &Path, key: &str) -> Result<(bool, u64, bool)> {
    validate_key(key)?;
    let manager = acquire_manager_lock(root, true)?;
    let Some(entry_lock) = try_entry_lock_while_manager_locked(root, key)? else {
        manager.unlock().context("releasing cache manager lock")?;
        return Ok((false, 0, true));
    };
    let directory = root.join(key.to_ascii_lowercase());
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            entry_lock.unlock()?;
            manager.unlock().context("releasing cache manager lock")?;
            bail!(
                "refusing to remove non-directory cache entry {}",
                directory.display()
            );
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let lock_path = entry_lock.path.clone();
            entry_lock.unlock()?;
            let _ = std::fs::remove_file(lock_path);
            manager.unlock().context("releasing cache manager lock")?;
            return Ok((false, 0, false));
        }
        Err(error) => {
            entry_lock.unlock()?;
            manager.unlock().context("releasing cache manager lock")?;
            return Err(error)
                .with_context(|| format!("reading cache entry {}", directory.display()));
        }
    };
    let bytes = directory_size(&directory)?;
    std::fs::remove_dir_all(&directory)
        .with_context(|| format!("removing cache entry {}", directory.display()))?;
    let lock_path = entry_lock.path.clone();
    entry_lock.unlock()?;
    // The manager lock prevents a new reader/compiler from opening this lock
    // file until after deletion and lock-file cleanup both finish.
    match std::fs::remove_file(&lock_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("removing cache lock {}", lock_path.display()))
        }
    }
    let _ = std::fs::remove_dir(lock_path.parent().unwrap_or(root));
    manager.unlock().context("releasing cache manager lock")?;
    Ok((true, bytes, false))
}

pub(crate) fn directory_size(directory: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(path) = pending.pop() {
        for item in std::fs::read_dir(&path)
            .with_context(|| format!("scanning cache directory {}", path.display()))?
        {
            let item = item?;
            let kind = item.file_type()?;
            if kind.is_dir() && !kind.is_symlink() {
                pending.push(item.path());
            } else if kind.is_file() {
                total = total.saturating_add(item.metadata()?.len());
            }
        }
    }
    Ok(total)
}

/// Explicit age/size retention. No automatic pruning runs during normal use.
/// Entries with recent use are retained first; unknown timestamps sort as old.
pub fn prune_to(maximum_bytes: u64) -> Result<CacheOperationReport> {
    let snapshot = inventory()?;
    let mut entries = snapshot.disk_entries;
    sort_for_pruning(&mut entries);
    let gcc_total = entries
        .iter()
        .fold(0_u64, |sum, entry| sum.saturating_add(entry.size_bytes));
    let mut total = snapshot.total_disk_bytes;
    let mut report = CacheOperationReport::default();
    for entry in entries {
        if total <= maximum_bytes {
            break;
        }
        match remove(&entry.id) {
            Ok(removed) => {
                total = total.saturating_sub(removed.removed_bytes);
                report.removed_entries += removed.removed_entries;
                report.removed_bytes = report.removed_bytes.saturating_add(removed.removed_bytes);
            }
            Err(error) if error.to_string().contains("active use") => {
                report.skipped_active += 1;
            }
            Err(error) => return Err(error),
        }
    }
    let remaining_gcc = gcc_total.saturating_sub(report.removed_bytes);
    if total > maximum_bytes {
        let tempo =
            crate::tempo_analysis::prune_cache_to(maximum_bytes.saturating_sub(remaining_gcc))?;
        report.removed_entries += tempo.entries;
        report.removed_bytes = report.removed_bytes.saturating_add(tempo.bytes);
    }
    Ok(report)
}

fn sort_for_pruning(entries: &mut [CacheEntry]) {
    entries.sort_by_key(|entry| {
        entry
            .last_used_unix_ms
            .or(entry.created_unix_ms)
            .unwrap_or_default()
    });
}

fn parse_entry_id(id: &str) -> Result<(&str, &str)> {
    let (store, key) = id
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("cache entry IDs must be STORE:SHA1"))?;
    if !matches!(store, "appdata" | "temp")
        || key.len() != 40
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid cache entry ID {id:?}");
    }
    Ok((store, key))
}

#[derive(Clone, Copy)]
pub(crate) enum EntryLockMode {
    Shared,
    Exclusive,
}

/// OS-released lock for one content identity. Lock files live outside the
/// removable entry directory so cache deletion cannot race a new opener.
pub(crate) struct EntryLock {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl EntryLock {
    pub(crate) fn unlock(self) -> Result<()> {
        self.file.unlock().context("releasing cache entry lock")
    }
}

fn lock_base(root: &Path) -> PathBuf {
    root.parent()
        .unwrap_or(root)
        .join(".locks")
        .join("sono-gcc-v2")
}

fn open_lock(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating cache lock directory {}", parent.display()))?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("opening cache lock {}", path.display()))
}

fn acquire_manager_lock(root: &Path, exclusive: bool) -> Result<File> {
    let path = root
        .parent()
        .unwrap_or(root)
        .join(".locks")
        .join("manager.lock");
    let file = open_lock(&path)?;
    if exclusive {
        file.lock().context("locking cache manager")?;
    } else {
        file.lock_shared().context("reading cache manager")?;
    }
    Ok(file)
}

pub(crate) fn acquire_entry_lock(root: &Path, key: &str, mode: EntryLockMode) -> Result<EntryLock> {
    validate_key(key)?;
    // Serialize lock-file creation/removal only. The per-entry lock remains
    // held independently, so unrelated graph compilations can proceed.
    let manager = acquire_manager_lock(root, false)?;
    let path = lock_base(root).join(format!("{key}.lock"));
    let file = open_lock(&path)?;
    match mode {
        EntryLockMode::Shared => file.lock_shared().context("locking cache entry for read")?,
        EntryLockMode::Exclusive => file.lock().context("locking cache entry for write")?,
    }
    manager.unlock().context("releasing cache manager lock")?;
    Ok(EntryLock { file, path })
}

pub(crate) fn entry_lock_if_present(
    root: &Path,
    key: &str,
    mode: EntryLockMode,
) -> Result<Option<EntryLock>> {
    validate_key(key)?;
    let entry = root.join(key);
    let Ok(metadata) = std::fs::symlink_metadata(&entry) else {
        return Ok(None);
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    acquire_entry_lock(root, key, mode).map(Some)
}

/// Inspect a cache entry without creating lock state. CLI/GUI inventory and
/// validation are read-only operations and must remain usable when a cache
/// store is readable but not writable. If a writer has already established
/// the stable lock files, hold a shared lock for the snapshot; otherwise no
/// new-format writer can be using this entry yet.
pub(crate) fn try_entry_inspection_lock(root: &Path, key: &str) -> Result<Option<EntryLock>> {
    validate_key(key)?;
    let manager_path = root
        .parent()
        .unwrap_or(root)
        .join(".locks")
        .join("manager.lock");
    let Some(manager) = open_existing_lock(&manager_path)? else {
        return Ok(None);
    };
    manager
        .lock_shared()
        .context("reading cache manager lock for inspection")?;

    let path = lock_base(root).join(format!("{key}.lock"));
    let entry = match open_existing_lock(&path)? {
        Some(file) => {
            file.lock_shared()
                .context("locking cache entry for inspection")?;
            Some(EntryLock { file, path })
        }
        None => None,
    };
    manager.unlock().context("releasing cache manager lock")?;
    Ok(entry)
}

fn open_existing_lock(path: &Path) -> Result<Option<File>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        // Lack of write permission must not prevent a read-only inventory.
        // We cannot promise a stable snapshot in that case, but cache payload
        // integrity is still checked and the runtime revalidates before use.
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("reading lock {}", path.display()))
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("refusing unsafe cache lock path {}", path.display());
    }
    match OpenOptions::new().read(true).open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(None),
        Err(error) => Err(error).with_context(|| format!("opening lock {}", path.display())),
    }
}

fn try_entry_lock_while_manager_locked(root: &Path, key: &str) -> Result<Option<EntryLock>> {
    let path = lock_base(root).join(format!("{key}.lock"));
    let file = open_lock(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(EntryLock { file, path })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => {
            Err(error).context("checking whether cache entry is active")
        }
    }
}

pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.len() != 40
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid Sono-GCC cache key {key:?}");
    }
    Ok(())
}

pub(crate) fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_entry_ids_are_store_qualified_and_path_safe() {
        assert_eq!(
            parse_entry_id("appdata:0123456789abcdef0123456789abcdef01234567").unwrap(),
            ("appdata", "0123456789abcdef0123456789abcdef01234567")
        );
        for invalid in [
            "../outside",
            "appdata:../outside",
            "unknown:0123456789abcdef0123456789abcdef01234567",
            "temp:short",
        ] {
            assert!(parse_entry_id(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn active_entry_lease_prevents_removal_and_user_data_stays_outside_scope() {
        let temp = tempfile::tempdir().unwrap();
        let cache_base = temp.path().join("Sono-Renderer").join("cache");
        let root = cache_base.join("sono-gcc-v2");
        let key = "0123456789abcdef0123456789abcdef01234567";
        let entry = root.join(key);
        std::fs::create_dir_all(&entry).unwrap();
        std::fs::write(entry.join("manifest.json"), b"generated cache").unwrap();
        let durable = temp.path().join("profiles.json");
        std::fs::write(&durable, b"durable profile data").unwrap();

        let lease = acquire_entry_lock(&root, key, EntryLockMode::Shared).unwrap();
        let (removed, _, active) = remove_gcc_entry_at(&root, key).unwrap();
        assert!(!removed);
        assert!(active);
        assert!(entry.exists());

        drop(lease);
        let (removed, bytes, active) = remove_gcc_entry_at(&root, key).unwrap();
        assert!(removed);
        assert!(!active);
        assert_eq!(bytes, b"generated cache".len() as u64);
        assert!(!entry.exists());
        assert_eq!(std::fs::read(durable).unwrap(), b"durable profile data");
    }

    #[test]
    fn per_entry_locks_serialize_same_key_without_blocking_other_keys() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache").join("sono-gcc-v2");
        let first_key = "0123456789abcdef0123456789abcdef01234567";
        let second_key = "1123456789abcdef0123456789abcdef01234567";
        let first = acquire_entry_lock(&root, first_key, EntryLockMode::Exclusive).unwrap();
        let unrelated = acquire_entry_lock(&root, second_key, EntryLockMode::Exclusive).unwrap();
        drop(unrelated);

        let worker_root = root.clone();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let lock =
                acquire_entry_lock(&worker_root, first_key, EntryLockMode::Exclusive).unwrap();
            acquired_tx.send(()).unwrap();
            lock
        });
        started_rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(matches!(
            acquired_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(first);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap().unlock().unwrap();
    }

    #[test]
    fn inspection_does_not_create_locks_for_a_read_only_store() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache").join("sono-gcc-v2");
        let key = "a".repeat(40);
        std::fs::create_dir_all(root.join(&key)).unwrap();

        assert!(try_entry_inspection_lock(&root, &key).unwrap().is_none());
        assert!(!root.parent().unwrap().join(".locks").exists());
    }

    #[test]
    fn retention_orders_oldest_last_use_first_and_falls_back_to_creation() {
        let make_entry = |id: &str, created, last_used| CacheEntry {
            id: id.into(),
            category: "test",
            associated_data: id.into(),
            size_bytes: 1,
            created_unix_ms: created,
            last_used_unix_ms: last_used,
            schema: None,
            target: None,
            status: CacheEntryStatus::Unverified,
            diagnostic: None,
        };
        let mut entries = vec![
            make_entry("used-later", Some(1), Some(20)),
            make_entry("creation-fallback", Some(5), None),
            make_entry("used-earliest", Some(2), Some(10)),
        ];

        sort_for_pruning(&mut entries);

        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["creation-fallback", "used-earliest", "used-later"]
        );
    }
}
