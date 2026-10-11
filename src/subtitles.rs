//! Local speech transcription and interchange for Presentation Studio captions.
//!
//! The editor stores each cue as an ordinary `ClipKind::Text` clip. Its
//! `timeline_start`, `duration`, and `text` are the editable source of truth;
//! `CompositionClip::caption` only records import/transcription provenance.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const WHISPER_VERSION: &str = "1.9.2";
const WHISPER_RUNTIME_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip";
const WHISPER_RUNTIME_SHA256: &str =
    "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a";
const BASE_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper.cpp/resolve/main/ggml-base.bin";
const BASE_MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
const BASE_MODEL_BYTES: u64 = 147_951_465;
const MAX_MODEL_BYTES: u64 = 200_000_000;
const MAX_RUNTIME_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RUNTIME_UNPACKED_BYTES: u64 = 96 * 1024 * 1024;
const MAX_TRANSCRIPTION_SECONDS: f64 = 4.0 * 60.0 * 60.0;
const MAX_SUBTITLE_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_CUES: usize = 100_000;
const MAX_CUE_TEXT_BYTES: usize = 16 * 1024;
const MAX_SUBTITLE_TIME_SECONDS: f64 = 10.0 * 365.0 * 24.0 * 60.0 * 60.0;
const MAX_TRANSCRIPTION_DIAGNOSTIC_BYTES: usize = 8 * 1024;
const PROCESS_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    WebVtt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleCue {
    /// Time in the document containing this interchange record. During
    /// transcription, these values are relative to the selected timeline clip.
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechDependency {
    Runtime,
    MultilingualBaseModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechSetupStatus {
    pub runtime_installed: bool,
    pub model_installed: bool,
    pub runtime_path: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
}

impl SpeechSetupStatus {
    pub fn ready(&self) -> bool {
        self.runtime_installed && self.model_installed
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscriptionCacheInfo {
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscriptionCacheClearReport {
    pub entries: usize,
    pub bytes: u64,
}

pub fn setup_status() -> SpeechSetupStatus {
    let runtime_path = managed_runtime_path();
    let model_path = managed_model_path();
    let runtime_installed = is_regular_file_without_symlink(&runtime_path);
    // This is called by the immediate-mode GUI. Keep it metadata-only; the
    // full digest is checked in the background immediately before inference.
    let model_installed = fs::symlink_metadata(&model_path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() == BASE_MODEL_BYTES
    });
    SpeechSetupStatus {
        runtime_installed,
        model_installed,
        runtime_path: runtime_installed.then_some(runtime_path),
        model_path: model_installed.then_some(model_path),
    }
}

/// Install exactly one user-requested optional dependency. Downloads are
/// version-pinned, HTTPS-only, size-bounded, checksum-checked, and published
/// by rename only after validation succeeds.
pub fn install_dependency(
    dependency: SpeechDependency,
    control: &crate::export_control::ExportControl,
    mut progress: impl FnMut(String, Option<f32>),
) -> Result<PathBuf> {
    match dependency {
        SpeechDependency::Runtime => install_runtime(control, &mut progress),
        SpeechDependency::MultilingualBaseModel => install_model(control, &mut progress),
    }
}

pub fn parse_subtitle_text(text: &str, format: SubtitleFormat) -> Result<Vec<SubtitleCue>> {
    if text.len() > MAX_SUBTITLE_FILE_BYTES {
        bail!("subtitle file exceeds the 16 MiB safety limit");
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = normalized.lines().collect::<Vec<_>>();
    if format == SubtitleFormat::WebVtt {
        let first = lines.first().copied().unwrap_or_default().trim();
        if !first.starts_with("WEBVTT") {
            bail!("WebVTT file must start with a WEBVTT header");
        }
        if let Some(header_end) = lines.iter().position(|line| line.trim().is_empty()) {
            lines.drain(..=header_end);
        } else {
            return Ok(Vec::new());
        }
    }

    let mut cues = Vec::new();
    let mut cursor = 0;
    while cursor < lines.len() {
        while cursor < lines.len() && lines[cursor].trim().is_empty() {
            cursor += 1;
        }
        if cursor >= lines.len() {
            break;
        }
        let mut timing_index = cursor;
        while timing_index < lines.len() && !lines[timing_index].contains("-->") {
            timing_index += 1;
        }
        if timing_index == lines.len() {
            bail!(
                "subtitle cue beginning at line {} has no timing line",
                cursor + 1
            );
        }
        let timing = lines[timing_index];
        let (start_text, end_with_settings) = timing
            .split_once("-->")
            .context("subtitle timing line is missing the --> separator")?;
        let end_text = end_with_settings
            .split_whitespace()
            .next()
            .unwrap_or_default();
        let start = parse_timestamp(start_text.trim())
            .with_context(|| format!("invalid cue start time at line {}", timing_index + 1))?;
        let end = parse_timestamp(end_text)
            .with_context(|| format!("invalid cue end time at line {}", timing_index + 1))?;
        if end <= start {
            bail!(
                "subtitle cue at line {} ends before it starts",
                timing_index + 1
            );
        }
        cursor = timing_index + 1;
        let mut body = Vec::new();
        while cursor < lines.len() && !lines[cursor].trim().is_empty() {
            body.push(lines[cursor]);
            cursor += 1;
        }
        let text = body.join("\n").trim().to_owned();
        let text = if format == SubtitleFormat::WebVtt {
            strip_vtt_markup(&text)
        } else {
            text
        };
        if !text.is_empty() {
            validate_cue_text(&text)?;
            cues.push(SubtitleCue { start, end, text });
            if cues.len() > MAX_CUES {
                bail!("subtitle file contains more than {MAX_CUES} cues");
            }
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
    Ok(cues)
}

pub fn parse_subtitle_file(path: &Path) -> Result<(SubtitleFormat, Vec<SubtitleCue>)> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let format = match extension.as_str() {
        "srt" => SubtitleFormat::Srt,
        "vtt" | "webvtt" => SubtitleFormat::WebVtt,
        _ => bail!("subtitle import supports .srt and .vtt files"),
    };
    let metadata = fs::metadata(path).with_context(|| format!("checking {}", path.display()))?;
    if metadata.len() > MAX_SUBTITLE_FILE_BYTES as u64 {
        bail!("subtitle file exceeds the 16 MiB safety limit");
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("reading {} as UTF-8", path.display()))?;
    Ok((format, parse_subtitle_text(&text, format)?))
}

pub fn serialize_cues(cues: &[SubtitleCue], format: SubtitleFormat) -> Result<String> {
    validate_cues(cues)?;
    let mut output = String::new();
    if format == SubtitleFormat::WebVtt {
        output.push_str("WEBVTT\n\n");
    }
    for (index, cue) in cues.iter().enumerate() {
        if format == SubtitleFormat::Srt {
            output.push_str(&(index + 1).to_string());
            output.push('\n');
        }
        output.push_str(&format_timestamp(cue.start, format));
        output.push_str(" --> ");
        output.push_str(&format_timestamp(cue.end, format));
        output.push('\n');
        output.push_str(&cue.text);
        output.push_str("\n\n");
    }
    Ok(output)
}

pub fn cues_from_project(project: &crate::composition::CompositionProject) -> Vec<SubtitleCue> {
    let mut cues = project
        .clips
        .iter()
        .filter(|clip| clip.kind == crate::composition::ClipKind::Text && clip.caption.is_some())
        .map(|clip| SubtitleCue {
            start: clip.timeline_start,
            end: clip.timeline_start + clip.duration,
            text: clip.text.clone(),
        })
        .collect::<Vec<_>>();
    cues.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
    cues
}

/// Add ordinary text clips as one atomic project edit. `offset` maps cue-local
/// time to composition time; `limit_end` clips imported/generated cues to the
/// selected source range when provided.
pub fn insert_cues(
    project: &mut crate::composition::CompositionProject,
    cues: &[SubtitleCue],
    offset: f64,
    limit_end: Option<f64>,
    preset_id: &str,
    metadata: &crate::composition::CaptionMetadata,
) -> Result<Vec<String>> {
    validate_cues(cues)?;
    if !offset.is_finite() || offset < 0.0 {
        bail!("caption placement offset must be finite and nonnegative");
    }
    let frame = 1.0 / f64::from(project.fps);
    let end_limit = limit_end.unwrap_or(project.duration).min(project.duration);
    let mut candidate = project.clone();
    let mut ids = Vec::new();
    for cue in cues {
        let start = offset + cue.start;
        let end = (offset + cue.end).min(end_limit);
        if start >= end_limit || end - start < frame * 0.5 {
            continue;
        }
        let duration = (end - start).max(frame * 0.5).min(end_limit - start);
        let id = candidate.add_title_from_preset(preset_id, start, duration)?;
        let clip = candidate
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("inserted caption clip disappeared")?;
        clip.text.clone_from(&cue.text);
        clip.caption = Some(metadata.clone());
        ids.push(id);
    }
    if ids.is_empty() {
        bail!("no subtitle cues intersect the composition timeline");
    }
    candidate.validate()?;
    *project = candidate;
    Ok(ids)
}

pub fn split_caption_at(
    project: &mut crate::composition::CompositionProject,
    clip_id: &str,
    composition_time: f64,
) -> Result<String> {
    let original = project
        .clips
        .iter()
        .find(|clip| clip.id == clip_id)
        .context("selected caption clip no longer exists")?
        .clone();
    if original.kind != crate::composition::ClipKind::Text || original.caption.is_none() {
        bail!("select an imported or generated caption clip to split it");
    }
    let relative = composition_time - original.timeline_start;
    let frame = 1.0 / f64::from(project.fps);
    if relative < frame || relative > original.duration - frame {
        bail!("caption split point must leave at least one frame on each side");
    }
    let (left_text, right_text) =
        split_text_near_fraction(&original.text, relative / original.duration)
            .context("caption needs at least two words to split its text")?;

    let mut candidate = project.clone();
    let new_id =
        candidate.add_text_clip(right_text, composition_time, original.duration - relative)?;
    let new = candidate
        .clips
        .iter_mut()
        .find(|clip| clip.id == new_id)
        .context("new caption clip was not created")?;
    copy_text_presentation(&original, new);
    new.caption = original.caption.clone();
    if let Some(metadata) = &mut new.caption {
        for word in &mut metadata.words {
            if word.start >= relative {
                word.start -= relative;
                word.end -= relative;
            }
        }
        metadata
            .words
            .retain(|word| word.start >= 0.0 && word.end > word.start);
    }
    let left = candidate
        .clips
        .iter_mut()
        .find(|clip| clip.id == clip_id)
        .context("original caption clip disappeared")?;
    left.duration = relative;
    left.text = left_text;
    if let Some(metadata) = &mut left.caption {
        metadata.words.retain(|word| word.end <= relative);
    }
    // Timing is the user's chosen split point. Character distribution is only
    // used to find a readable word boundary, never to claim model word timing.
    candidate.validate()?;
    *project = candidate;
    Ok(new_id)
}

pub fn merge_captions(
    project: &mut crate::composition::CompositionProject,
    first_id: &str,
    second_id: &str,
) -> Result<()> {
    if first_id == second_id {
        bail!("select two different caption clips to merge");
    }
    let first = project
        .clips
        .iter()
        .find(|clip| clip.id == first_id)
        .context("first caption clip no longer exists")?
        .clone();
    let second = project
        .clips
        .iter()
        .find(|clip| clip.id == second_id)
        .context("second caption clip no longer exists")?
        .clone();
    if first.kind != crate::composition::ClipKind::Text
        || second.kind != crate::composition::ClipKind::Text
        || first.caption.is_none()
        || second.caption.is_none()
    {
        bail!("both selected clips must be captions");
    }
    if first.track_id != second.track_id || first.caption != second.caption {
        bail!("captions must share a track and source/model provenance to merge");
    }
    let (keep, remove) = if first.timeline_start <= second.timeline_start {
        (first, second)
    } else {
        (second, first)
    };
    let mut candidate = project.clone();
    let merged = candidate
        .clips
        .iter_mut()
        .find(|clip| clip.id == keep.id)
        .context("caption selected for merge disappeared")?;
    merged.duration =
        (remove.timeline_start + remove.duration - keep.timeline_start).max(keep.duration);
    if !merged.text.trim().is_empty() && !remove.text.trim().is_empty() {
        merged.text = format!("{} {}", keep.text.trim_end(), remove.text.trim_start());
    } else {
        merged.text = format!("{}{}", keep.text, remove.text);
    }
    if let Some(metadata) = &mut merged.caption {
        metadata.words.clear();
    }
    candidate.clips.retain(|clip| clip.id != remove.id);
    candidate.validate()?;
    *project = candidate;
    Ok(())
}

/// Transcribe one selected timeline clip, never the full project mix. The
/// extracted mono PCM follows the clip's source in-point and forward playback
/// rate, so recognizer-relative cue times map directly onto the clip timeline.
pub fn transcribe_clip(
    project: &crate::composition::CompositionProject,
    clip_id: &str,
    language: &str,
    control: &crate::export_control::ExportControl,
    mut progress: impl FnMut(String, Option<f32>),
) -> Result<Vec<SubtitleCue>> {
    let language = validate_language(language)?;
    let status = setup_status();
    let runtime = status.runtime_path.context(
        "Local speech recognition is not installed. Choose Install CPU recognizer (~8 MB) in Presentation Studio.",
    )?;
    let model = status.model_path.context(
        "The multilingual speech model is not installed. Choose Download Base model (~148 MB) in Presentation Studio.",
    )?;
    let (model_digest, model_bytes) =
        crate::sha256_file(&model).context("validating the installed local speech model")?;
    if model_bytes != BASE_MODEL_BYTES || !model_digest.eq_ignore_ascii_case(BASE_MODEL_SHA256) {
        bail!("the installed multilingual Base model failed SHA-256 validation; reinstall it from the speech setup controls");
    }
    if runtime_version(&runtime)? != WHISPER_VERSION {
        bail!("the installed whisper.cpp runtime version is invalid; reinstall the local CPU recognizer");
    }
    let clip = project
        .clips
        .iter()
        .find(|clip| clip.id == clip_id)
        .context("selected timeline clip no longer exists")?;
    if !matches!(
        clip.kind,
        crate::composition::ClipKind::Audio | crate::composition::ClipKind::Video
    ) {
        bail!("select an audio or video clip to transcribe");
    }
    if clip.source_time_mode != crate::composition::SourceTimeMode::Forward {
        bail!("transcription currently supports forward-played clips; reverse/freeze audio has no reliable speech-time mapping");
    }
    if clip.duration > MAX_TRANSCRIPTION_SECONDS {
        bail!("select a clip or trim its range to four hours or less before transcription");
    }
    let source_id = clip
        .source_id
        .as_deref()
        .context("selected clip has no media source")?;
    let source = project
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .context("selected clip's media source no longer exists")?;
    if !source.has_audio {
        bail!("selected media source does not contain an audio stream");
    }
    if source.nested_composition_path.is_some() {
        bail!("select an underlying audio/video media clip; nested composition transcription is not supported yet");
    }
    if !source.path.is_file() {
        bail!("selected media is missing: {}", source.path.display());
    }

    control.check()?;
    progress("Preparing selected clip audio locally…".into(), None);
    let work = tempfile::Builder::new()
        .prefix("sono-transcription-")
        .tempdir()
        .context("creating temporary transcription workspace")?;
    let wav_path = work.path().join("selected-clip-16k-mono.wav");
    extract_clip_audio(project, clip_id, &wav_path, control)?;
    control.check()?;

    let (audio_digest, audio_bytes) = crate::sha256_file(&wav_path)
        .context("hashing selected clip audio for local transcription cache")?;
    if audio_bytes < 44 {
        bail!("selected clip produced an empty audio file");
    }
    let key_material = format!(
        "whisper.cpp={WHISPER_VERSION}\nmodel=ggml-base:sha256={BASE_MODEL_SHA256}\nlanguage={language}\nsuppress_non_speech=true\nsegment-output=srt-v1\naudio={audio_digest}"
    );
    let cache_key = crate::content_hash::sha256_hex(key_material.as_bytes());
    if let Some(cues) = read_cached_transcript(&cache_key)? {
        progress("Using cached local transcription results".into(), Some(1.0));
        return Ok(cues);
    }

    progress(
        "Transcribing selected clip on this device (no audio upload)…".into(),
        None,
    );
    let output_prefix = work.path().join("transcript");
    let wav_arg = process_safe_path(&wav_path)?;
    let model_arg = process_safe_path(&model)?;
    let output_arg = process_safe_path(&output_prefix)?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 8);
    let mut command = Command::new(process_safe_path(&runtime)?);
    command
        .current_dir(work.path())
        .arg("-m")
        .arg(model_arg)
        .arg("-f")
        .arg(wav_arg)
        .arg("-of")
        .arg(output_arg)
        .arg("-osrt")
        .arg("-l")
        .arg(&language)
        .arg("-t")
        .arg(threads.to_string())
        .arg("-sns");
    command.stdout(Stdio::null());
    let output = control
        .output(&mut command)
        .context("running the local whisper.cpp CPU recognizer")?;
    if !output.status.success() {
        let details = bounded_diagnostic(&output.stderr);
        bail!(
            "local transcription failed (whisper.cpp exit {}). {}",
            output.status,
            details
        );
    }
    control.check()?;
    let srt_path = output_prefix.with_extension("srt");
    let (format, cues) = parse_subtitle_file(&srt_path)
        .with_context(|| format!("reading whisper.cpp output {}", srt_path.display()))?;
    debug_assert_eq!(format, SubtitleFormat::Srt);
    if cues.is_empty() {
        bail!("no speech segments were recognized in the selected clip");
    }
    let cues = cues
        .into_iter()
        .filter_map(|mut cue| {
            if cue.start >= clip.duration {
                return None;
            }
            cue.end = cue.end.min(clip.duration);
            (cue.end > cue.start).then_some(cue)
        })
        .collect::<Vec<_>>();
    if cues.is_empty() {
        bail!("recognizer output contained no cues inside the selected clip range");
    }
    write_cached_transcript(&cache_key, &cues)?;
    progress(
        "Transcription complete; cues are ready for review".into(),
        Some(1.0),
    );
    Ok(cues)
}

pub fn cache_info() -> Result<TranscriptionCacheInfo> {
    let Some(root) = validated_transcription_cache_root(false)? else {
        return Ok(TranscriptionCacheInfo::default());
    };
    let mut info = TranscriptionCacheInfo::default();
    for entry in fs::read_dir(&root).with_context(|| format!("scanning {}", root.display()))? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_file()
            && entry.path().extension().is_some_and(|ext| ext == "json")
        {
            info.entries += 1;
            info.bytes = info.bytes.saturating_add(metadata.len());
        }
    }
    Ok(info)
}

pub fn clear_cache() -> Result<TranscriptionCacheClearReport> {
    let Some(root) = validated_transcription_cache_root(false)? else {
        return Ok(TranscriptionCacheClearReport::default());
    };
    let mut report = TranscriptionCacheClearReport::default();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_file() && path.extension().is_some_and(|ext| ext == "json") {
            report.entries += 1;
            report.bytes = report.bytes.saturating_add(metadata.len());
            fs::remove_file(path)?;
        }
    }
    Ok(report)
}

fn extract_clip_audio(
    project: &crate::composition::CompositionProject,
    clip_id: &str,
    output: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    let clip = project
        .clips
        .iter()
        .find(|clip| clip.id == clip_id)
        .context("selected clip no longer exists")?;
    let source = project
        .sources
        .iter()
        .find(|source| clip.source_id.as_deref() == Some(source.id.as_str()))
        .context("selected clip source no longer exists")?;
    let duration = clip.duration * clip.playback_rate;
    let mut filter = format!(
        "atrim=start={:.6}:duration={duration:.6},asetpts=PTS-STARTPTS",
        clip.source_start
    );
    for tempo in crate::composition::atempo_factors(clip.playback_rate) {
        filter.push_str(&format!(",atempo={tempo:.6}"));
    }
    filter.push_str(",aresample=16000,aformat=sample_fmts=s16:channel_layouts=mono");
    let ffmpeg = crate::ffmpeg::resolve_controlled(control)?;
    let output = output.to_path_buf();
    let mut command = Command::new(ffmpeg.ffmpeg);
    command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(&source.path)
        .arg("-map")
        .arg("0:a:0")
        .arg("-vn")
        .arg("-af")
        .arg(filter)
        .arg("-t")
        .arg(format!("{:.6}", clip.duration))
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("16000")
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg(&output);
    let result = control
        .output(&mut command)
        .context("decoding the selected clip's trimmed audio to 16 kHz mono PCM")?;
    if !result.status.success() {
        bail!(
            "FFmpeg could not decode selected clip audio: {}",
            bounded_diagnostic(&result.stderr)
        );
    }
    let metadata = fs::metadata(&output).context("checking temporary transcription WAV")?;
    if !metadata.is_file() || metadata.len() < 44 {
        bail!("FFmpeg produced no usable audio samples for transcription");
    }
    Ok(())
}

fn install_runtime(
    control: &crate::export_control::ExportControl,
    progress: &mut impl FnMut(String, Option<f32>),
) -> Result<PathBuf> {
    #[cfg(not(windows))]
    {
        let _ = (control, progress);
        bail!("managed whisper.cpp provisioning is currently supported only by the Windows release; configure a local whisper-cli build on this platform");
    }
    #[cfg(windows)]
    {
        let parent = managed_runtime_path()
            .parent()
            .context("managed recognizer path has no parent")?
            .to_path_buf();
        let final_directory = parent.join(WHISPER_VERSION);
        let executable = final_directory.join("whisper-cli.exe");
        if is_regular_file_without_symlink(&executable)
            && runtime_version(&executable).is_ok_and(|version| version == WHISPER_VERSION)
        {
            return Ok(executable);
        }
        fs::create_dir_all(&parent).context("creating speech runtime dependency directory")?;
        let staging = tempfile::Builder::new()
            .prefix(".whisper-install-")
            .tempdir_in(&parent)
            .context("creating staged speech runtime directory")?;
        let archive_path = staging.path().join("whisper-runtime.zip");
        progress(
            "Downloading pinned whisper.cpp CPU runtime (~8 MB)…".into(),
            None,
        );
        download_checked_file(
            WHISPER_RUNTIME_URL,
            &archive_path,
            MAX_RUNTIME_ARCHIVE_BYTES,
            None,
            control,
            progress,
        )?;
        let (digest, _) =
            crate::sha256_file(&archive_path).context("hashing whisper.cpp runtime archive")?;
        if !digest.eq_ignore_ascii_case(WHISPER_RUNTIME_SHA256) {
            bail!("whisper.cpp runtime archive SHA-256 did not match the pinned v{WHISPER_VERSION} release");
        }
        let payload = staging.path().join("runtime");
        fs::create_dir(&payload).context("creating staged whisper.cpp runtime payload")?;
        extract_runtime_archive(&archive_path, &payload)?;
        let staged_executable = payload.join("whisper-cli.exe");
        if !staged_executable.is_file() {
            bail!("the official whisper.cpp archive did not contain a runnable whisper-cli.exe");
        }
        if runtime_version(&staged_executable)? != WHISPER_VERSION {
            bail!("downloaded whisper.cpp runtime version did not match the pinned v{WHISPER_VERSION}");
        }
        fs::write(
            payload.join("LICENSE-MIT.txt"),
            include_str!("../third_party_licenses/whisper.cpp-MIT.txt"),
        )
        .context("writing whisper.cpp runtime license notice")?;
        control.check()?;
        if final_directory.exists() {
            fs::remove_dir_all(&final_directory)
                .context("removing invalid old whisper.cpp runtime")?;
        }
        fs::rename(&payload, &final_directory)
            .context("atomically installing whisper.cpp CPU runtime")?;
        Ok(executable)
    }
}

fn install_model(
    control: &crate::export_control::ExportControl,
    progress: &mut impl FnMut(String, Option<f32>),
) -> Result<PathBuf> {
    let target = managed_model_path();
    if target.is_file()
        && target
            .metadata()
            .is_ok_and(|metadata| metadata.len() == BASE_MODEL_BYTES)
        && crate::sha256_file(&target)
            .is_ok_and(|(digest, _)| digest.eq_ignore_ascii_case(BASE_MODEL_SHA256))
    {
        return Ok(target);
    }
    let parent = target
        .parent()
        .context("managed speech model path has no parent")?
        .to_path_buf();
    fs::create_dir_all(&parent).context("creating local speech model directory")?;
    let staging = tempfile::Builder::new()
        .prefix(".whisper-model-")
        .tempdir_in(&parent)
        .context("creating staged speech model directory")?;
    let staged_model = staging.path().join("ggml-base.bin");
    progress(
        "Downloading multilingual Base speech model (147,951,465 bytes)…".into(),
        Some(0.0),
    );
    download_checked_file(
        BASE_MODEL_URL,
        &staged_model,
        MAX_MODEL_BYTES,
        Some(BASE_MODEL_BYTES),
        control,
        progress,
    )?;
    let (digest, bytes) = crate::sha256_file(&staged_model)
        .context("hashing downloaded multilingual speech model")?;
    if bytes != BASE_MODEL_BYTES || !digest.eq_ignore_ascii_case(BASE_MODEL_SHA256) {
        bail!("downloaded multilingual Base model failed its pinned size/SHA-256 validation");
    }
    control.check()?;
    if target.exists() {
        fs::remove_file(&target).context("removing invalid previous speech model")?;
    }
    fs::rename(&staged_model, &target).context("atomically installing multilingual Base model")?;
    fs::write(
        parent.join("MODEL-LICENSE.txt"),
        "The ggml-org/whisper.cpp ggml-base model is distributed under the MIT License.\nSee https://huggingface.co/ggml-org/whisper.cpp for the model card and attribution.\n",
    )
    .context("writing local speech model license notice")?;
    progress("Multilingual Base model installed".into(), Some(1.0));
    Ok(target)
}

fn download_checked_file(
    url: &str,
    target: &Path,
    max_bytes: u64,
    expected_bytes: Option<u64>,
    control: &crate::export_control::ExportControl,
    progress: &mut impl FnMut(String, Option<f32>),
) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = (url, target, max_bytes, expected_bytes, control, progress);
        bail!("managed speech dependency downloads are currently supported only on Windows");
    }
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .context("Windows system directory is unavailable for secure dependency download")?;
        let curl = windir.join("System32").join("curl.exe");
        if !curl.is_file() {
            bail!("Windows system curl.exe is unavailable; download/install the speech dependency through the updated Windows package");
        }
        let mut stderr = tempfile::tempfile().context("creating speech downloader stderr sink")?;
        let mut command = Command::new(curl);
        command
            .arg("--fail")
            .arg("--location")
            .arg("--silent")
            .arg("--show-error")
            .arg("--proto")
            .arg("=https")
            .arg("--proto-redir")
            .arg("=https")
            .arg("--tlsv1.2")
            .arg("--max-filesize")
            .arg(max_bytes.to_string())
            .arg("--output")
            .arg(target)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr.try_clone()?));
        crate::export_control::hide_child_window(&mut command);
        let mut child = command
            .spawn()
            .context("starting Windows HTTPS dependency downloader")?;
        let mut last_reported = 0_u64;
        let status = loop {
            if control.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(target);
                bail!("speech dependency download canceled");
            }
            if let Ok(metadata) = fs::metadata(target) {
                if metadata.len() > max_bytes {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_file(target);
                    bail!("speech dependency download exceeded its size limit");
                }
                let downloaded = metadata.len();
                if downloaded.saturating_sub(last_reported) >= 512 * 1024 {
                    last_reported = downloaded;
                    let fraction =
                        expected_bytes
                            .filter(|expected| *expected > 0)
                            .map(|expected| {
                                (downloaded as f64 / expected as f64).clamp(0.0, 0.99) as f32
                            });
                    progress(
                        format!(
                            "Downloaded {} / {} bytes",
                            downloaded,
                            expected_bytes.map_or("unknown".into(), |v| v.to_string())
                        ),
                        fraction,
                    );
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(PROCESS_POLL),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_file(target);
                    return Err(error).context("checking speech dependency download process");
                }
            }
        };
        stderr.seek(std::io::SeekFrom::Start(0))?;
        let mut diagnostics = Vec::new();
        stderr
            .take(MAX_TRANSCRIPTION_DIAGNOSTIC_BYTES as u64)
            .read_to_end(&mut diagnostics)?;
        if !status.success() {
            let _ = fs::remove_file(target);
            bail!(
                "HTTPS speech dependency download failed: {}",
                bounded_diagnostic(&diagnostics)
            );
        }
        control.check()?;
        let metadata = fs::metadata(target).context("download did not create speech dependency")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes {
            let _ = fs::remove_file(target);
            bail!("downloaded speech dependency has an invalid file size");
        }
        if expected_bytes.is_some_and(|expected| metadata.len() != expected) {
            let _ = fs::remove_file(target);
            bail!("downloaded speech model size did not match the upstream pinned size");
        }
        progress(
            "Dependency download finished; validating contents…".into(),
            Some(1.0),
        );
        Ok(())
    }
}

