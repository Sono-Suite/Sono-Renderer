//! yt-dlp acquisition adapter for the renderer's existing file-based media inputs.
//!
//! This module ends at a validated local file. Project persistence, BGM and MV
//! consumption continue to use their existing `PathBuf` references.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const YTDLP_VERSION: &str = "2026.08.19";
const YTDLP_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe";
// SHA-256 published for the official yt-dlp 2026.08.19 Windows executable.
const YTDLP_SHA256: &str = "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a";
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 128 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 8 * 1024;
const MAX_TOOL_BYTES: u64 = 32 * 1024 * 1024;
const PROCESS_POLL: Duration = Duration::from_millis(100);
static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AcquisitionMode {
    Audio,
    AudioMp3,
    Video,
    VideoAudio,
}

impl AcquisitionMode {
    pub fn has_audio(self) -> bool {
        matches!(self, Self::Audio | Self::AudioMp3 | Self::VideoAudio)
    }

    pub fn has_video(self) -> bool {
        matches!(self, Self::Video | Self::VideoAudio)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio (source format)",
            Self::AudioMp3 => "Audio (MP3)",
            Self::Video => "Video (MP4-compatible)",
            Self::VideoAudio => "Video + audio (MP4-compatible)",
        }
    }

    fn requires_ffmpeg(self) -> bool {
        !matches!(self, Self::Audio)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration_seconds: Option<f64>,
    pub is_playlist: bool,
    pub is_live_or_upcoming: bool,
    pub live_status: Option<String>,
    pub format_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct YtDlpInfo {
    pub available: bool,
    pub version: Option<String>,
    pub source: &'static str,
    pub pinned_version: &'static str,
    pub update_policy: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcquiredMedia {
    pub path: PathBuf,
    pub sha256: String,
    pub duration_seconds: f64,
    pub has_audio: bool,
    pub has_video: bool,
    pub format_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AcquisitionStage {
    Inspecting,
    Waiting,
    Downloading,
    Processing,
    Validating,
    Importing,
    Complete,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcquisitionProgress {
    pub stage: AcquisitionStage,
    pub percent: Option<f32>,
    pub speed: Option<String>,
    pub eta: Option<String>,
    pub detail: String,
}

impl Default for AcquisitionProgress {
    fn default() -> Self {
        Self {
            stage: AcquisitionStage::Waiting,
            percent: None,
            speed: None,
            eta: None,
            detail: "Waiting to start".into(),
        }
    }
}

#[derive(Clone, Default)]
pub struct AcquisitionControl {
    canceled: Arc<AtomicBool>,
    ffmpeg: crate::export_control::ExportControl,
}

impl AcquisitionControl {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        self.ffmpeg.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

/// Inspect an already-installed tool without installing it or touching the
/// network. The application-managed binary is the only accepted source.
pub fn inspect_tool() -> Result<YtDlpInfo> {
    let path = managed_tool_path();
    if !path.is_file() {
        return Ok(YtDlpInfo {
            available: false,
            version: None,
            source: "managed",
            pinned_version: YTDLP_VERSION,
            update_policy: "updated only with a renderer release; no silent updates",
        });
    }
    validate_managed_tool(&path)?;
    let version = tool_version(&path)?;
    Ok(YtDlpInfo {
        available: true,
        version: Some(version),
        source: "managed",
        pinned_version: YTDLP_VERSION,
        update_policy: "updated only with a renderer release; no silent updates",
    })
}

/// Inspect one public HTTP(S) URL. No cookies, config files, plugins, or
/// authentication helpers are loaded by the managed extractor invocation.
pub fn inspect_url(
    url: &str,
    control: &AcquisitionControl,
    progress: &Arc<Mutex<AcquisitionProgress>>,
) -> Result<MediaInfo> {
    let url = validate_url(url)?;
    set_progress(
        progress,
        AcquisitionStage::Inspecting,
        "Inspecting source metadata",
    );
    let tool = ensure_managed_tool(control, progress)?;
    let args = vec![
        OsArg::from("--ignore-config"),
        OsArg::from("--no-plugin-dirs"),
        OsArg::from("--no-warnings"),
        OsArg::from("--no-color"),
        OsArg::from("--no-playlist"),
        OsArg::from("--skip-download"),
        OsArg::from("--dump-single-json"),
        OsArg::from("--"),
        OsArg::from(url),
    ];
    let output = run_child(&tool, &args, control, progress, MAX_METADATA_BYTES)?;
    if !output.status.success() {
        bail!(
            "Source inspection failed: {}",
            sanitize_diagnostic(&output.stderr)
        );
    }
    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("yt-dlp returned invalid or incomplete metadata JSON")?;
    let info = media_info(&raw);
    if info.is_playlist {
        bail!("This URL identifies a playlist or collection. Bulk acquisition is not supported; use a direct item URL.");
    }
    if info.is_live_or_upcoming {
        bail!("Live and upcoming sources are not supported by the ordinary media acquisition workflow.");
    }
    Ok(info)
}

/// Download a single item to a staging directory, validate the actual streams
/// with the shared FFprobe pair, then atomically publish a generated filename.
pub fn acquire(
    url: &str,
    mode: AcquisitionMode,
    destination: &Path,
    control: &AcquisitionControl,
    progress: &Arc<Mutex<AcquisitionProgress>>,
) -> Result<AcquiredMedia> {
    let url = validate_url(url)?;
    fs::create_dir_all(destination)
        .with_context(|| format!("creating media destination {}", destination.display()))?;
    if !destination.is_dir() {
        bail!(
            "media destination is not a directory: {}",
            destination.display()
        );
    }
    let info = inspect_url(&url, control, progress)?;
    if info.is_playlist || info.is_live_or_upcoming {
        bail!("This source is not a supported single, non-live media item.");
    }
    if control.is_cancelled() {
        bail!("media acquisition canceled");
    }
    let tool = ensure_managed_tool(control, progress)?;
    let ffmpeg = crate::ffmpeg::resolve_controlled(&control.ffmpeg)
        .context("resolving the existing managed FFmpeg/FFprobe pair")?;
    let staging = tempfile::Builder::new()
        .prefix(".sono-media-stage-")
        .tempdir_in(destination)
        .context("creating private media acquisition staging directory")?;
    let template = staging.path().join("acquired.%(ext)s");
    let args = build_download_args(mode, &url, &template, &ffmpeg.ffmpeg);
    set_progress(
        progress,
        AcquisitionStage::Downloading,
        "Downloading one media item",
    );
    let output = run_child(&tool, &args, control, progress, MAX_METADATA_BYTES)?;
    if !output.status.success() {
        bail!(
            "Media acquisition failed: {}",
            sanitize_diagnostic(&output.stderr)
        );
    }
    if control.is_cancelled() {
        bail!("media acquisition canceled");
    }
    set_progress(
        progress,
        AcquisitionStage::Validating,
        "Checking container and audio/video streams",
    );
    let staged_file = one_staged_media_file(staging.path())?;
    let probe = probe_media(&ffmpeg.ffprobe, &staged_file, control, progress)?;
    if mode.has_audio() && !probe.has_audio {
        bail!("The acquired file has no audio stream, but the selected mode requires audio.");
    }
    if mode.has_video() && !probe.has_video {
        bail!("The acquired file has no video stream, but the selected mode requires video.");
    }
    if !mode.has_audio() && !mode.has_video() {
        bail!("selected media mode does not request a media stream");
    }
    if control.is_cancelled() {
        bail!("media acquisition canceled");
    }
    set_progress(
        progress,
        AcquisitionStage::Importing,
        "Finalizing validated media file",
    );
    let extension = staged_file
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .context("acquired file has no valid extension")?;
    if !is_supported_extension(&extension) {
        bail!("yt-dlp produced an unsupported media container extension: {extension}");
    }
    let mut final_path = None;
    for _ in 0..32 {
        let candidate = destination.join(generated_filename(&extension));
        match fs::rename(&staged_file, &candidate) {
            Ok(()) => {
                final_path = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("atomically publishing acquired media"),
        }
    }
    let final_path = final_path.context("could not allocate a unique final media name")?;
    let sha256 = match crate::sha256_file(&final_path) {
        Ok((value, _)) => value,
        Err(error) => {
            let _ = fs::remove_file(&final_path);
            return Err(error).context("hashing the finalized media file");
        }
    };
    set_progress(
        progress,
        AcquisitionStage::Complete,
        "Media is ready for ordinary renderer inputs",
    );
    Ok(AcquiredMedia {
        path: final_path,
        sha256,
        duration_seconds: probe.duration_seconds,
        has_audio: probe.has_audio,
        has_video: probe.has_video,
        format_name: probe.format_name,
    })
}

fn build_download_args(
    mode: AcquisitionMode,
    url: &str,
    template: &Path,
    ffmpeg_path: &Path,
) -> Vec<OsArg> {
    let mut args = vec![
        OsArg::from("--ignore-config"),
        OsArg::from("--no-plugin-dirs"),
        OsArg::from("--no-warnings"),
        OsArg::from("--no-color"),
        OsArg::from("--no-playlist"),
        OsArg::from("--no-overwrites"),
        OsArg::from("--no-write-info-json"),
        OsArg::from("--no-write-thumbnail"),
        OsArg::from("--no-write-subs"),
        OsArg::from("--no-write-auto-subs"),
        OsArg::from("--newline"),
        OsArg::from("--progress-template"),
        OsArg::from(
            "download:%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s",
        ),
        OsArg::from("--output"),
        OsArg::from(template.as_os_str()),
        OsArg::from("--"),
        OsArg::from(url),
    ];
    match mode {
        AcquisitionMode::Audio => {
            args.splice(
                0..0,
                [OsArg::from("--format"), OsArg::from("bestaudio/best")],
            );
        }
        AcquisitionMode::AudioMp3 => {
            args.splice(
                0..0,
                [
                    OsArg::from("--format"),
                    OsArg::from("bestaudio/best"),
                    OsArg::from("--extract-audio"),
                    OsArg::from("--audio-format"),
                    OsArg::from("mp3"),
                    OsArg::from("--audio-quality"),
                    OsArg::from("0"),
                ],
            );
        }
        AcquisitionMode::Video => {
            args.splice(
                0..0,
                [
                    OsArg::from("--format"),
                    OsArg::from("bestvideo[ext=mp4]/bestvideo/best"),
                    OsArg::from("--remux-video"),
                    OsArg::from("mp4"),
                ],
            );
        }
        AcquisitionMode::VideoAudio => {
            args.splice(
                0..0,
                [
                    OsArg::from("--format"),
                    OsArg::from("bestvideo+bestaudio/best"),
                    OsArg::from("--merge-output-format"),
                    OsArg::from("mp4"),
                    OsArg::from("--remux-video"),
                    OsArg::from("mp4"),
                ],
            );
        }
    }
    if mode.requires_ffmpeg() {
        // Keep the invocation rooted to the already-validated renderer tools.
        let insertion = args.len() - 2;
        args.splice(
            insertion..insertion,
            [
                OsArg::from("--ffmpeg-location"),
                OsArg::from(ffmpeg_path.as_os_str()),
            ],
        );
    }
    args
}

pub fn default_media_directory() -> PathBuf {
    crate::store_paths::user_data_root().join("media")
}

pub fn validate_url(input: &str) -> Result<String> {
    let url = input.trim();
    if url.is_empty() || url.len() > MAX_URL_BYTES {
        bail!("enter one HTTP(S) URL no longer than {MAX_URL_BYTES} bytes");
    }
    if url
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
    {
        bail!("URL must not contain whitespace or control characters");
    }
    let (scheme, remainder) = url
        .split_once("://")
        .context("URL must use http:// or https://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        bail!("only http:// and https:// media URLs are supported");
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.starts_with('-') {
        bail!("URL must have a host and must not contain embedded credentials");
    }
    if authority.starts_with(':') || authority.ends_with(':') {
        bail!("URL host or port is malformed");
    }
    Ok(url.to_owned())
}

fn media_info(raw: &serde_json::Value) -> MediaInfo {
    let live_status = raw
        .get("live_status")
        .and_then(serde_json::Value::as_str)
        .map(|value| clean_text(value, 80));
    let is_live_or_upcoming = raw
        .get("is_live")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || matches!(live_status.as_deref(), Some("is_live" | "is_upcoming"));
    let is_playlist = raw.get("_type").and_then(serde_json::Value::as_str) == Some("playlist")
        || raw.get("entries").is_some_and(serde_json::Value::is_array);
    MediaInfo {
        title: raw
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(|value| clean_text(value, 320)),
        uploader: raw
            .get("uploader")
            .or_else(|| raw.get("channel"))
            .and_then(serde_json::Value::as_str)
            .map(|value| clean_text(value, 200)),
        duration_seconds: raw
            .get("duration")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0),
        is_playlist,
        is_live_or_upcoming,
        live_status,
        format_count: raw
            .get("formats")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
    }
}

fn clean_text(input: &str, max_chars: usize) -> String {
    input
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

fn managed_tool_path() -> PathBuf {
    crate::store_paths::user_data_root()
        .join("dependencies")
        .join("yt-dlp")
        .join(YTDLP_VERSION)
        .join("yt-dlp.exe")
}

fn ensure_managed_tool(
    control: &AcquisitionControl,
    progress: &Arc<Mutex<AcquisitionProgress>>,
) -> Result<PathBuf> {
    #[cfg(not(windows))]
    {
        let _ = (control, progress);
        bail!("managed yt-dlp provisioning is currently supported only by the Windows release");
    }
    #[cfg(windows)]
    {
        let target = managed_tool_path();
        if target.is_file() {
            validate_managed_tool(&target)?;
            return Ok(target);
        }
        if control.is_cancelled() {
            bail!("media acquisition canceled");
        }
        set_progress(
            progress,
            AcquisitionStage::Waiting,
            "Installing the pinned official yt-dlp release",
        );
        let parent = target
            .parent()
            .context("managed yt-dlp path has no parent")?;
        fs::create_dir_all(parent).context("creating managed yt-dlp directory")?;
        let staging = tempfile::Builder::new()
            .prefix(".yt-dlp-install-")
            .tempdir_in(parent)
            .context("creating yt-dlp install staging directory")?;
        let staged_executable = staging.path().join("yt-dlp.exe");
        let curl = system_curl()?;
        let args = vec![
            OsArg::from("--disable"),
            OsArg::from("--fail"),
            OsArg::from("--location"),
            OsArg::from("--silent"),
            OsArg::from("--show-error"),
            OsArg::from("--proto"),
            OsArg::from("=https"),
            OsArg::from("--tlsv1.2"),
            OsArg::from("--max-filesize"),
            OsArg::from(MAX_TOOL_BYTES.to_string()),
            OsArg::from("--output"),
            OsArg::from(staged_executable.as_os_str()),
            OsArg::from(YTDLP_URL),
        ];
        let output = run_child(&curl, &args, control, progress, 64 * 1024)?;
        if !output.status.success() {
            bail!("Could not install the pinned yt-dlp executable from its official HTTPS release: {}", sanitize_diagnostic(&output.stderr));
        }
        if control.is_cancelled() {
            bail!("media acquisition canceled");
        }
        let metadata = fs::metadata(&staged_executable)
            .context("official yt-dlp download did not create an executable")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_TOOL_BYTES {
            bail!("official yt-dlp executable has an invalid file size");
        }
        let (digest, _) = crate::sha256_file(&staged_executable)
            .context("calculating official yt-dlp executable SHA-256")?;
        if !digest.eq_ignore_ascii_case(YTDLP_SHA256) {
            bail!("official yt-dlp executable SHA-256 did not match the pinned release digest");
        }
        let stage_version = tool_version(&staged_executable)?;
        if stage_version.trim() != YTDLP_VERSION {
            bail!("official yt-dlp binary version did not match the pinned release version");
        }
        if target.exists() {
            validate_managed_tool(&target)?;
            return Ok(target);
        }
        match fs::rename(&staged_executable, &target) {
            Ok(()) => Ok(target),
            Err(_) if target.is_file() => {
                validate_managed_tool(&target)?;
                Ok(target)
            }
            Err(error) => Err(error).context("atomically installing the managed yt-dlp executable"),
        }
    }
}

#[cfg(windows)]
fn system_curl() -> Result<PathBuf> {
    let windows = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .context("Windows system directory is unavailable; cannot provision yt-dlp")?;
    let curl = windows.join("System32").join("curl.exe");
    if !curl.is_file() {
        bail!("Windows system curl.exe is unavailable; install the pinned yt-dlp dependency through an updated renderer package");
    }
    Ok(curl)
}

fn validate_managed_tool(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path).context("checking managed yt-dlp executable")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_TOOL_BYTES {
        bail!("managed yt-dlp executable has an invalid file size");
    }
    let (digest, _) = crate::sha256_file(path).context("validating managed yt-dlp SHA-256")?;
    if !digest.eq_ignore_ascii_case(YTDLP_SHA256) {
        bail!("managed yt-dlp executable failed its pinned SHA-256 check");
    }
    let version = tool_version(path)?;
    if version.trim() != YTDLP_VERSION {
        bail!(
            "managed yt-dlp version {} does not match pinned {}",
            version.trim(),
            YTDLP_VERSION
        );
    }
    Ok(())
}

fn tool_version(path: &Path) -> Result<String> {
    let control = AcquisitionControl::default();
    let progress = Arc::new(Mutex::new(AcquisitionProgress::default()));
    let output = run_child(path, &[OsArg::from("--version")], &control, &progress, 4096)?;
    if !output.status.success() {
        bail!(
            "yt-dlp version check failed: {}",
            sanitize_diagnostic(&output.stderr)
        );
    }
    String::from_utf8(output.stdout)
        .context("yt-dlp version output was not UTF-8")
        .map(|version| version.trim().to_owned())
}

#[cfg(windows)]
type OsArg = std::ffi::OsString;
#[cfg(not(windows))]
type OsArg = std::ffi::OsString;

fn run_child(
    executable: &Path,
    args: &[OsArg],
    control: &AcquisitionControl,
    progress: &Arc<Mutex<AcquisitionProgress>>,
    stdout_limit: usize,
) -> Result<ProcessOutput> {
    if control.is_cancelled() {
        bail!("media acquisition canceled");
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::export_control::hide_child_window(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("starting managed media tool at {}", executable.display()))?;
    let stdout = child.stdout.take().context("child stdout was not piped")?;
    let stderr = child.stderr.take().context("child stderr was not piped")?;
    let stdout_overflow = Arc::new(AtomicBool::new(false));
    let overflow_signal = stdout_overflow.clone();
    let stdout_reader = thread::spawn(move || read_bounded(stdout, stdout_limit, overflow_signal));
    let progress_for_reader = progress.clone();
    let stderr_reader =
        thread::spawn(move || read_stderr_lines(stderr, MAX_STDERR_BYTES, &progress_for_reader));
    let started = Instant::now();
    let status = loop {
        if control.is_cancelled() || stdout_overflow.load(Ordering::Acquire) {
            terminate_process_tree(&mut child);
            break child.wait().context("waiting for media tool termination")?;
        }
        if let Some(status) = child.try_wait().context("checking media tool process")? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60 * 60 * 24) {
            terminate_process_tree(&mut child);
            bail!("media tool exceeded the 24-hour operation limit");
        }
        thread::sleep(PROCESS_POLL);
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| anyhow!("media tool stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| anyhow!("media tool stderr reader panicked"))??;
    if control.is_cancelled() {
        bail!("media acquisition canceled");
    }
    if stdout_overflow.load(Ordering::Acquire) {
        bail!("media tool metadata output exceeded its safety limit");
    }
    Ok(ProcessOutput {
        status,
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: String,
}

fn read_bounded<R: Read>(
    mut reader: R,
    limit: usize,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(output.len());
        output.extend_from_slice(&chunk[..count.min(remaining)]);
        if count > remaining {
            overflow.store(true, Ordering::Release);
        }
    }
    Ok(output)
}

fn read_stderr_lines<R: Read>(
    mut reader: R,
    limit: usize,
    progress: &Arc<Mutex<AcquisitionProgress>>,
) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut line = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            if !line.is_empty() {
                observe_progress(&line, progress);
                append_tail(&mut output, &line, limit);
            }
            break;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                observe_progress(&line, progress);
                append_tail(&mut output, &line, limit);
                append_tail(&mut output, b"\n", limit);
                line.clear();
            } else if line.len() < MAX_STDERR_LINE_BYTES {
                line.push(*byte);
            }
        }
    }
    Ok(output)
}

fn append_tail(output: &mut Vec<u8>, bytes: &[u8], limit: usize) {
    if bytes.len() >= limit {
        output.clear();
        output.extend_from_slice(&bytes[bytes.len() - limit..]);
        return;
    }
    let excess = output
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(limit);
    if excess > 0 {
        output.drain(..excess);
    }
    output.extend_from_slice(bytes);
}

fn observe_progress(line: &[u8], progress: &Arc<Mutex<AcquisitionProgress>>) {
    let line = String::from_utf8_lossy(line).trim().to_owned();
    if line.is_empty() {
        return;
    }
    let mut state = progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if line.contains("download:") {
        let values = line
            .split_once("download:")
            .map(|(_, value)| value)
            .unwrap_or_default();
        let mut fields = values.split('|');
        state.percent = fields
            .next()
            .map(|value| value.trim().trim_end_matches('%'))
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
        state.speed = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| clean_text(value, 48));
        state.eta = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| clean_text(value, 48));
        state.stage = AcquisitionStage::Downloading;
        state.detail = "Downloading media".into();
    } else if [
        "[Merger]",
        "[ExtractAudio]",
        "[VideoRemuxer]",
        "[VideoConvertor]",
        "[Fixup",
    ]
    .iter()
    .any(|marker| line.contains(marker))
    {
        state.stage = AcquisitionStage::Processing;
        state.detail = "Processing or remuxing downloaded streams".into();
    }
}

fn set_progress(progress: &Arc<Mutex<AcquisitionProgress>>, stage: AcquisitionStage, detail: &str) {
    let mut state = progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.stage = stage;
    state.detail = detail.to_owned();
    if !matches!(stage, AcquisitionStage::Downloading) {
        state.percent = None;
        state.speed = None;
        state.eta = None;
    }
}

#[cfg(windows)]
fn terminate_process_tree(child: &mut Child) {
    if let Ok(windows) = std::env::var("WINDIR") {
        let taskkill = Path::new(&windows).join("System32").join("taskkill.exe");
        if taskkill.is_file() {
            let mut command = Command::new(taskkill);
            command.args(["/PID", &child.id().to_string(), "/T", "/F"]);
            crate::export_control::hide_child_window(&mut command);
            let _ = command.output();
        }
    }
    let _ = child.kill();
}

#[cfg(not(windows))]
fn terminate_process_tree(child: &mut Child) {
    let _ = child.kill();
}

struct ProbedMedia {
    duration_seconds: f64,
    has_audio: bool,
    has_video: bool,
    format_name: String,
}

fn probe_media(
    ffprobe: &Path,
    path: &Path,
    control: &AcquisitionControl,
    progress: &Arc<Mutex<AcquisitionProgress>>,
) -> Result<ProbedMedia> {
    let args = vec![
        OsArg::from("-v"),
        OsArg::from("error"),
        OsArg::from("-show_entries"),
        OsArg::from("format=format_name,duration:stream=codec_type,width,height,duration"),
        OsArg::from("-of"),
        OsArg::from("json"),
        OsArg::from(path.as_os_str()),
    ];
    let output = run_child(ffprobe, &args, control, progress, MAX_METADATA_BYTES)?;
    if !output.status.success() {
        bail!(
            "FFprobe could not validate acquired media: {}",
            sanitize_diagnostic(&output.stderr)
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("FFprobe returned invalid media metadata")?;
    let streams = value
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .context("FFprobe returned no stream list")?;
    let has_audio = streams
        .iter()
        .any(|stream| stream["codec_type"].as_str() == Some("audio"));
    let has_video = streams
        .iter()
        .any(|stream| stream["codec_type"].as_str() == Some("video"));
    let duration_seconds = value["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .or_else(|| {
            streams
                .iter()
                .filter_map(|stream| stream["duration"].as_str()?.parse::<f64>().ok())
                .find(|value| value.is_finite() && *value > 0.0)
        })
        .context("acquired file has no finite positive duration")?;
    let format_name = value["format"]["format_name"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| clean_text(value, 160))
        .context("acquired file has no recognized container format")?;
    if !has_audio && !has_video {
        bail!("acquired file contains neither audio nor video streams");
    }
    Ok(ProbedMedia {
        duration_seconds,
        has_audio,
        has_video,
        format_name,
    })
}

fn one_staged_media_file(directory: &Path) -> Result<PathBuf> {
    let mut media = Vec::new();
    for entry in fs::read_dir(directory).context("listing staged acquisition output")? {
        let entry = entry.context("reading staged acquisition entry")?;
        let file_type = entry
            .file_type()
            .context("checking staged acquisition entry")?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if is_supported_extension(extension) {
            media.push(path);
        }
    }
    if media.len() != 1 {
        bail!(
            "expected exactly one completed media file in staging, found {}",
            media.len()
        );
    }
    Ok(media.remove(0))
}

fn is_supported_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "aac"
            | "avi"
            | "flac"
            | "m4a"
            | "m4v"
            | "mkv"
            | "mov"
            | "mp3"
            | "mp4"
            | "ogg"
            | "opus"
            | "ts"
            | "wav"
            | "webm"
    )
}

fn generated_filename(extension: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "media-{timestamp}-{}-{counter}.{extension}",
        std::process::id()
    )
}

fn sanitize_diagnostic(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_STDERR_BYTES));
    for line in input.lines().take(80) {
        let mut redacted = String::new();
        let mut rest = line;
        loop {
            let http = rest.find("http://");
            let https = rest.find("https://");
            let start = match (http, https) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            let Some(start) = start else {
                redacted.push_str(rest);
                break;
            };
            redacted.push_str(&rest[..start]);
            let url_tail = &rest[start..];
            let end = url_tail.find(char::is_whitespace).unwrap_or(url_tail.len());
            let url = &url_tail[..end];
            let safe_end = url.find(['?', '#']).unwrap_or(url.len());
            redacted.push_str(&url[..safe_end]);
            if safe_end < url.len() {
                redacted.push_str("?[redacted]");
            }
            rest = &url_tail[end..];
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&clean_text(&redacted, 1024));
        if output.len() >= MAX_STDERR_BYTES {
            output.truncate(MAX_STDERR_BYTES);
            break;
        }
    }
    if output.is_empty() {
        "no diagnostic detail was returned".into()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_single_http_urls_without_credentials_are_accepted() {
        assert_eq!(
            validate_url(" https://example.test/watch?v=1 ").unwrap(),
            "https://example.test/watch?v=1"
        );
        for value in [
            "file:///C:/private.mp4",
            "javascript:alert(1)",
            "https://name:password@example.test/video",
            "https://example.test/a\nb",
            "-f bestaudio https://example.test/video",
        ] {
            assert!(validate_url(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn metadata_marks_playlists_live_sources_and_missing_fields_without_guessing() {
        let playlist =
            serde_json::json!({"_type":"playlist", "entries":[{}, {}], "title":"Collection"});
        let info = media_info(&playlist);
        assert!(info.is_playlist);
        assert_eq!(info.title.as_deref(), Some("Collection"));
        assert_eq!(info.duration_seconds, None);
        let live = serde_json::json!({"is_live":true, "live_status":"is_live"});
        assert!(media_info(&live).is_live_or_upcoming);
        assert_eq!(media_info(&serde_json::json!({})).title, None);
    }

    #[test]
    fn mode_stream_requirements_match_existing_music_and_mv_roles() {
        assert!(AcquisitionMode::Audio.has_audio());
        assert!(!AcquisitionMode::Audio.has_video());
        assert!(AcquisitionMode::AudioMp3.has_audio());
        assert!(AcquisitionMode::Video.has_video());
        assert!(!AcquisitionMode::Video.has_audio());
        assert!(AcquisitionMode::VideoAudio.has_audio());
        assert!(AcquisitionMode::VideoAudio.has_video());
    }

    #[test]
    fn generated_download_arguments_pin_safe_single_item_modes() {
        let audio = build_download_args(
            AcquisitionMode::Audio,
            "https://example.test/media?token=private",
            Path::new("C:/safe stage/acquired.%(ext)s"),
            Path::new("C:/validated/ffmpeg.exe"),
        );
        let audio = audio
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(audio.contains(&"--no-playlist".into()));
        assert!(audio.contains(&"--ignore-config".into()));
        assert!(audio.contains(&"--format".into()));
        assert!(!audio.contains(&"--ffmpeg-location".into()));
        assert_eq!(audio[audio.len() - 2], "--");
        assert_eq!(
            audio[audio.len() - 1],
            "https://example.test/media?token=private"
        );
        let video = build_download_args(
            AcquisitionMode::VideoAudio,
            "https://example.test/item",
            Path::new("C:/stage/file.%(ext)s"),
            Path::new("C:/validated/ffmpeg.exe"),
        );
        let video = video
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let ffmpeg_index = video
            .iter()
            .position(|value| value == "--ffmpeg-location")
            .unwrap();
        assert_eq!(video[ffmpeg_index + 1], "C:/validated/ffmpeg.exe");
        assert!(video.contains(&"--merge-output-format".into()));
        assert_eq!(video[video.len() - 2], "--");
    }

    #[test]
    fn progress_template_is_parsed_without_retaining_extractor_log() {
        let progress = Arc::new(Mutex::new(AcquisitionProgress::default()));
        observe_progress(b"download: 42.5%|1.3MiB/s|00:03", &progress);
        let state = progress.lock().unwrap();
        assert_eq!(state.stage, AcquisitionStage::Downloading);
        assert_eq!(state.percent, Some(42.5));
        assert_eq!(state.eta.as_deref(), Some("00:03"));
    }

    #[test]
    fn diagnostic_redacts_query_and_fragment_from_every_url() {
        let sanitized =
            sanitize_diagnostic("ERROR https://host.test/media?id=private&sig=secret#frag failed");
        assert!(sanitized.contains("https://host.test/media?[redacted]"));
        assert!(!sanitized.contains("private"));
        assert!(!sanitized.contains("secret"));
    }

    #[test]
    fn stderr_tail_and_metadata_capture_are_bounded() {
        let mut tail = Vec::new();
        append_tail(&mut tail, b"123456", 4);
        assert_eq!(tail, b"3456");
        let overflow = Arc::new(AtomicBool::new(false));
        let data = read_bounded(&b"abcdef"[..], 4, overflow.clone()).unwrap();
        assert_eq!(&data, b"abcd");
        assert!(overflow.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_is_shared_with_the_existing_ffmpeg_provisioner() {
        let control = AcquisitionControl::default();
        control.cancel();
        assert!(control.is_cancelled());
        assert!(control.ffmpeg.check().is_err());
        assert!(crate::ffmpeg::resolve_controlled(&control.ffmpeg).is_err());
    }
}
