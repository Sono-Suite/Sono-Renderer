//! Local, reviewable tempo proposals for Chart Studio.
//!
//! Decoding is delegated to the application's managed FFmpeg installation,
//! the same decoder used by its media/waveform and preview paths. Only a small
//! compact onset envelope is retained; decoded song PCM is streamed and dropped.
//! Detector output is disposable. Accepted timing is a separate Chart Studio
//! authoring command and is never inferred from this cache.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;
pub const DETECTOR_VERSION: &str = "sono-onset-autocorrelation-2";
const ENVELOPE_HZ: usize = 220;
const DECODE_RATE: u32 = 11_000;
const FRAME_SAMPLES: usize = DECODE_RATE as usize / ENVELOPE_HZ;
const MIN_BPM: f64 = 40.0;
const MAX_BPM: f64 = 240.0;
const MIN_RHYTHM_SCORE: f64 = 0.18;
const MAX_ANALYSIS_SECONDS: f64 = 4.0 * 60.0 * 60.0;
const MAX_CACHE_ENTRIES: usize = 256;
const GLOBAL_CORRELATION_SEGMENT_FRAMES: usize = 30 * ENVELOPE_HZ;
const MAX_GLOBAL_CORRELATION_SEGMENTS: usize = 4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceQuality {
    High,
    Medium,
    Low,
    NoReliableTempo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TempoStability {
    MostlyConsistent,
    Varies,
    #[default]
    Uncertain,
}

impl ConfidenceQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "High (heuristic)",
            Self::Medium => "Medium (heuristic)",
            Self::Low => "Low (heuristic)",
            Self::NoReliableTempo => "No reliable tempo",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TempoCandidate {
    pub bpm: f64,
    /// Beat zero as a media-time timestamp, in seconds. The timeline relation
    /// remains media_time = Watch_time + bgmOffset.
    pub phase_seconds: f64,
    /// Normalized onset-envelope periodicity. It is a heuristic score, not a
    /// probability that the tempo is musically correct.
    pub periodicity: f64,
    pub beat_times_seconds: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TempoAnalysis {
    pub schema_version: u32,
    pub detector_version: String,
    pub source_sha256: String,
    pub decoded_duration_seconds: f64,
    pub primary_index: Option<usize>,
    pub candidates: Vec<TempoCandidate>,
    pub quality: ConfidenceQuality,
    /// Peak normalized autocorrelation used for the quality label.
    pub confidence_score: f64,
    #[serde(default)]
    pub tempo_stability: TempoStability,
    #[serde(default)]
    pub source_identity_ms: u64,
    pub decode_ms: u64,
    pub analysis_ms: u64,
    pub cache_lookup_ms: u64,
    #[serde(default)]
    pub from_cache: bool,
}

impl TempoAnalysis {
    pub fn primary(&self) -> Option<&TempoCandidate> {
        self.primary_index
            .and_then(|index| self.candidates.get(index))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisStage {
    Hashing,
    CacheLookup,
    Decoding,
    Estimating,
    Complete,
}

#[derive(Debug, Clone, Copy)]
pub struct AnalysisProgress {
    pub stage: AnalysisStage,
    pub decoded_seconds: f64,
}

/// The UI owns this control. Cancel both sets a flag and kills an active local
/// FFmpeg child so cancellation does not wait for an entire media decode.
#[derive(Clone, Default)]
pub struct AnalysisControl {
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl AnalysisControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            bail!("tempo analysis was cancelled");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AnalysisCacheInfo {
    pub entries: usize,
    pub bytes: u64,
}

pub fn analyze_file(
    path: &Path,
    control: &AnalysisControl,
    mut progress: impl FnMut(AnalysisProgress),
) -> Result<TempoAnalysis> {
    if !path.is_file() {
        bail!("audio source does not exist: {}", path.display());
    }
    control.check()?;
    progress(AnalysisProgress {
        stage: AnalysisStage::Hashing,
        decoded_seconds: 0.0,
    });
    let hash_started = Instant::now();
    let source_sha256 = hash_file_cancelable(path, control)?;
    let source_identity_ms = hash_started.elapsed().as_millis() as u64;
    let cache_lookup_started = Instant::now();
    progress(AnalysisProgress {
        stage: AnalysisStage::CacheLookup,
        decoded_seconds: 0.0,
    });
    let key = cache_key(&source_sha256);
    if let Some(mut cached) = read_cached(&key, &source_sha256) {
        control.check()?;
        cached.source_identity_ms = source_identity_ms;
        cached.decode_ms = 0;
        cached.analysis_ms = 0;
        cached.cache_lookup_ms = cache_lookup_started.elapsed().as_millis() as u64;
        cached.from_cache = true;
        progress(AnalysisProgress {
            stage: AnalysisStage::Complete,
            decoded_seconds: cached.decoded_duration_seconds,
        });
        return Ok(cached);
    }
    let cache_lookup_ms = cache_lookup_started.elapsed().as_millis() as u64;
    control.check()?;
    progress(AnalysisProgress {
        stage: AnalysisStage::Decoding,
        decoded_seconds: 0.0,
    });
    let decode_started = Instant::now();
    let envelope = decode_onset_envelope(path, control, |seconds| {
        progress(AnalysisProgress {
            stage: AnalysisStage::Decoding,
            decoded_seconds: seconds,
        })
    })?;
    let decode_ms = decode_started.elapsed().as_millis() as u64;
    control.check()?;
    progress(AnalysisProgress {
        stage: AnalysisStage::Estimating,
        decoded_seconds: envelope.len() as f64 / ENVELOPE_HZ as f64,
    });
    let analysis_started = Instant::now();
    let mut analysis = analyze_envelope(&envelope, source_sha256.clone())?;
    analysis.decode_ms = decode_ms;
    analysis.source_identity_ms = source_identity_ms;
    analysis.cache_lookup_ms = cache_lookup_ms;
    analysis.analysis_ms = analysis_started.elapsed().as_millis() as u64;
    analysis.from_cache = false;
    control.check()?;
    write_cached(&key, &analysis);
    progress(AnalysisProgress {
        stage: AnalysisStage::Complete,
        decoded_seconds: analysis.decoded_duration_seconds,
    });
    Ok(analysis)
}

fn hash_file_cancelable(path: &Path, control: &AnalysisControl) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening audio {}", path.display()))?;
    let mut hash = crate::content_hash::Sha256::default();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        control.check()?;
        let count = file
            .read(&mut buffer)
            .with_context(|| format!("hashing audio {}", path.display()))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn cache_key(source_sha256: &str) -> String {
    crate::content_hash::sha256_hex(
        format!(
            "{source_sha256}\n{DETECTOR_VERSION}\nrate={DECODE_RATE}\nwindow={FRAME_SAMPLES}\nenvelope_hz={ENVELOPE_HZ}\nbpm={MIN_BPM}-{MAX_BPM}\nrhythm_floor={MIN_RHYTHM_SCORE}"
        )
            .as_bytes(),
    )
}

fn cache_directory() -> PathBuf {
    crate::store_paths::cache_root().join("tempo-analysis-v2")
}

fn read_cached(key: &str, source_sha256: &str) -> Option<TempoAnalysis> {
    read_cached_at(&cache_directory(), key, source_sha256)
}

fn read_cached_at(directory: &Path, key: &str, source_sha256: &str) -> Option<TempoAnalysis> {
    let path = directory.join(format!("{key}.json"));
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 8 * 1024 * 1024
    {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let mut analysis: TempoAnalysis = serde_json::from_slice(&bytes).ok()?;
    if analysis.schema_version != ANALYSIS_SCHEMA_VERSION
        || analysis.detector_version != DETECTOR_VERSION
        || analysis.source_sha256 != source_sha256
        || !valid_analysis(&analysis)
    {
        return None;
    }
    analysis.from_cache = true;
    Some(analysis)
}

fn write_cached(key: &str, analysis: &TempoAnalysis) {
    let directory = cache_directory();
    write_cached_at(&directory, key, analysis);
}

fn write_cached_at(directory: &Path, key: &str, analysis: &TempoAnalysis) {
    if fs::create_dir_all(directory).is_err() {
        return;
    }
    let target = directory.join(format!("{key}.json"));
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            if read_cached_at(directory, key, &analysis.source_sha256).is_some() {
                return;
            }
            if !is_analysis_cache_path(&target) || fs::remove_file(&target).is_err() {
                return;
            }
        }
        Ok(_) => return,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return,
    }
    let Ok(bytes) = serde_json::to_vec(analysis) else {
        return;
    };
    let Ok(mut temp) = tempfile::NamedTempFile::new_in(directory) else {
        return;
    };
    if temp.write_all(&bytes).is_err() || temp.as_file().sync_all().is_err() {
        return;
    }
    // Cache loss is harmless. If replacement is unavailable on this platform,
    // retain the previous result and treat the current result as uncached.
    if temp.persist(&target).is_ok() {
        prune_cache(directory);
    }
}

fn prune_cache(directory: &Path) {
    let mut files = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            if !kind.is_file()
                || kind.is_symlink()
                || !entry.path().extension().is_some_and(|ext| ext == "json")
            {
                return None;
            }
            if !is_analysis_cache_path(&entry.path()) {
                return None;
            }
            Some((entry.path(), entry.metadata().ok()?.modified().ok()?))
        })
        .collect::<Vec<_>>();
    if files.len() <= MAX_CACHE_ENTRIES {
        return;
    }
    files.sort_by_key(|(_, modified)| *modified);
    for (path, _) in files.into_iter().take(MAX_CACHE_ENTRIES / 4) {
        let _ = fs::remove_file(path);
    }
}

pub fn cache_info() -> AnalysisCacheInfo {
    cache_info_at(&cache_directory())
}

fn cache_info_at(directory: &Path) -> AnalysisCacheInfo {
    let mut info = AnalysisCacheInfo::default();
    let Ok(entries) = fs::read_dir(directory) else {
        return info;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && !kind.is_symlink() && is_analysis_cache_path(&entry.path()) {
            info.entries += 1;
            info.bytes = info
                .bytes
                .saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
        }
    }
    info
}

pub fn clear_cache() -> Result<AnalysisCacheInfo> {
    clear_cache_at(&cache_directory())
}

pub fn prune_cache_to(maximum_bytes: u64) -> Result<AnalysisCacheInfo> {
    let directory = cache_directory();
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AnalysisCacheInfo::default())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting cache {}", directory.display()))
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "refusing to prune unsafe tempo-analysis cache path {}",
            directory.display()
        );
    }
    let mut entries = fs::read_dir(&directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || !is_analysis_cache_path(&entry.path())
            {
                return None;
            }
            Some((entry.path(), metadata.len(), metadata.modified().ok()))
        })
        .collect::<Vec<_>>();
    let mut total = entries
        .iter()
        .fold(0u64, |sum, (_, bytes, _)| sum.saturating_add(*bytes));
    let mut removed = AnalysisCacheInfo::default();
    entries.sort_by_key(|(_, _, modified)| *modified);
    for (path, bytes, _) in entries {
        if total <= maximum_bytes {
            break;
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                total = total.saturating_sub(bytes);
                removed.entries += 1;
                removed.bytes = removed.bytes.saturating_add(bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("pruning cache entry {}", path.display()))
            }
        }
    }
    let _ = fs::remove_dir(&directory);
    Ok(removed)
}