#[cfg(windows)]
fn extract_runtime_archive(archive_path: &Path, output: &Path) -> Result<()> {
    let file = File::open(archive_path).context("opening pinned whisper.cpp runtime archive")?;
    let mut archive = zip::ZipArchive::new(file).context("reading pinned whisper.cpp ZIP")?;
    if archive.len() > 128 {
        bail!("whisper.cpp runtime archive contains too many entries");
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let size = entry.size();
        total = total.saturating_add(size);
        if size == 0 || size > MAX_RUNTIME_UNPACKED_BYTES || total > MAX_RUNTIME_UNPACKED_BYTES {
            bail!("whisper.cpp runtime archive exceeds the extraction size limit");
        }
        let leaf = Path::new(entry.name())
            .file_name()
            .and_then(|name| name.to_str())
            .context("whisper.cpp archive contains a non-UTF-8 entry name")?;
        if !leaf.to_ascii_lowercase().ends_with(".dll")
            && !leaf.eq_ignore_ascii_case("whisper-cli.exe")
        {
            continue;
        }
        if leaf.contains(['/', '\\', ':']) || leaf == "." || leaf == ".." {
            bail!("whisper.cpp archive contains an unsafe file name");
        }
        let destination = output.join(leaf);
        if destination.exists() {
            bail!("whisper.cpp archive contains duplicate runtime file names");
        }
        let mut target = File::create(destination)?;
        std::io::copy(&mut entry, &mut target)
            .context("extracting pinned whisper.cpp runtime file")?;
        target.flush()?;
    }
    Ok(())
}

