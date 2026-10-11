//! FFmpeg executable discovery and managed Windows provisioning.
//!
//! The shared `addons` directory is preferred. Managed provisioning is lazy: it
//! happens only when a caller asks this module to resolve FFmpeg.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

const DISTRIBUTION_URL: &str =
    "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-full_build.zip";
const DISTRIBUTION_SHA256: &str =
    "587B1C37DE29C5003D01CF65DA10001BAC43A58B88E61AF0FC77C61DAFF04761";
const MANAGED_VERSION: &str = "8.1";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FfmpegSource {
    SharedAddons,
    Managed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FfmpegInstallation {
    pub source: FfmpegSource,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// Find a validated app-local installation, otherwise use or provision the
/// per-user managed installation. PATH is deliberately never consulted.
pub fn resolve() -> Result<FfmpegInstallation> {
    resolve_controlled(&crate::export_control::ExportControl::default())
}

/// Resolve or provision FFmpeg while allowing an owning task to cancel a
/// pending download. Ordinary renderer callers continue to use [`resolve`].
pub(crate) fn resolve_controlled(
    control: &crate::export_control::ExportControl,
) -> Result<FfmpegInstallation> {
    control.check()?;
    #[cfg(windows)]
    let app_root = application_root()?;
    #[cfg(windows)]
    {
        let shared = shared_addons_directory(&app_root)?;
        if let Ok(installation) = validate_pair(
            FfmpegSource::SharedAddons,
            shared.join("ffmpeg.exe"),
            shared.join("ffprobe.exe"),
        ) {
            control.check()?;
            return Ok(installation);
        }
    }

    let (ffmpeg_name, ffprobe_name) = binary_names();
    let managed = crate::store_paths::user_data_root()
        .join("dependencies")
        .join("ffmpeg")
        .join(MANAGED_VERSION);
    let mut managed_directories = vec![managed.as_path()];
    #[cfg(windows)]
    let legacy_managed = app_root
        .join("dependencies")
        .join("ffmpeg")
        .join(MANAGED_VERSION);
    #[cfg(windows)]
    managed_directories.push(legacy_managed.as_path());
    for directory in managed_directories {
        if let Ok(installation) = validate_pair(
            FfmpegSource::Managed,
            directory.join(ffmpeg_name),
            directory.join(ffprobe_name),
        ) {
            control.check()?;
            return Ok(installation);
        }
    }

    install_managed_controlled(&managed, control).with_context(|| {
        format!(
            "Sono-Renderer needs FFmpeg 8.1 for MP4 export. The first export downloads the pinned Windows build from Gyan at {DISTRIBUTION_URL} into the current user's application data; check the network connection and retry. The renderer will not use an unvalidated executable from PATH"
        )
    })?;
    validate_pair(
        FfmpegSource::Managed,
        managed.join(ffmpeg_name),
        managed.join(ffprobe_name),
    )
    .context("newly provisioned FFmpeg did not pass validation")
}

/// Inspect already-installed FFmpeg tools without downloading or installing
/// anything. Interactive preflight can use this to explain export readiness;
/// the actual export still calls [`resolve`] and retains lazy provisioning.
pub fn inspect_existing() -> Result<Option<FfmpegInstallation>> {
    #[cfg(windows)]
    let app_root = application_root()?;
    #[cfg(windows)]
    {
        let shared = shared_addons_directory(&app_root)?;
        if let Ok(installation) = validate_pair(
            FfmpegSource::SharedAddons,
            shared.join("ffmpeg.exe"),
            shared.join("ffprobe.exe"),
        ) {
            return Ok(Some(installation));
        }
    }

    let (ffmpeg_name, ffprobe_name) = binary_names();
    let managed = crate::store_paths::user_data_root()
        .join("dependencies")
        .join("ffmpeg")
        .join(MANAGED_VERSION);
    let mut managed_directories = vec![managed.as_path()];
    #[cfg(windows)]
    let legacy_managed = app_root
        .join("dependencies")
        .join("ffmpeg")
        .join(MANAGED_VERSION);
    #[cfg(windows)]
    managed_directories.push(legacy_managed.as_path());
    for directory in managed_directories {
        if let Ok(installation) = validate_pair(
            FfmpegSource::Managed,
            directory.join(ffmpeg_name),
            directory.join(ffprobe_name),
        ) {
            return Ok(Some(installation));
        }
    }
    Ok(None)
}

#[cfg(windows)]
fn shared_addons_directory(app_root: &Path) -> Result<PathBuf> {
    // Keep the historical repository-level developer add-on available to
    // source builds. Installed/portable archives only search their own folder;
    // they must never discover an unrelated sibling of Program Files.
    if app_root == Path::new(env!("CARGO_MANIFEST_DIR")) {
        return app_root
            .parent()
            .map(|parent| parent.join("addons"))
            .ok_or_else(|| anyhow!("source application root has no parent directory"));
    }
    Ok(app_root.join("addons"))
}

#[cfg(windows)]
fn binary_names() -> (&'static str, &'static str) {
    ("ffmpeg.exe", "ffprobe.exe")
}

#[cfg(not(windows))]
fn binary_names() -> (&'static str, &'static str) {
    ("ffmpeg", "ffprobe")
}

/// Resolve the application directory independently of the process working
/// directory. Cargo debug/release binaries use the repository as their root.
#[cfg(windows)]
fn application_root() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("locating Sono-Renderer executable")?;
    let mut executable_dir = executable
        .parent()
        .ok_or_else(|| anyhow!("executable path has no parent directory"))?;
    // Cargo integration/unit test executables live in target/{debug,release}/deps.
    // Resolve their sibling profile directory before applying the normal
    // development-build rule below.
    if executable_dir.file_name().and_then(|s| s.to_str()) == Some("deps") {
        executable_dir = executable_dir
            .parent()
            .ok_or_else(|| anyhow!("test executable directory has no profile parent"))?;
    }
    let profile = executable_dir.file_name().and_then(|s| s.to_str());
    if matches!(profile, Some("debug" | "release")) {
        return Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    }
    Ok(executable_dir.to_path_buf())
}

fn validate_pair(
    source: FfmpegSource,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<FfmpegInstallation> {
    validate_binary(&ffmpeg, "ffmpeg")?;
    validate_binary(&ffprobe, "ffprobe")?;
    let ffmpeg_version = version_token(&run_version(&ffmpeg)?, "ffmpeg")?;
    let ffprobe_version = version_token(&run_version(&ffprobe)?, "ffprobe")?;
    if ffmpeg_version != ffprobe_version {
        bail!("FFmpeg and FFprobe versions do not match ({ffmpeg_version} vs {ffprobe_version})");
    }
    Ok(FfmpegInstallation {
        source,
        ffmpeg,
        ffprobe,
    })
}

fn validate_binary(path: &Path, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("accessing {} at {}", label, path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("{} is not a non-empty file: {}", label, path.display());
    }
    Ok(())
}

fn run_version(path: &Path) -> Result<String> {
    let output = crate::export_control::hide_child_window(&mut Command::new(path))
        .arg("-version")
        .output()
        .with_context(|| format!("launching {}", path.display()))?;
    // Windows PowerShell can report an unusual pipeline status for ffprobe.
    // A valid version banner is authoritative here; don't reject solely by code.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !text
        .lines()
        .any(|line| line.starts_with("ffmpeg version ") || line.starts_with("ffprobe version "))
    {
        bail!(
            "{} did not report a valid version banner (process status {:?})",
            path.display(),
            output.status.code()
        );
    }
    Ok(text)
}

fn version_token(output: &str, program: &str) -> Result<String> {
    let prefix = format!("{program} version ");
    output
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|version| version.split_whitespace().next())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{program} version output could not be parsed"))
}

#[cfg(windows)]
fn install_managed_controlled(
    destination: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    let managed_root = destination
        .parent()
        .ok_or_else(|| anyhow!("managed FFmpeg path has no parent"))?;
    fs::create_dir_all(managed_root).context("creating managed FFmpeg directory")?;
    let lock_path = managed_root.join(format!(".{MANAGED_VERSION}.install.lock"));
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("opening FFmpeg installer lock at {}", lock_path.display()))?;
    let lock_deadline = Instant::now() + Duration::from_secs(10 * 60);
    loop {
        control.check()?;
        match lock_file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::Error(error)) => {
                return Err(error).context("acquiring managed FFmpeg installer lock");
            }
            Err(TryLockError::WouldBlock) => {
                if Instant::now() >= lock_deadline {
                    bail!(
                        "timed out waiting for managed FFmpeg installation lock at {}",
                        lock_path.display()
                    );
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    }
    // Keep the OS-released lock held until the install and validation finish.
    let _lock_file: File = lock_file;
    let (ffmpeg_name, ffprobe_name) = binary_names();
    if validate_pair(
        FfmpegSource::Managed,
        destination.join(ffmpeg_name),
        destination.join(ffprobe_name),
    )
    .is_ok()
    {
        control.check()?;
        return Ok(());
    }

    let archive_path = managed_root.join(format!("ffmpeg-{MANAGED_VERSION}.zip"));
    if let Err(error) = download_archive_controlled(&archive_path, control) {
        let _ = fs::remove_file(&archive_path);
        return Err(error);
    }
    if let Err(error) = verify_archive_controlled(&archive_path, control) {
        let _ = fs::remove_file(&archive_path);
        return Err(error);
    }

    let staging = managed_root.join(format!("{MANAGED_VERSION}.installing"));
    if staging.exists() {
        fs::remove_dir_all(&staging).context("removing incomplete FFmpeg staging directory")?;
    }
    fs::create_dir_all(&staging).context("creating FFmpeg staging directory")?;
    if let Err(error) = extract_executables_controlled(&archive_path, &staging, control) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = control.check() {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let (ffmpeg_name, ffprobe_name) = binary_names();
    if let Err(error) = validate_pair(
        FfmpegSource::Managed,
        staging.join(ffmpeg_name),
        staging.join(ffprobe_name),
    )
    .context("validating staged FFmpeg executables")
    {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if let Err(error) = control.check() {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    if destination.exists() {
        fs::remove_dir_all(destination).context("replacing invalid managed FFmpeg installation")?;
    }
    fs::rename(&staging, destination).context("activating managed FFmpeg installation")?;
    let _ = fs::remove_file(&archive_path);
    Ok(())
}

#[cfg(windows)]
fn download_archive_controlled(
    destination: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    let destination = destination.canonicalize().or_else(|_| {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        Ok::<_, std::io::Error>(parent.join(destination.file_name().unwrap_or_default()))
    })?;
    let script_path = destination.to_string_lossy().replace('\'', "''");
    let command = format!(
        "$ErrorActionPreference='Stop'; [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -Uri '{DISTRIBUTION_URL}' -OutFile '{script_path}'"
    );
    let output =
        control
            .output(
                crate::export_control::hide_child_window(&mut Command::new("powershell.exe"))
                    .args(["-NoProfile", "-NonInteractive", "-Command", &command]),
            )
            .context("starting the managed FFmpeg download")?;
    ensure_success(output, "downloading managed FFmpeg archive").map(|_| ())
}

#[cfg(not(windows))]
fn download_archive_controlled(
    _destination: &Path,
    _control: &crate::export_control::ExportControl,
) -> Result<()> {
    bail!("automatic managed FFmpeg provisioning is not yet available on this platform")
}

#[cfg(not(windows))]
fn install_managed_controlled(
    _destination: &Path,
    _control: &crate::export_control::ExportControl,
) -> Result<()> {
    bail!("automatic managed FFmpeg provisioning is not yet available on this platform")
}

fn verify_archive_controlled(
    path: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    #[cfg(windows)]
    {
        let escaped = path.to_string_lossy().replace('\'', "''");
        let command = format!(
            "$ErrorActionPreference='Stop'; (Get-FileHash -Algorithm SHA256 -LiteralPath '{escaped}').Hash"
        );
        let output = control
            .output(
                crate::export_control::hide_child_window(&mut Command::new("powershell.exe"))
                    .args(["-NoProfile", "-NonInteractive", "-Command", &command]),
            )
            .context("calculating managed FFmpeg archive SHA-256")?;
        let result = ensure_success(output, "calculating managed FFmpeg archive SHA-256")?;
        let actual = String::from_utf8_lossy(&result.stdout)
            .trim()
            .to_ascii_uppercase();
        if actual != DISTRIBUTION_SHA256 {
            let _ = fs::remove_file(path);
            bail!("managed FFmpeg archive SHA-256 mismatch (got {actual})");
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    bail!("managed FFmpeg archive verification is currently supported on Windows only")
}

fn extract_executables_controlled(
    archive_path: &Path,
    destination: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    let file = fs::File::open(archive_path).context("opening managed FFmpeg archive")?;
    let mut archive = zip::ZipArchive::new(file).context("reading managed FFmpeg ZIP archive")?;
    for required in ["ffmpeg.exe", "ffprobe.exe"] {
        let mut found = false;
        for index in 0..archive.len() {
            control.check()?;
            let mut entry = archive
                .by_index(index)
                .context("reading FFmpeg ZIP entry")?;
            let name = entry.name().replace('\\', "/");
            if name.ends_with(&format!("/bin/{required}")) || name == format!("bin/{required}") {
                let output_path = destination.join(required);
                let mut output = fs::File::create(&output_path)
                    .with_context(|| format!("creating {}", output_path.display()))?;
                let copied = std::io::copy(&mut entry, &mut output)
                    .with_context(|| format!("extracting {required}"))?;
                if copied == 0 {
                    bail!("managed archive contained an empty {required}");
                }
                output
                    .flush()
                    .context("flushing extracted FFmpeg executable")?;
                found = true;
                break;
            }
        }
        if !found {
            bail!("managed archive does not contain {required}");
        }
    }
    Ok(())
}

fn ensure_success(output: Output, action: &str) -> Result<Output> {
    if !output.status.success() {
        bail!(
            "{action} failed with status {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn managed_installer_lock_excludes_other_handles_and_releases_on_drop() {
        let directory = tempfile::tempdir().expect("temporary lock directory");
        let path = directory.path().join("install.lock");
        let first = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .expect("open first lock handle");
        first.try_lock().expect("acquire first lock");

        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .expect("open second lock handle");
        assert!(matches!(second.try_lock(), Err(TryLockError::WouldBlock)));

        drop(first);
        assert!(
            second.try_lock().is_ok(),
            "lock should release when owner exits"
        );
    }
}