fn clear_cache_at(directory: &Path) -> Result<AnalysisCacheInfo> {
    let mut removed = AnalysisCacheInfo::default();
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting cache {}", directory.display()))
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "refusing to clear unsafe tempo-analysis cache path {}",
            directory.display()
        );
    }
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("scanning cache {}", directory.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && is_analysis_cache_path(&path)
        {
            fs::remove_file(&path)
                .with_context(|| format!("removing cache entry {}", path.display()))?;
            removed.entries += 1;
            removed.bytes = removed.bytes.saturating_add(metadata.len());
        }
    }
    let _ = fs::remove_dir(&directory);
    Ok(removed)
}

fn is_analysis_cache_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json")
        && path.file_stem().is_some_and(|stem| {
            let stem = stem.to_string_lossy();
            stem.len() == 64
                && stem
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn valid_analysis(analysis: &TempoAnalysis) -> bool {
    analysis.decoded_duration_seconds.is_finite()
        && analysis.decoded_duration_seconds >= 0.0
        && analysis.confidence_score.is_finite()
        && (0.0..=1.0).contains(&analysis.confidence_score)
        && analysis.candidates.len() <= 5
        && analysis.candidates.iter().all(|candidate| {
            candidate.bpm.is_finite()
                && (MIN_BPM..=MAX_BPM).contains(&candidate.bpm)
                && candidate.phase_seconds.is_finite()
                && candidate.periodicity.is_finite()
                && (0.0..=1.0).contains(&candidate.periodicity)
                && candidate.beat_times_seconds.len() <= 40_000
                && candidate
                    .beat_times_seconds
                    .iter()
                    .all(|time| time.is_finite() && *time >= 0.0)
        })
        && analysis
            .primary_index
            .is_none_or(|index| index < analysis.candidates.len())
}

fn reap_active_decoder(control: &AnalysisControl, terminate: bool) {
    if let Ok(mut slot) = control.child.lock() {
        if let Some(mut child) = slot.take() {
            if terminate {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn decode_onset_envelope(
    path: &Path,
    control: &AnalysisControl,
    mut progress: impl FnMut(f64),
) -> Result<Vec<f32>> {
    let tools = crate::ffmpeg::inspect_existing()?
        .context("tempo analysis needs the application's existing FFmpeg installation")?;
    let mut child = crate::export_control::hide_child_window(&mut Command::new(tools.ffmpeg))
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file,pipe,crypto,data",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "11000",
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting audio decode for {}", path.display()))?;
    let mut stdout = child
        .stdout
        .take()
        .context("FFmpeg tempo decoder has no output pipe")?;
    *control
        .child
        .lock()
        .map_err(|_| anyhow::anyhow!("tempo analysis process lock was poisoned"))? = Some(child);
    let file_size = fs::metadata(path)
        .map(|meta| meta.len())
        .unwrap_or(0)
        .max(1);
    let mut read_buffer = [0u8; 64 * 1024];
    let mut trailing = Vec::<u8>::with_capacity(4);
    let mut window_sum = 0.0f64;
    let mut window_count = 0usize;
    let mut previous_rms = 0.0f64;
    let mut envelope = Vec::new();
    let mut decoded_bytes = 0u64;
    let mut emitted_since_progress = 0usize;
    loop {
        if control.is_cancelled() {
            reap_active_decoder(control, true);
            bail!("tempo analysis was cancelled");
        }
        let count = match stdout.read(&mut read_buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                reap_active_decoder(control, true);
                return Err(error).context("reading decoded audio for tempo analysis");
            }
        };
        if count == 0 {
            break;
        }
        decoded_bytes = decoded_bytes.saturating_add(count as u64);
        let mut bytes = Vec::with_capacity(trailing.len() + count);
        bytes.extend_from_slice(&trailing);
        bytes.extend_from_slice(&read_buffer[..count]);
        let complete = bytes.len() & !3;
        for sample_bytes in bytes[..complete].chunks_exact(4) {
            let sample = f32::from_le_bytes(sample_bytes.try_into().expect("four-byte sample"));
            let sample = if sample.is_finite() {
                f64::from(sample.clamp(-1.0, 1.0))
            } else {
                0.0
            };
            window_sum += sample * sample;
            window_count += 1;
            if window_count == FRAME_SAMPLES {
                let rms = (window_sum / FRAME_SAMPLES as f64).sqrt();
                let onset = (rms - previous_rms * 0.88).max(0.0);
                envelope.push(onset.min(1.0) as f32);
                previous_rms = rms;
                window_sum = 0.0;
                window_count = 0;
                emitted_since_progress += 1;
                if emitted_since_progress >= 500 {
                    progress(envelope.len() as f64 / ENVELOPE_HZ as f64);
                    emitted_since_progress = 0;
                }
            }
        }
        trailing.clear();
        trailing.extend_from_slice(&bytes[complete..]);
        // Output bytes scale approximately linearly with decoded duration for
        // raw PCM; stop untrusted/oversized inputs at four hours.
        if decoded_bytes as f64 / f64::from(DECODE_RATE * 4) > MAX_ANALYSIS_SECONDS {
            reap_active_decoder(control, true);
            bail!("audio is longer than the four-hour tempo-analysis limit");
        }
        let _file_progress = (decoded_bytes as f64 / file_size as f64).clamp(0.0, 1.0);
    }
    let mut slot = control
        .child
        .lock()
        .map_err(|_| anyhow::anyhow!("tempo analysis process lock was poisoned"))?;
    let mut child = slot.take().context("tempo decoder process disappeared")?;
    drop(slot);
    let status = child.wait().context("waiting for FFmpeg tempo decoder")?;
    if !status.success() {
        bail!("FFmpeg could not decode the audio source for tempo analysis");
    }
    if window_count > 0 {
        let rms = (window_sum / window_count as f64).sqrt();
        envelope.push((rms - previous_rms * 0.88).max(0.0).min(1.0) as f32);
    }
    Ok(envelope)
}

/// Deterministic global tempo estimator over a 220 Hz positive RMS-flux
/// envelope. This intentionally reports metrical ambiguity instead of
/// claiming that statistical regularity establishes musical downbeats.
fn analyze_envelope(envelope: &[f32], source_sha256: String) -> Result<TempoAnalysis> {
    let duration = envelope.len() as f64 / ENVELOPE_HZ as f64;
    let empty = |quality, score| TempoAnalysis {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        detector_version: DETECTOR_VERSION.into(),
        source_sha256: source_sha256.clone(),
        decoded_duration_seconds: duration,
        primary_index: None,
        candidates: Vec::new(),
        quality,
        confidence_score: score,
        tempo_stability: TempoStability::Uncertain,
        source_identity_ms: 0,
        decode_ms: 0,
        analysis_ms: 0,
        cache_lookup_ms: 0,
        from_cache: false,
    };
    if envelope.len() < 5 * ENVELOPE_HZ {
        return Ok(empty(ConfidenceQuality::NoReliableTempo, 0.0));
    }
    let mean = envelope.iter().map(|value| f64::from(*value)).sum::<f64>() / envelope.len() as f64;
    let rms = (envelope
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        / envelope.len() as f64)
        .sqrt();
    if !rms.is_finite() || rms < 0.000_02 || mean < 0.000_002 {
        return Ok(empty(ConfidenceQuality::NoReliableTempo, 0.0));
    }
    let centered = envelope
        .iter()
        .map(|value| (f64::from(*value) - mean).max(0.0))
        .collect::<Vec<_>>();
    let min_lag = (60.0 * ENVELOPE_HZ as f64 / MAX_BPM).floor() as usize;
    let max_lag = (60.0 * ENVELOPE_HZ as f64 / MIN_BPM).ceil() as usize;
    let mut correlations = vec![0.0; max_lag + 1];
    for lag in min_lag..=max_lag {
        correlations[lag] = global_autocorrelation(&centered, lag as f64);
    }
    let mut peaks = (min_lag + 1..max_lag)
        .filter(|lag| {
            correlations[*lag] >= correlations[*lag - 1]
                && correlations[*lag] > correlations[*lag + 1]
        })
        .collect::<Vec<_>>();
    peaks.sort_by(|a, b| correlations[*b].total_cmp(&correlations[*a]));
    let Some(best_lag) = peaks.first().copied() else {
        return Ok(empty(ConfidenceQuality::NoReliableTempo, 0.0));
    };
    let denominator =
        correlations[best_lag - 1] - 2.0 * correlations[best_lag] + correlations[best_lag + 1];
    let delta = if denominator.abs() > 1.0e-9 {
        (0.5 * (correlations[best_lag - 1] - correlations[best_lag + 1]) / denominator)
            .clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let best_period = (best_lag as f64 + delta)
        .max(min_lag as f64)
        .min(max_lag as f64);
    let best_score = global_autocorrelation(&centered, best_period);
    if !best_score.is_finite() || best_score < MIN_RHYTHM_SCORE {
        return Ok(empty(
            ConfidenceQuality::NoReliableTempo,
            best_score.max(0.0),
        ));
    }
    let best_bpm = 60.0 * ENVELOPE_HZ as f64 / best_period;
    let mut bpms = vec![best_bpm];
    if best_bpm / 2.0 >= MIN_BPM {
        bpms.push(best_bpm / 2.0);
    }
    if best_bpm * 2.0 <= MAX_BPM {
        bpms.push(best_bpm * 2.0);
    }
    if let Some((onset_bpm, _)) = best_local_tempo(&centered) {
        for candidate in [onset_bpm, onset_bpm * 2.0, onset_bpm / 2.0] {
            if (MIN_BPM..=MAX_BPM).contains(&candidate) {
                bpms.push(candidate);
            }
        }
    }
    bpms.sort_by(|a, b| (a - 120.0).abs().total_cmp(&(b - 120.0).abs()));
    bpms.dedup_by(|a, b| (*a - *b).abs() < 0.001);
    let mut candidates = bpms
        .into_iter()
        .map(|bpm| {
            let period = 60.0 * ENVELOPE_HZ as f64 / bpm;
            let (phase_frames, phase_score) = first_onset_phase(&centered, period);
            let beat_interval = 60.0 / bpm;
            let phase_seconds = phase_frames / ENVELOPE_HZ as f64;
            let mut beat_times_seconds = Vec::new();
            let mut beat_time = phase_seconds;
            while beat_time <= duration && beat_times_seconds.len() < 40_000 {
                beat_times_seconds.push(beat_time);
                beat_time += beat_interval;
            }
            TempoCandidate {
                bpm,
                phase_seconds,
                periodicity: (phase_score.max(0.0).min(1.0) * 0.5
                    + global_autocorrelation(&centered, period).max(0.0).min(1.0) * 0.5),
                beat_times_seconds,
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.periodicity
            .total_cmp(&a.periodicity)
            .then_with(|| (a.bpm - 120.0).abs().total_cmp(&(b.bpm - 120.0).abs()))
    });
    // All candidates share a periodic family. Place the best global-tempo
    // interpretation first and retain explicit octave alternatives.
    let primary_index = candidates
        .iter()
        .position(|candidate| (candidate.bpm - best_bpm).abs() < 0.001)
        .unwrap_or(0);
    let base_score = candidates
        .get(primary_index)
        .map_or(best_score, |candidate| candidate.periodicity);
    let tempo_stability = inspect_local_tempo_stability(&centered);
    let score = if tempo_stability == TempoStability::Varies {
        base_score * 0.55
    } else {
        base_score
    };
    let quality = if score >= 0.68 {
        ConfidenceQuality::High
    } else if score >= 0.42 {
        ConfidenceQuality::Medium
    } else {
        ConfidenceQuality::Low
    };
    let primary = candidates.get(primary_index).cloned();
    let mut ordered = Vec::with_capacity(candidates.len());
    if let Some(primary) = primary {
        ordered.push(primary);
    }
    for (index, candidate) in candidates.into_iter().enumerate() {
        if index != primary_index {
            ordered.push(candidate);
        }
    }
    Ok(TempoAnalysis {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        detector_version: DETECTOR_VERSION.into(),
        source_sha256,
        decoded_duration_seconds: duration,
        primary_index: (!ordered.is_empty()).then_some(0),
        candidates: ordered,
        quality,
        confidence_score: score.clamp(0.0, 1.0),
        tempo_stability,
        source_identity_ms: 0,
        decode_ms: 0,
        analysis_ms: 0,
        cache_lookup_ms: 0,
        from_cache: false,
    })
}

fn autocorrelation(values: &[f64], lag: f64) -> f64 {
    if lag <= 0.0 || lag >= values.len() as f64 {
        return 0.0;
    }
    let count = values.len() - lag.ceil() as usize;
    if count < 8 {
        return 0.0;
    }
    let mut cross = 0.0;
    let mut left_energy = 0.0;
    let mut right_energy = 0.0;
    for index in 0..count {
        let left = values[index];
        let right = interpolate(values, index as f64 + lag);
        cross += left * right;
        left_energy += left * left;
        right_energy += right * right;
    }
    let denominator = (left_energy * right_energy).sqrt();
    if denominator <= 1.0e-20 {
        0.0
    } else {
        (cross / denominator).clamp(0.0, 1.0)
    }
}

/// Keep the quadratic lag search bounded for long recordings. Short songs use
/// the full signal; longer audio samples four evenly spaced 30-second windows
/// so intros, middles, and endings can contribute without work scaling with
/// an hours-long file.
fn global_autocorrelation(values: &[f64], lag: f64) -> f64 {
    let segment_len = GLOBAL_CORRELATION_SEGMENT_FRAMES;
    if values.len() <= segment_len * MAX_GLOBAL_CORRELATION_SEGMENTS {
        return autocorrelation(values, lag);
    }
    let last_start = values.len() - segment_len;
    let last_segment = MAX_GLOBAL_CORRELATION_SEGMENTS - 1;
    (0..MAX_GLOBAL_CORRELATION_SEGMENTS)
        .map(|segment| {
            let start = last_start * segment / last_segment;
            autocorrelation(&values[start..start + segment_len], lag)
        })
        .sum::<f64>()
        / MAX_GLOBAL_CORRELATION_SEGMENTS as f64
}

fn first_onset_phase(values: &[f64], period: f64) -> (f64, f64) {
    let peak = values.iter().copied().fold(0.0_f64, f64::max);
    let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
    let threshold = (peak * 0.22).max(mean * 3.0);
    let phase = (1..values.len().saturating_sub(1))
        .find(|index| {
            values[*index] >= threshold
                && values[*index] >= values[*index - 1]
                && values[*index] > values[*index + 1]
        })
        .map(|index| {
            let left = values[index - 1];
            let center = values[index];
            let right = values[index + 1];
            let denominator = left - 2.0 * center + right;
            let delta = if denominator.abs() > 1.0e-12 {
                (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            index as f64 + delta
        })
        .unwrap_or(0.0);
    let mut sum = 0.0;
    let mut count = 0usize;
    let mut at = phase;
    while at < values.len() as f64 {
        sum += interpolate(values, at);
        count += 1;
        at += period;
    }
    let pulse_score = (sum / count.max(1) as f64 / (mean * 4.0 + 1.0e-12)).clamp(0.0, 1.0);
    (phase, pulse_score)
}

fn inspect_local_tempo_stability(values: &[f64]) -> TempoStability {
    const WINDOW: usize = 15 * ENVELOPE_HZ;
    const HOP: usize = WINDOW / 2;
    if values.len() < WINDOW * 2 {
        return TempoStability::Uncertain;
    }
    let mut local_tempos = Vec::new();
    let mut start = 0usize;
    while start + WINDOW <= values.len() {
        let segment = &values[start..start + WINDOW];
        if let Some((bpm, score)) = best_local_tempo(segment) {
            if score >= 0.35 {
                local_tempos.push(bpm);
            }
        }
        start += HOP;
    }
    if local_tempos.len() < 2 {
        return TempoStability::Uncertain;
    }
    for left in 0..local_tempos.len() {
        for right in left + 1..local_tempos.len() {
            let a = local_tempos[left];
            let b = local_tempos[right];
            let octave_adjusted = [b, b * 2.0, b / 2.0]
                .into_iter()
                .map(|candidate| (candidate - a).abs() / a.max(1.0))
                .fold(f64::INFINITY, f64::min);
            if octave_adjusted > 0.08 {
                return TempoStability::Varies;
            }
        }
    }
    TempoStability::MostlyConsistent
}

fn best_local_tempo(values: &[f64]) -> Option<(f64, f64)> {
    let maximum = values.iter().copied().fold(0.0_f64, f64::max);
    if maximum <= 1.0e-9 {
        return None;
    }
    let threshold = maximum * 0.22;
    let peaks = (1..values.len().saturating_sub(1))
        .filter(|index| {
            values[*index] >= threshold
                && values[*index] >= values[*index - 1]
                && values[*index] > values[*index + 1]
        })
        .collect::<Vec<_>>();
    let intervals = peaks
        .windows(2)
        .map(|pair| (pair[1] - pair[0]) as f64)
        .filter(|interval| {
            (60.0 * ENVELOPE_HZ as f64 / MAX_BPM..=60.0 * ENVELOPE_HZ as f64 / MIN_BPM)
                .contains(interval)
        })
        .collect::<Vec<_>>();
    if intervals.len() < 2 {
        return None;
    }
    let mut sorted = intervals.clone();
    sorted.sort_by(f64::total_cmp);
    let period = median(&sorted)?;
    let mut deviations = intervals
        .iter()
        .map(|interval| (interval - period).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    let normalized_deviation = median(&deviations)? / period.max(1.0);
    let stability = (1.0 - normalized_deviation * 3.0).clamp(0.0, 1.0);
    Some((60.0 * ENVELOPE_HZ as f64 / period, stability))
}

fn interpolate(values: &[f64], position: f64) -> f64 {
    let left = position.floor() as usize;
    if left >= values.len() {
        return 0.0;
    }
    let right = (left + 1).min(values.len() - 1);
    let fraction = position - left as f64;
    values[left] * (1.0 - fraction) + values[right] * fraction
}

/// Convert a mono decoded PCM slice to the same bounded 220 Hz feature stream
/// used by file analysis. Exposed for deterministic semantic tests.
#[cfg(test)]
fn onset_envelope_from_pcm(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if sample_rate == 0 || samples.is_empty() {
        return Vec::new();
    }
    let mut envelope = Vec::new();
    let input_frame = (sample_rate as f64 / ENVELOPE_HZ as f64).round().max(1.0) as usize;
    let mut previous = 0.0f64;
    for frame in samples.chunks(input_frame) {
        let energy = frame
            .iter()
            .map(|sample| {
                let sample = if sample.is_finite() {
                    f64::from(sample.clamp(-1.0, 1.0))
                } else {
                    0.0
                };
                sample * sample
            })
            .sum::<f64>();
        let rms = (energy / frame.len().max(1) as f64).sqrt();
        envelope.push((rms - previous * 0.88).max(0.0) as f32);
        previous = rms;
    }
    envelope
}

#[cfg(test)]
pub fn analyze_samples_for_test(samples: &[f32], sample_rate: u32) -> Result<TempoAnalysis> {
    analyze_envelope(
        &onset_envelope_from_pcm(samples, sample_rate),
        "test-fixture".into(),
    )
}

/// Robust tap tempo estimate. Isolated double/half intervals are removed by
/// the median absolute-deviation fence before the final median is taken.
pub fn tap_tempo_bpm(tap_intervals_seconds: &[f64]) -> Option<f64> {
    let mut intervals = tap_intervals_seconds
        .iter()
        .copied()
        .filter(|value| value.is_finite() && (60.0 / MAX_BPM..=60.0 / MIN_BPM).contains(value))
        .collect::<Vec<_>>();
    if intervals.len() < 2 {
        return None;
    }
    intervals.sort_by(f64::total_cmp);
    let median_interval = median(&intervals)?;
    let mut deviations = intervals
        .iter()
        .map(|value| (value - median_interval).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    let mad = median(&deviations).unwrap_or(0.0);
    let tolerance = (mad * 3.5).max(median_interval * 0.12);
    intervals.retain(|value| (value - median_interval).abs() <= tolerance);
    let interval: f64 = median(&intervals)?;
    Some((60.0 / interval).clamp(MIN_BPM, MAX_BPM))
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) * 0.5
    } else {
        values[middle]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn click_track(bpm: f64, phase: f64, duration: f64, sample_rate: u32) -> Vec<f32> {
        let count = (duration * f64::from(sample_rate)) as usize;
        let mut samples = vec![0.0f32; count];
        let period = 60.0 / bpm;
        let mut beat = phase;
        while beat < duration {
            let start = (beat * f64::from(sample_rate)) as usize;
            let click_len = (0.012 * f64::from(sample_rate)) as usize;
            for index in start..(start + click_len).min(samples.len()) {
                let t = (index - start) as f64 / f64::from(sample_rate);
                let envelope = (-t * 220.0).exp();
                samples[index] = (0.8 * envelope * (TAU * 1300.0 * t).sin()) as f32;
            }
            beat += period;
        }
        samples
    }

    #[test]
    fn synthetic_common_and_fractional_tempos_keep_precision_and_phase() {
        for bpm in [60.0, 90.0, 120.0, 128.0, 150.0, 174.0, 180.0, 128.5, 174.25] {
            let samples = click_track(bpm, 0.137, 80.0, 11_000);
            let result = analyze_samples_for_test(&samples, 11_000).unwrap();
            let detected = result
                .candidates
                .iter()
                .min_by(|left, right| (left.bpm - bpm).abs().total_cmp(&(right.bpm - bpm).abs()))
                .expect("synthetic beat has a candidate");
            assert!(
                (detected.bpm - bpm).abs() <= if bpm == 180.0 { 1.0 } else { 0.7 },
                "wanted {bpm}, got {}; candidates {:?}",
                detected.bpm,
                result
                    .candidates
                    .iter()
                    .map(|candidate| candidate.bpm)
                    .collect::<Vec<_>>()
            );
            assert!(
                (detected.phase_seconds - 0.137).abs() <= 0.04,
                "wanted phase 0.137s, got {}s for {bpm}",
                detected.phase_seconds
            );
            assert!(detected.beat_times_seconds.len() > 8);
        }
    }

    #[test]
    fn tempo_octave_candidates_are_available_without_reanalysis() {
        let result =
            analyze_samples_for_test(&click_track(174.0, 0.23, 50.0, 11_000), 11_000).unwrap();
        let bpms = result
            .candidates
            .iter()
            .map(|candidate| candidate.bpm)
            .collect::<Vec<_>>();
        assert!(bpms.iter().any(|bpm| (*bpm - 87.0).abs() < 1.0), "{bpms:?}");
        assert!(
            bpms.iter().any(|bpm| (*bpm - 174.0).abs() < 1.0),
            "{bpms:?}"
        );
    }

    #[test]
    fn long_audio_uses_bounded_correlation_windows_and_keeps_global_tempo_useful() {
        let result =
            analyze_samples_for_test(&click_track(128.5, 0.217, 180.0, 11_000), 11_000).unwrap();
        let candidate = result
            .candidates
            .iter()
            .min_by(|left, right| {
                (left.bpm - 128.5)
                    .abs()
                    .total_cmp(&(right.bpm - 128.5).abs())
            })
            .unwrap();
        assert!((candidate.bpm - 128.5).abs() < 1.0);
        assert!((candidate.phase_seconds - 0.217).abs() < 0.04);
        assert!(candidate.beat_times_seconds.len() > 300);
    }

    #[test]
    fn silence_short_audio_and_random_noise_never_get_default_120_bpm() {
        let silence = vec![0.0; 20 * 11_000];
        let result = analyze_samples_for_test(&silence, 11_000).unwrap();
        assert_eq!(result.quality, ConfidenceQuality::NoReliableTempo);
        assert!(result.candidates.is_empty());

        let short = analyze_samples_for_test(&vec![0.0; 2 * 11_000], 11_000).unwrap();
        assert_eq!(short.quality, ConfidenceQuality::NoReliableTempo);

        let mut random = 0x1234_5678u32;
        let noise = (0..90 * 11_000)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                ((random as i32) as f32 / i32::MAX as f32) * 0.2
            })
            .collect::<Vec<_>>();
        let result = analyze_samples_for_test(&noise, 11_000).unwrap();
        assert_ne!(result.quality, ConfidenceQuality::High);
        assert!(result.primary().is_none() || result.confidence_score < 0.68);
    }

    #[test]
    fn silence_intro_keeps_audio_time_phase_and_tempo_change_is_not_called_constant_high_confidence(
    ) {
        let mut samples = vec![0.0; 5 * 11_000];
        samples.extend(click_track(120.0, 0.137, 30.0, 11_000));
        let intro = analyze_samples_for_test(&samples, 11_000).unwrap();
        let candidate = intro.primary().unwrap();
        assert!((candidate.bpm - 120.0).abs() < 1.0);
        assert!((candidate.phase_seconds - 5.13).abs() < 0.05);

        let mut changed = click_track(120.0, 0.137, 30.0, 11_000);
        changed.extend(click_track(160.0, 0.137, 30.0, 11_000));
        let variable = analyze_samples_for_test(&changed, 11_000).unwrap();
        assert_eq!(variable.tempo_stability, TempoStability::Varies);
        assert_ne!(variable.quality, ConfidenceQuality::High);
    }

    #[test]
    fn tap_tempo_uses_recent_intervals_and_ignores_single_outlier() {
        let bpm = tap_tempo_bpm(&[0.5, 0.502, 1.0, 0.498, 0.501]).unwrap();
        assert!((bpm - 120.0).abs() < 1.0);
        assert_eq!(tap_tempo_bpm(&[0.5]), None);
    }

    #[test]
    fn project_cache_is_content_and_algorithm_keyed_and_clears_safely() {
        let first = cache_key(&"a".repeat(64));
        assert_ne!(first, cache_key(&"b".repeat(64)));
        assert_eq!(first.len(), 64);

        let directory = tempfile::tempdir().unwrap();
        let source_hash = "a".repeat(64);
        let key = cache_key(&source_hash);
        let analysis =
            analyze_samples_for_test(&click_track(128.5, 0.17, 20.0, 11_000), 11_000).unwrap();
        let analysis = TempoAnalysis {
            source_sha256: source_hash.clone(),
            ..analysis
        };
        write_cached_at(directory.path(), &key, &analysis);
        fs::write(
            directory.path().join(format!("{key}.json")),
            b"corrupt cache",
        )
        .unwrap();
        write_cached_at(directory.path(), &key, &analysis);
        let cached = read_cached_at(directory.path(), &key, &source_hash).unwrap();
        assert!(cached.from_cache);
        assert_eq!(cached.candidates.len(), analysis.candidates.len());
        let original = analysis.primary().unwrap();
        let restored = cached.primary().unwrap();
        assert!((restored.bpm - original.bpm).abs() < 1.0e-9);
        assert!((restored.phase_seconds - original.phase_seconds).abs() < 1.0e-9);
        assert_eq!(
            restored.beat_times_seconds.len(),
            original.beat_times_seconds.len()
        );
        assert_eq!(cache_info_at(directory.path()).entries, 1);

        let foreign = directory.path().join("keep-me.txt");
        fs::write(&foreign, b"not owned by tempo cache").unwrap();
        let cleared = clear_cache_at(directory.path()).unwrap();
        assert_eq!(cleared.entries, 1);
        assert!(
            foreign.exists(),
            "cache cleanup must only remove owned records"
        );
    }

    #[test]
    fn mp3_decode_and_chart_playback_keep_the_same_media_time_alignment() -> Result<()> {
        let Some(tools) = crate::ffmpeg::inspect_existing()? else {
            eprintln!("SKIP: no preinstalled application FFmpeg/FFprobe pair");
            return Ok(());
        };
        let directory = tempfile::tempdir()?;
        let wav = directory.path().join("known-120-bpm.wav");
        let mp3 = directory.path().join("known-120-bpm.mp3");
        let samples = click_track(120.0, 0.137, 50.0, 44_100);
        let pcm = samples
            .into_iter()
            .flat_map(|sample| ((sample * i16::MAX as f32) as i16).to_le_bytes())
            .collect::<Vec<_>>();
        fs::write(&wav, crate::audio::wav_header(44_100, 1, &pcm))?;
        let mut encoder = Command::new(&tools.ffmpeg);
        encoder
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&wav)
            .args(["-codec:a", "libmp3lame", "-q:a", "2"])
            .arg(&mp3);
        let encoded = crate::export_control::ExportControl::default()
            .output(crate::export_control::hide_child_window(&mut encoder))?;
        if !encoded.status.success() {
            eprintln!("SKIP: installed FFmpeg does not provide its MP3 encoder");
            return Ok(());
        }

        let control = AnalysisControl::default();
        let raw_envelope = decode_onset_envelope(&mp3, &control, |_| {})?;
        let raw = analyze_envelope(&raw_envelope, "synthetic-mp3".into())?;
        let playback_project =
            crate::composition::CompositionProject::single_audio_preview_with_existing_tools(&mp3)?;
        let work = directory.path().join("playback-work");
        fs::create_dir_all(&work)?;
        let (program, args) =
            crate::composition::playback_audio_command(&playback_project, DECODE_RATE, &work)?;
        let output = Command::new(program)
            .args(args)
            .stderr(Stdio::null())
            .output()
            .context("decoding MP3 through the production chart playback filter graph")?;
        anyhow::ensure!(
            output.status.success(),
            "production MP3 playback decode failed"
        );
        let playback_pcm = output
            .stdout
            .chunks_exact(8)
            .map(|frame| f32::from_le_bytes(frame[..4].try_into().unwrap()))
            .collect::<Vec<_>>();
        let playback_envelope = onset_envelope_from_pcm(&playback_pcm, DECODE_RATE);
        let playback = analyze_envelope(&playback_envelope, "synthetic-mp3-playback".into())?;
        let raw_candidate = raw
            .candidates
            .iter()
            .min_by(|left, right| {
                (left.bpm - 120.0)
                    .abs()
                    .total_cmp(&(right.bpm - 120.0).abs())
            })
            .context("tempo estimator produced no candidate from encoded MP3")?;
        let playback_candidate = playback
            .candidates
            .iter()
            .min_by(|left, right| {
                (left.bpm - 120.0)
                    .abs()
                    .total_cmp(&(right.bpm - 120.0).abs())
            })
            .context("playback decode produced no tempo candidate")?;
        anyhow::ensure!((raw_candidate.bpm - 120.0).abs() < 1.0);
        anyhow::ensure!((playback_candidate.bpm - 120.0).abs() < 1.0);
        anyhow::ensure!(
            (raw_candidate.phase_seconds - playback_candidate.phase_seconds).abs() < 0.025,
            "analysis/playback phase differed: {} vs {}",
            raw_candidate.phase_seconds,
            playback_candidate.phase_seconds
        );
        for target in [0.137, 25.137, 49.137] {
            let nearest = |candidate: &TempoCandidate| {
                candidate
                    .beat_times_seconds
                    .iter()
                    .min_by(|left, right| {
                        (*left - target).abs().total_cmp(&(*right - target).abs())
                    })
                    .copied()
            };
            let decoded = nearest(raw_candidate).context("missing analysis beat marker")?;
            let heard = nearest(playback_candidate).context("missing playback beat marker")?;
            anyhow::ensure!(
                (decoded - target).abs() < 0.05,
                "MP3 beat at {decoded:.3}s missed expected {target:.3}s"
            );
            anyhow::ensure!(
                (heard - target).abs() < 0.05 && (heard - decoded).abs() < 0.025,
                "playback beat at {heard:.3}s differs from analysis {decoded:.3}s at target {target:.3}s"
            );
        }
        Ok(())
    }
}