fn runtime_version(path: &Path) -> Result<String> {
    let path = process_safe_path(path)?;
    let mut command = Command::new(path);
    command.arg("--version");
    let output = crate::export_control::ExportControl::default()
        .output(&mut command)
        .context("checking whisper.cpp executable version")?;
    if !output.status.success() {
        bail!("whisper.cpp --version exited with {}", output.status);
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if text.len() > 16 * 1024 {
        bail!("whisper.cpp version response is unexpectedly large");
    }
    let lowered = text.to_ascii_lowercase();
    if !lowered.contains("whisper") || !lowered.contains(WHISPER_VERSION) {
        bail!("whisper.cpp executable did not report the pinned v{WHISPER_VERSION} version");
    }
    Ok(WHISPER_VERSION.into())
}

fn managed_runtime_path() -> PathBuf {
    crate::store_paths::user_data_root()
        .join("dependencies")
        .join("whisper.cpp")
        .join(WHISPER_VERSION)
        .join(if cfg!(windows) {
            "whisper-cli.exe"
        } else {
            "whisper-cli"
        })
}

fn is_regular_file_without_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
}

fn managed_model_path() -> PathBuf {
    crate::store_paths::user_data_root()
        .join("models")
        .join("whisper.cpp")
        .join("ggml-base.bin")
}

