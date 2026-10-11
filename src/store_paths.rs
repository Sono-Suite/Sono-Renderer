//! Canonical per-user storage roots.
//!
//! Windows portable archives opt into a `UserData` directory by including a
//! `portable.mode` marker beside the executable. Installed builds use the
//! Windows user profile. Other platforms retain their XDG locations.

use std::path::{Path, PathBuf};

const PORTABLE_MARKER: &str = "portable.mode";
const PORTABLE_MARKER_CONTENT: &str = "sono-renderer-portable-v1";

/// Root for user-owned application state and generated data.
pub(crate) fn user_data_root() -> PathBuf {
    #[cfg(windows)]
    if let Some(root) = portable_user_data_root(current_executable_directory().as_deref()) {
        return root;
    }

    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(root).join("Sono-Renderer");
        }
        if let Some(root) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(root)
                .join("AppData")
                .join("Local")
                .join("Sono-Renderer");
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(root) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(root).join("sono-renderer");
        }
        if let Some(root) = std::env::var_os("HOME") {
            return PathBuf::from(root)
                .join(".local")
                .join("share")
                .join("sono-renderer");
        }
    }

    std::env::temp_dir().join("Sono-Renderer")
}

pub(crate) fn config_root() -> PathBuf {
    #[cfg(windows)]
    {
        return user_data_root();
    }
    #[cfg(not(windows))]
    {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root).join("sono-renderer");
        }
        if let Some(root) = std::env::var_os("HOME") {
            return PathBuf::from(root).join(".config").join("sono-renderer");
        }
        std::env::temp_dir().join("sono-renderer")
    }
}

/// Resolve immutable application resources beside a packaged executable, or
/// from the repository during Cargo builds/tests. This does not depend on the
/// process working directory for installed applications.
pub(crate) fn application_resource_root() -> PathBuf {
    let executable_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    if let Some(mut directory) = executable_dir {
        if directory.file_name().and_then(|name| name.to_str()) == Some("deps") {
            directory = directory
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(directory);
        }
        if matches!(
            directory.file_name().and_then(|name| name.to_str()),
            Some("debug" | "release")
        ) {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        }
        return directory;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub(crate) fn state_root() -> PathBuf {
    #[cfg(windows)]
    {
        return user_data_root();
    }
    #[cfg(not(windows))]
    {
        if let Some(root) = std::env::var_os("XDG_STATE_HOME") {
            return PathBuf::from(root).join("sono-renderer");
        }
        if let Some(root) = std::env::var_os("HOME") {
            return PathBuf::from(root)
                .join(".local")
                .join("state")
                .join("sono-renderer");
        }
        std::env::temp_dir().join("sono-renderer")
    }
}

#[cfg_attr(all(windows, test), allow(dead_code))]
pub(crate) fn cache_root() -> PathBuf {
    #[cfg(windows)]
    {
        return user_data_root().join("cache");
    }
    #[cfg(not(windows))]
    {
        if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(root).join("sono-renderer");
        }
        if let Some(root) = std::env::var_os("HOME") {
            return PathBuf::from(root).join(".cache").join("sono-renderer");
        }
        std::env::temp_dir().join("sono-renderer-cache")
    }
}

#[cfg(windows)]
fn current_executable_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

#[cfg(windows)]
fn portable_user_data_root(executable_directory: Option<&Path>) -> Option<PathBuf> {
    let executable_directory = executable_directory?;
    let marker = executable_directory.join(PORTABLE_MARKER);
    let contents = std::fs::read_to_string(marker).ok()?;
    (contents.trim() == PORTABLE_MARKER_CONTENT).then(|| executable_directory.join("UserData"))
}

pub(crate) fn default_job_store_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_JOB_STORE") {
        return PathBuf::from(path);
    }
    state_root().join("jobs.json")
}

pub(crate) fn history_store_path_for_queue(queue_path: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_HISTORY_DIR") {
        return PathBuf::from(path);
    }
    queue_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("history")
}

pub(crate) fn default_history_store_path() -> PathBuf {
    history_store_path_for_queue(&default_job_store_path())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn portable_mode_requires_the_explicit_versioned_marker() {
        let directory = tempfile::tempdir().unwrap();
        let exe_dir = directory.path();
        assert!(portable_user_data_root(Some(exe_dir)).is_none());

        std::fs::write(exe_dir.join(PORTABLE_MARKER), "").unwrap();
        assert!(portable_user_data_root(Some(exe_dir)).is_none());

        std::fs::write(exe_dir.join(PORTABLE_MARKER), PORTABLE_MARKER_CONTENT).unwrap();
        assert_eq!(
            portable_user_data_root(Some(exe_dir)),
            Some(exe_dir.join("UserData"))
        );
    }
}