fn transcription_cache_root() -> PathBuf {
    crate::store_paths::cache_root().join("speech-transcription-v1")
}

fn validated_transcription_cache_root(create_if_missing: bool) -> Result<Option<PathBuf>> {
    let root = transcription_cache_root();
    let metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create_if_missing => {
            return Ok(None);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(&root).context("creating local transcription cache")?;
            fs::symlink_metadata(&root).context("checking newly created transcription cache")?
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "checking local transcription cache directory {}",
                    root.display()
                )
            });
        }
    };
    match metadata {
        metadata if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            Ok(Some(root))
        }
        _ => bail!(
            "refusing to use unsafe transcription cache path {}",
            root.display()
        ),
    }
}

fn read_cached_transcript(key: &str) -> Result<Option<Vec<SubtitleCue>>> {
    validate_cache_key(key)?;
    let Some(root) = validated_transcription_cache_root(false)? else {
        return Ok(None);
    };
    let path = root.join(format!("{key}.json"));
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("checking local transcription cache entry"),
    };
    if !metadata.file_type().is_file() || metadata.len() > 4 * 1024 * 1024 {
        let _ = fs::remove_file(&path);
        return Ok(None);
    }
    let bytes = fs::read(&path).context("reading local transcription cache entry")?;
    let cues = match serde_json::from_slice::<Vec<SubtitleCue>>(&bytes) {
        Ok(cues) if validate_cues(&cues).is_ok() => cues,
        _ => {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    };
    Ok(Some(cues))
}

fn write_cached_transcript(key: &str, cues: &[SubtitleCue]) -> Result<()> {
    validate_cache_key(key)?;
    let root = validated_transcription_cache_root(true)?
        .context("local transcription cache directory was not created")?;
    let destination = root.join(format!("{key}.json"));
    let staging = tempfile::Builder::new()
        .prefix(".transcript-cache-")
        .tempfile_in(&root)
        .context("staging local transcription cache record")?;
    serde_json::to_writer(staging.as_file(), cues)
        .context("serializing raw transcript cache record")?;
    staging
        .as_file()
        .sync_all()
        .context("flushing raw transcript cache record")?;
    staging
        .persist(&destination)
        .map_err(|error| error.error)
        .context("atomically publishing transcript cache record")?;
    prune_transcription_cache(&root)?;
    Ok(())
}

fn prune_transcription_cache(root: &Path) -> Result<()> {
    let mut entries = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        let metadata = fs::symlink_metadata(item.path())?;
        if metadata.file_type().is_file()
            && item.path().extension().is_some_and(|ext| ext == "json")
        {
            entries.push((
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                metadata.len(),
                item.path(),
            ));
        }
    }
    entries.sort_by_key(|(modified, _, _)| *modified);
    let mut total = entries
        .iter()
        .fold(0_u64, |sum, (_, size, _)| sum.saturating_add(*size));
    while entries.len() > 512 || total > 64 * 1024 * 1024 {
        let (_, size, path) = entries.remove(0);
        fs::remove_file(path).context("pruning oldest local transcription cache entry")?;
        total = total.saturating_sub(size);
    }
    Ok(())
}

fn validate_cache_key(key: &str) -> Result<()> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("transcription cache key is invalid");
    }
    Ok(())
}

fn validate_language(language: &str) -> Result<String> {
    let language = language.trim().to_ascii_lowercase();
    if language != "auto" && !(language.len() == 2 || language.len() == 3)
        || !language.bytes().all(|byte| byte.is_ascii_lowercase())
    {
        bail!("language must be `auto` or a two/three-letter Whisper language code such as `en` or `ja`");
    }
    Ok(language)
}

fn validate_cues(cues: &[SubtitleCue]) -> Result<()> {
    if cues.len() > MAX_CUES {
        bail!("subtitle cue count exceeds {MAX_CUES}");
    }
    for cue in cues {
        if !cue.start.is_finite()
            || !cue.end.is_finite()
            || cue.start < 0.0
            || cue.end <= cue.start
            || cue.end > MAX_SUBTITLE_TIME_SECONDS
        {
            bail!("subtitle cue times must be finite, nonnegative, and end after start");
        }
        validate_cue_text(&cue.text)?;
    }
    Ok(())
}

fn validate_cue_text(text: &str) -> Result<()> {
    if text.len() > MAX_CUE_TEXT_BYTES {
        bail!("subtitle cue text exceeds 16 KiB");
    }
    if text
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        bail!("subtitle cue contains unsupported control characters");
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<f64> {
    let value = value.trim();
    let (whole, fraction) = value
        .split_once([',', '.'])
        .map_or((value, "0"), |(whole, fraction)| (whole, fraction));
    if fraction.is_empty()
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        bail!("timestamp fractional part must contain one to three digits");
    }
    let millis = fraction.parse::<u64>()?
        * match fraction.len() {
            1 => 100,
            2 => 10,
            _ => 1,
        };
    let parts = whole.split(':').collect::<Vec<_>>();
    if parts.len() != 2 && parts.len() != 3 {
        bail!("timestamp must use MM:SS,mmm or HH:MM:SS,mmm form");
    }
    let numbers = parts
        .iter()
        .map(|part| {
            part.parse::<u64>()
                .context("timestamp component is not an integer")
        })
        .collect::<Result<Vec<_>>>()?;
    let (hours, minutes, seconds) = if numbers.len() == 3 {
        (numbers[0], numbers[1], numbers[2])
    } else {
        (0, numbers[0], numbers[1])
    };
    if minutes > 59 || seconds > 59 {
        bail!("timestamp minutes and seconds must be below 60");
    }
    let whole_seconds = hours
        .checked_mul(3600)
        .and_then(|value| value.checked_add(minutes * 60))
        .and_then(|value| value.checked_add(seconds))
        .context("timestamp exceeds the supported range")?;
    let timestamp = whole_seconds as f64 + millis as f64 / 1000.0;
    if timestamp > MAX_SUBTITLE_TIME_SECONDS {
        bail!("subtitle timestamp exceeds the ten-year safety limit");
    }
    Ok(timestamp)
}

fn format_timestamp(seconds: f64, format: SubtitleFormat) -> String {
    let total_millis = (seconds.max(0.0) * 1000.0).round() as u64;
    let hours = total_millis / 3_600_000;
    let minutes = (total_millis / 60_000) % 60;
    let secs = (total_millis / 1000) % 60;
    let millis = total_millis % 1000;
    match format {
        SubtitleFormat::Srt => format!("{hours:02}:{minutes:02}:{secs:02},{millis:03}"),
        SubtitleFormat::WebVtt => format!("{hours:02}:{minutes:02}:{secs:02}.{millis:03}"),
    }
}

fn strip_vtt_markup(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_tag = false;
    for character in text.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => result.push(character),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{00a0}")
}

fn split_text_near_fraction(text: &str, fraction: f64) -> Option<(String, String)> {
    let total = text.chars().count();
    if total < 2 {
        return None;
    }
    let target = ((total as f64 * fraction).round() as usize).clamp(1, total - 1);
    let boundaries = text
        .char_indices()
        .filter_map(|(byte, character)| {
            character
                .is_whitespace()
                .then_some((byte, text[..byte].chars().count()))
        })
        .filter(|(_, character_index)| *character_index > 0 && *character_index < total)
        .collect::<Vec<_>>();
    let (byte, _index) = boundaries
        .into_iter()
        .min_by_key(|(_, character_index)| character_index.abs_diff(target))?;
    let left = text[..byte].trim_end().to_owned();
    let right = text[byte..].trim_start().to_owned();
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

fn copy_text_presentation(
    source: &crate::composition::CompositionClip,
    target: &mut crate::composition::CompositionClip,
) {
    target.name = source.name.clone();
    target.track_id.clone_from(&source.track_id);
    target.z_order = source.z_order;
    target.x = source.x;
    target.y = source.y;
    target.scale_x = source.scale_x;
    target.scale_y = source.scale_y;
    target.rotation_degrees = source.rotation_degrees;
    target.opacity = source.opacity;
    target.effects.clone_from(&source.effects);
    target.mask.clone_from(&source.mask);
    target.font_path.clone_from(&source.font_path);
    target.text_alignment = source.text_alignment;
    target.text_background_enabled = source.text_background_enabled;
    target
        .text_background_color
        .clone_from(&source.text_background_color);
    target.text_background_opacity = source.text_background_opacity;
    target.text_background_padding = source.text_background_padding;
    target.font_size = source.font_size;
    target.text_color.clone_from(&source.text_color);
    target.outline_color.clone_from(&source.outline_color);
    target.outline_width = source.outline_width;
    target.shadow_x = source.shadow_x;
    target.shadow_y = source.shadow_y;
    target
        .position_keyframes
        .clone_from(&source.position_keyframes);
    target
        .animated_properties
        .clone_from(&source.animated_properties);
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let clean = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect::<String>();
    clean
        .chars()
        .take(2_000)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(windows)]
fn process_safe_path(path: &Path) -> Result<std::ffi::OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let text = path.to_string_lossy();
    if text.is_ascii() {
        return Ok(path.as_os_str().to_os_string());
    }
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // whisper.cpp's Windows runtime has used narrow path conversion in some
    // releases. Passing an available DOS short path avoids corrupting ordinary
    // Unicode profile/temp directories. The buffer length is queried first.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetShortPathNameW(long_path: *const u16, short_path: *mut u16, capacity: u32) -> u32;
    }
    let required = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if required == 0 {
        bail!("speech runtime cannot safely address this non-ASCII path; enable Windows 8.3 short names or move the app data to an ASCII-only path");
    }
    let mut short = vec![0_u16; required as usize + 1];
    let written =
        unsafe { GetShortPathNameW(wide.as_ptr(), short.as_mut_ptr(), short.len() as u32) };
    if written == 0 || written as usize >= short.len() {
        bail!("could not obtain a Windows short path for the local speech runtime");
    }
    Ok(std::ffi::OsString::from_wide(&short[..written as usize]))
}

#[cfg(not(windows))]
fn process_safe_path(path: &Path) -> Result<std::ffi::OsString> {
    Ok(path.as_os_str().to_os_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_round_trip_preserves_unicode_multiline_text_and_millisecond_times() {
        let cues = vec![
            SubtitleCue {
                start: 2.125,
                end: 3.75,
                text: "こんにちは,世界\nsecond line".into(),
            },
            SubtitleCue {
                start: 4.0,
                end: 4.51,
                text: "Café & music".into(),
            },
        ];
        let srt = serialize_cues(&cues, SubtitleFormat::Srt).unwrap();
        assert!(srt.contains("00:00:02,125 --> 00:00:03,750"));
        assert_eq!(
            parse_subtitle_text(&srt, SubtitleFormat::Srt).unwrap(),
            cues
        );
    }

    #[test]
    fn webvtt_accepts_settings_speaker_tags_and_dot_timestamps() {
        let vtt = "WEBVTT\n\ncue-id\n00:00:01.250 --> 00:00:02.500 align:center\n<v Alex>こんにちは &amp; welcome</v>\n";
        let cues = parse_subtitle_text(vtt, SubtitleFormat::WebVtt).unwrap();
        assert_eq!(cues[0].start, 1.25);
        assert_eq!(cues[0].end, 2.5);
        assert_eq!(cues[0].text, "こんにちは & welcome");
        let encoded = serialize_cues(&cues, SubtitleFormat::WebVtt).unwrap();
        assert!(encoded.starts_with("WEBVTT\n\n"));
        assert!(encoded.contains("00:00:01.250 --> 00:00:02.500"));
    }

    #[test]
    fn malformed_or_oversized_subtitle_data_is_rejected() {
        assert!(parse_subtitle_text(
            "1\n00:00:03,000 --> 00:00:02,000\nbackwards",
            SubtitleFormat::Srt
        )
        .is_err());
        assert!(parse_subtitle_text("not vtt", SubtitleFormat::WebVtt).is_err());
        assert!(parse_subtitle_text(
            "1\n18446744073709551615:00:00,000 --> 18446744073709551615:00:01,000\nway too long",
            SubtitleFormat::Srt
        )
        .is_err());
        assert!(parse_subtitle_text(
            &"x".repeat(MAX_SUBTITLE_FILE_BYTES + 1),
            SubtitleFormat::Srt
        )
        .is_err());
    }

    #[test]
    fn automatic_captions_keep_timeline_text_authoritative() {
        let mut project = crate::composition::CompositionProject::new();
        project.duration = 10.0;
        let ids = insert_cues(
            &mut project,
            &[SubtitleCue {
                start: 1.0,
                end: 2.0,
                text: "A line".into(),
            }],
            3.0,
            Some(5.0),
            "builtin-subtitle-banner",
            &crate::composition::CaptionMetadata {
                origin: crate::composition::CaptionOrigin::Transcribed,
                source_id: Some("source-a".into()),
                source_clip_id: Some("video-a".into()),
                language: Some("en".into()),
                model_id: Some("whisper.cpp 1.9.2 / ggml-base".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let caption = project.clips.iter().find(|clip| clip.id == ids[0]).unwrap();
        assert_eq!(caption.timeline_start, 4.0);
        assert_eq!(caption.duration, 1.0);
        assert_eq!(caption.text, "A line");
        assert_eq!(cues_from_project(&project).len(), 1);
        let serialized = serde_json::to_vec(&project).unwrap();
        let reopened: crate::composition::CompositionProject =
            serde_json::from_slice(&serialized).unwrap();
        assert_eq!(cues_from_project(&reopened), cues_from_project(&project));
    }

    #[test]
    fn caption_split_and_merge_are_safe_project_edits() {
        let mut project = crate::composition::CompositionProject::new();
        project.duration = 10.0;
        let metadata = crate::composition::CaptionMetadata {
            origin: crate::composition::CaptionOrigin::Imported,
            ..Default::default()
        };
        let ids = insert_cues(
            &mut project,
            &[SubtitleCue {
                start: 1.0,
                end: 3.0,
                text: "hello there friend".into(),
            }],
            0.0,
            None,
            "builtin-subtitle-banner",
            &metadata,
        )
        .unwrap();
        let right = split_caption_at(&mut project, &ids[0], 2.0).unwrap();
        assert_eq!(
            project
                .clips
                .iter()
                .filter(|clip| clip.caption.is_some())
                .count(),
            2
        );
        merge_captions(&mut project, &ids[0], &right).unwrap();
        assert_eq!(
            project
                .clips
                .iter()
                .filter(|clip| clip.caption.is_some())
                .count(),
            1
        );
        assert_eq!(
            project
                .clips
                .iter()
                .find(|clip| clip.id == ids[0])
                .unwrap()
                .text,
            "hello there friend"
        );
    }

    #[test]
    #[ignore = "requires an already-installed, validated FFmpeg/FFprobe pair"]
    fn real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm() -> Result<()> {
        let tools = crate::ffmpeg::inspect_existing()?
            .context("real-media subtitle extraction requires the managed FFmpeg pair")?;
        let directory = tempfile::tempdir()?;
        let source_path = directory.path().join("source.wav");
        let generated = crate::export_control::ExportControl::default().output(
            Command::new(&tools.ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000:duration=3",
                    "-c:a",
                    "pcm_s16le",
                ])
                .arg(&source_path),
        )?;
        anyhow::ensure!(
            generated.status.success(),
            "could not generate the real-media audio fixture"
        );

        let mut project = crate::composition::CompositionProject::new();
        project.duration = 5.0;
        project.sources.push(crate::composition::MediaSource {
            id: "voice-source".into(),
            path: source_path,
            display_name: "Voice source".into(),
            has_audio: true,
            duration: 3.0,
            ..Default::default()
        });
        let clip_id = project.add_clip("voice-source", crate::composition::ClipKind::Audio, 1.0)?;
        let clip = project
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .unwrap();
        clip.source_start = 0.5;
        clip.duration = 0.5;
        clip.playback_rate = 2.0;
        project.validate()?;

        let extracted = directory.path().join("selected-clip.wav");
        extract_clip_audio(
            &project,
            &clip_id,
            &extracted,
            &crate::export_control::ExportControl::default(),
        )?;
        let probe = crate::export_control::ExportControl::default().output(
            Command::new(&tools.ffprobe)
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=sample_rate,channels",
                    "-show_entries",
                    "format=duration",
                    "-of",
                    "json",
                ])
                .arg(&extracted),
        )?;
        anyhow::ensure!(
            probe.status.success(),
            "FFprobe could not inspect extracted clip audio"
        );
        let inspected: serde_json::Value = serde_json::from_slice(&probe.stdout)?;
        anyhow::ensure!(inspected["streams"][0]["sample_rate"] == "16000");
        anyhow::ensure!(inspected["streams"][0]["channels"] == 1);
        let duration = inspected["format"]["duration"]
            .as_str()
            .context("FFprobe omitted WAV duration")?
            .parse::<f64>()?;
        anyhow::ensure!(
            (duration - 0.5).abs() < 0.025,
            "retimed clip audio duration was {duration:.3}s, expected 0.5s"
        );
        Ok(())
    }
}
