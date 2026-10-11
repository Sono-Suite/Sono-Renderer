//! Explicit microphone capture for Presentation Studio.
//!
//! Capture is an input boundary only. Finalized takes are ordinary PCM WAV
//! files and are imported through `CompositionProject` like any other media.

use crate::media_audio::{PcmWavFormat as WavFormat, PCM_WAV_HEADER_BYTES};
use anyhow::{bail, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{SystemTime, UNIX_EPOCH},
};

const PREFERENCES_SCHEMA: u32 = 1;
const MAX_WAV_DATA_BYTES: u64 = u32::MAX as u64 - PCM_WAV_HEADER_BYTES;
const WRITER_QUEUE_BLOCKS: usize = 64;
const RECOVERY_SCAN_LIMIT: usize = 256;
static NEXT_TAKE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AudioInputPreferences {
    pub schema_version: u32,
    /// CPAL's stable device identifier. `None` selects the system default.
    pub selected_device_id: Option<String>,
}

impl Default for AudioInputPreferences {
    fn default() -> Self {
        Self {
            schema_version: PREFERENCES_SCHEMA,
            selected_device_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}

#[derive(Debug, Clone, Default)]
pub struct InputLevel {
    pub rms: f32,
    pub peak: f32,
    pub clipping: bool,
    pub captured_frames: u64,
    /// Capture-to-callback latency reported by the selected audio backend.
    pub reported_input_latency_ms: Option<f64>,
    pub error: Option<String>,
}

#[derive(Default)]
struct SharedCaptureState {
    rms_bits: AtomicU32,
    peak_bits: AtomicU32,
    clipping: AtomicBool,
    captured_frames: AtomicU64,
    reported_latency_ns: AtomicU64,
    has_latency: AtomicBool,
    accepting_audio: AtomicBool,
    error: Mutex<Option<String>>,
}

impl SharedCaptureState {
    fn set_error(&self, message: String) {
        if let Ok(mut error) = self.error.lock() {
            if error.is_none() {
                *error = Some(message);
            }
        }
    }

    fn level(&self) -> InputLevel {
        let error = self.error.lock().ok().and_then(|error| error.clone());
        let has_latency = self.has_latency.load(Ordering::Relaxed);
        InputLevel {
            rms: f32::from_bits(self.rms_bits.load(Ordering::Relaxed)),
            peak: f32::from_bits(self.peak_bits.load(Ordering::Relaxed)),
            clipping: self.clipping.load(Ordering::Relaxed),
            captured_frames: self.captured_frames.load(Ordering::Relaxed),
            reported_input_latency_ms: has_latency
                .then(|| self.reported_latency_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0),
            error,
        }
    }
}

// Keep the former module path available for callers while generic waveform
// and PCM inspection semantics live in `media_audio`.
pub use crate::media_audio::{inspect_pcm_wav, waveform_for_media, waveform_peaks};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PartialTakeMetadata {
    schema_version: u32,
    timeline_start_seconds: f64,
    sample_rate: u32,
    channels: u16,
}

#[derive(Debug, Clone)]
pub struct RecoveredTake {
    pub partial_path: PathBuf,
    pub timeline_start_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone)]
pub struct RecordedAudio {
    pub path: PathBuf,
    pub timeline_start_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub warning: Option<String>,
}

/// Owns the CPAL stream and, for a recording, its bounded disk-writer worker.
/// Capture starts only after an explicit UI action.
pub struct CaptureSession {
    stream: Option<cpal::Stream>,
    sender: Option<SyncSender<Vec<i16>>>,
    writer: Option<JoinHandle<Result<u64>>>,
    state: Arc<SharedCaptureState>,
    stage_path: Option<PathBuf>,
    final_path: Option<PathBuf>,
    metadata_path: Option<PathBuf>,
    timeline_start_seconds: f64,
    format: WavFormat,
    selected_device_id: String,
    fallback_warning: Option<String>,
}

impl CaptureSession {
    /// Open the selected microphone. With `record_path == None`, this is an
    /// explicit temporary level check and sample buffers are discarded.
    pub fn start(
        device_id: Option<&str>,
        record_path: Option<&Path>,
        timeline_start_seconds: f64,
    ) -> Result<Self> {
        if !timeline_start_seconds.is_finite() || timeline_start_seconds < 0.0 {
            bail!("recording timeline position must be finite and nonnegative");
        }
        let host = cpal::default_host();
        let (device, fallback_warning) = resolve_device(&host, device_id)?;
        let device_id = device
            .id()
            .context("reading microphone device identity")?
            .to_string();
        let supported = device
            .default_input_config()
            .map_err(|error| explain_device_error("opening the microphone input format", error))?;
        let format = WavFormat {
            sample_rate: supported.sample_rate(),
            channels: supported.channels(),
        };
        if format.sample_rate == 0 || format.channels == 0 {
            bail!("the selected microphone returned an invalid sample rate or channel count");
        }

        let state = Arc::new(SharedCaptureState::default());
        let (sender, writer, stage_path, final_path, metadata_path) =
            if let Some(final_path) = record_path {
                let (stage, metadata) = stage_paths(final_path);
                if let Some(parent) = stage.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "creating voiceover recording directory {}",
                            parent.display()
                        )
                    })?;
                }
                let file = create_partial_wav(&stage, &format)?;
                if let Err(error) = write_metadata_atomic(
                    &metadata,
                    &PartialTakeMetadata {
                        schema_version: 1,
                        timeline_start_seconds,
                        sample_rate: format.sample_rate,
                        channels: format.channels,
                    },
                ) {
                    let _ = fs::remove_file(&stage);
                    return Err(error);
                }
                let (sender, receiver) = mpsc::sync_channel(WRITER_QUEUE_BLOCKS);
                state.accepting_audio.store(true, Ordering::Relaxed);
                let writer_state = state.clone();
                let writer_format = format.clone();
                let writer = thread::Builder::new()
                    .name("presentation-audio-writer".into())
                    .spawn(move || write_audio_worker(file, receiver, writer_state, writer_format))
                    .context("starting voiceover WAV writer")?;
                (
                    Some(sender),
                    Some(writer),
                    Some(stage),
                    Some(final_path.to_path_buf()),
                    Some(metadata),
                )
            } else {
                (None, None, None, None, None)
            };

        let stream_config: cpal::StreamConfig = supported.clone().into();
        let stream = match build_capture_stream(
            &device,
            stream_config,
            supported.sample_format(),
            state.clone(),
            sender.clone(),
        ) {
            Ok(stream) => stream,
            Err(error) => {
                discard_empty_stage(
                    sender,
                    writer,
                    stage_path.as_deref(),
                    metadata_path.as_deref(),
                );
                return Err(error).context("building microphone capture stream");
            }
        };
        if let Err(error) = stream.play() {
            let error = explain_device_error("starting microphone capture", error);
            drop(stream);
            discard_empty_stage(
                sender,
                writer,
                stage_path.as_deref(),
                metadata_path.as_deref(),
            );
            return Err(error);
        }

        Ok(Self {
            stream: Some(stream),
            sender,
            writer,
            state,
            stage_path,
            final_path,
            metadata_path,
            timeline_start_seconds,
            format,
            selected_device_id: device_id,
            fallback_warning,
        })
    }

    pub fn level(&self) -> InputLevel {
        self.state.level()
    }

    pub fn device_id(&self) -> &str {
        &self.selected_device_id
    }

    pub fn sample_rate(&self) -> u32 {
        self.format.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.format.channels
    }

    pub fn is_recording(&self) -> bool {
        self.stage_path.is_some()
    }

    pub fn active_partial_path(&self) -> Option<&Path> {
        self.stage_path.as_deref()
    }

    /// Stops capture, drains queued samples, flushes the recoverable WAV, and
    /// atomically promotes a successful take. Meter-only sessions return None.
    pub fn stop(mut self) -> Result<Option<RecordedAudio>> {
        self.stream.take();
        self.sender.take();
        let Some(stage_path) = self.stage_path.take() else {
            return Ok(None);
        };
        let final_path = self
            .final_path
            .take()
            .context("recording destination was lost")?;
        let metadata_path = self
            .metadata_path
            .take()
            .context("recording metadata path was lost")?;
        let frames = match self
            .writer
            .take()
            .context("recording writer was lost")?
            .join()
        {
            Ok(Ok(frames)) => frames,
            Ok(Err(error)) => {
                bail!(
                    "voiceover writer failed; recoverable partial audio remains at {}: {error:#}",
                    stage_path.display()
                );
            }
            Err(_) => bail!(
                "voiceover writer stopped unexpectedly; recoverable partial audio remains at {}",
                stage_path.display()
            ),
        };
        if frames == 0 {
            let _ = fs::remove_file(&stage_path);
            let _ = fs::remove_file(&metadata_path);
            bail!("the microphone stopped before any audio samples arrived");
        }
        let (verified_format, verified_frames) =
            crate::media_audio::read_pcm_wav_format(&stage_path)
                .context("validating the finalized staged WAV before publication")?;
        if verified_format.sample_rate != self.format.sample_rate
            || verified_format.channels != self.format.channels
            || verified_frames != frames
        {
            bail!("finalized WAV metadata does not match captured sample count; partial audio remains at {}", stage_path.display());
        }
        if final_path.exists() {
            bail!("recording output already exists: {}", final_path.display());
        }
        fs::rename(&stage_path, &final_path)
            .with_context(|| format!("finalizing recorded WAV into {}", final_path.display()))?;
        let _ = fs::remove_file(&metadata_path);
        let callback_error = self.state.level().error;
        Ok(Some(RecordedAudio {
            path: final_path,
            timeline_start_seconds: self.timeline_start_seconds,
            sample_rate: self.format.sample_rate,
            channels: self.format.channels,
            frames,
            warning: callback_error.or(self.fallback_warning.take()),
        }))
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stream.take();
        self.sender.take();
        // A deliberate stop joins and finalizes. An unexpected drop leaves the
        // periodically checkpointed `.partial` WAV available for recovery.
    }
}

pub fn enumerate_input_devices() -> Result<Vec<InputDeviceInfo>> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices = host
        .input_devices()
        .map_err(|error| explain_device_error("enumerating microphone inputs", error))?;
    let mut result = Vec::new();
    for device in devices {
        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(_) => continue,
        };
        let name = device
            .description()
            .map(|description| description.to_string())
            .unwrap_or_else(|_| "Microphone input".into());
        let config = device.default_input_config().ok();
        result.push(InputDeviceInfo {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name: name.chars().take(512).collect(),
            sample_rate: config
                .as_ref()
                .map(|value| value.sample_rate())
                .unwrap_or(0),
            channels: config.as_ref().map(|value| value.channels()).unwrap_or(0),
            sample_format: config
                .map(|value| value.sample_format().to_string())
                .unwrap_or_else(|| "unavailable".into()),
        });
    }
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    Ok(result)
}

fn resolve_device(
    host: &cpal::Host,
    requested_id: Option<&str>,
) -> Result<(cpal::Device, Option<String>)> {
    if let Some(requested_id) = requested_id.filter(|id| !id.is_empty()) {
        let parsed = cpal::DeviceId::from_str(requested_id)
            .context("the saved microphone device identity is not valid for this audio backend")?;
        if let Some(device) = host.device_by_id(&parsed) {
            return Ok((device, None));
        }
        let fallback = host.default_input_device().context(
            "the saved microphone is unavailable and there is no system-default input device",
        )?;
        return Ok((fallback, Some("The saved microphone is unavailable. This recording uses the system default input; update the input selection when convenient.".into())));
    }
    host.default_input_device()
        .map(|device| (device, None))
        .context("no system-default microphone input device is available")
}

fn build_capture_stream(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    sample_format: SampleFormat,
    state: Arc<SharedCaptureState>,
    sender: Option<SyncSender<Vec<i16>>>,
) -> Result<cpal::Stream> {
    macro_rules! build {
        ($sample_type:ty) => {
            build_typed_stream::<$sample_type>(device, config, state, sender)
        };
    }
    match sample_format {
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I24 => build!(cpal::I24),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U24 => build!(cpal::U24),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        unsupported => {
            bail!("microphone sample format {unsupported} is not supported for PCM WAV capture")
        }
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    state: Arc<SharedCaptureState>,
    sender: Option<SyncSender<Vec<i16>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample + Sample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let callback_state = state.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], info| {
                let mut pcm = sender.as_ref().map(|_| Vec::with_capacity(data.len()));
                let mut squares = 0.0f64;
                let mut peak = 0.0f32;
                let mut clipped = false;
                for sample in data {
                    let sample = f32::from_sample(*sample);
                    let sample = if sample.is_finite() { sample.clamp(-1.0, 1.0) } else { 0.0 };
                    let magnitude = sample.abs();
                    squares += f64::from(sample) * f64::from(sample);
                    peak = peak.max(magnitude);
                    clipped |= magnitude >= 0.999;
                    if let Some(pcm) = pcm.as_mut() {
                        pcm.push((sample * i16::MAX as f32).round() as i16);
                    }
                }
                let samples = data.len().max(1);
                callback_state.rms_bits.store(((squares / samples as f64).sqrt() as f32).to_bits(), Ordering::Relaxed);
                callback_state.peak_bits.store(peak.to_bits(), Ordering::Relaxed);
                if clipped {
                    callback_state.clipping.store(true, Ordering::Relaxed);
                }
                if channels > 0 {
                    callback_state.captured_frames.fetch_add((data.len() / channels) as u64, Ordering::Relaxed);
                }
                let timestamp = info.timestamp();
                if let Some(latency) = timestamp.callback.checked_duration_since(timestamp.capture) {
                    callback_state.reported_latency_ns.store(latency.as_nanos().min(u128::from(u64::MAX)) as u64, Ordering::Relaxed);
                    callback_state.has_latency.store(true, Ordering::Relaxed);
                }
                if let (Some(sender), Some(pcm)) = (sender.as_ref(), pcm) {
                    if callback_state.accepting_audio.load(Ordering::Relaxed) {
                        match sender.try_send(pcm) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                callback_state.accepting_audio.store(false, Ordering::Relaxed);
                                callback_state.set_error("microphone writer could not keep up; the partial WAV was preserved up to the last queued audio block".into());
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                callback_state.accepting_audio.store(false, Ordering::Relaxed);
                                callback_state.set_error("microphone writer stopped unexpectedly".into());
                            }
                        }
                    }
                }
            },
            move |error| {
                state.set_error(explain_device_error("microphone capture", error).to_string());
                state.accepting_audio.store(false, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|error| explain_device_error("building microphone capture stream", error).into())
}

fn explain_device_error(action: &str, error: cpal::Error) -> anyhow::Error {
    let detail = error.to_string();
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => anyhow::anyhow!("microphone permission denied while {action}. Allow Sono-Renderer to access the microphone in your operating system's privacy settings, then retry. ({detail})"),
        cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::DeviceChanged => anyhow::anyhow!("microphone device became unavailable while {action}. Reconnect it or select another input. ({detail})"),
        _ => anyhow::anyhow!("could not complete {action}: {detail}"),
    }
}

fn write_audio_worker(
    mut file: File,
    receiver: mpsc::Receiver<Vec<i16>>,
    state: Arc<SharedCaptureState>,
    format: WavFormat,
) -> Result<u64> {
    state.accepting_audio.store(true, Ordering::Relaxed);
    let mut data_bytes = 0u64;
    let mut checkpoint_frames = 0u64;
    while let Ok(samples) = receiver.recv() {
        let chunk_bytes = (samples.len() as u64).saturating_mul(2);
        if data_bytes.saturating_add(chunk_bytes) > MAX_WAV_DATA_BYTES {
            state.accepting_audio.store(false, Ordering::Relaxed);
            state.set_error(
                "WAV reached the 4 GiB RIFF size limit; stop and begin another take".into(),
            );
            break;
        }
        let mut bytes = Vec::with_capacity(samples.len().saturating_mul(2));
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        file.write_all(&bytes)
            .context("writing captured PCM samples")?;
        data_bytes += chunk_bytes;
        checkpoint_frames += chunk_bytes / (u64::from(format.channels) * 2);
        if checkpoint_frames >= u64::from(format.sample_rate) * 2 {
            update_wav_header(&mut file, data_bytes, &format)?;
            file.sync_data()
                .context("checkpointing recoverable voiceover audio")?;
            checkpoint_frames = 0;
        }
    }
    update_wav_header(&mut file, data_bytes, &format)?;
    file.sync_all()
        .context("flushing finalized voiceover audio")?;
    Ok(data_bytes / (u64::from(format.channels) * 2))
}

fn stage_paths(final_path: &Path) -> (PathBuf, PathBuf) {
    let mut stage_name = final_path.as_os_str().to_os_string();
    stage_name.push(".partial");
    let mut metadata_name = stage_name.clone();
    metadata_name.push(".json");
    (PathBuf::from(stage_name), PathBuf::from(metadata_name))
}

fn discard_empty_stage(
    sender: Option<SyncSender<Vec<i16>>>,
    writer: Option<JoinHandle<Result<u64>>>,
    stage_path: Option<&Path>,
    metadata_path: Option<&Path>,
) {
    drop(sender);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    if let Some(path) = stage_path {
        let _ = fs::remove_file(path);
    }
    if let Some(path) = metadata_path {
        let _ = fs::remove_file(path);
    }
}

fn create_partial_wav(path: &Path, format: &WavFormat) -> Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating staged voiceover file {}", path.display()))?;
    let mut file = file;
    write_wav_header(&mut file, 0, format)?;
    file.sync_all().context("flushing initial WAV header")?;
    Ok(file)
}

fn update_wav_header(file: &mut File, data_bytes: u64, format: &WavFormat) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    write_wav_header(file, data_bytes, format)?;
    file.seek(SeekFrom::End(0))?;
    Ok(())
}

fn write_wav_header(file: &mut File, data_bytes: u64, format: &WavFormat) -> Result<()> {
    let data_bytes = data_bytes.min(MAX_WAV_DATA_BYTES) as u32;
    let byte_rate = format
        .sample_rate
        .checked_mul(u32::from(format.channels))
        .and_then(|value| value.checked_mul(2))
        .context("WAV byte rate exceeds the PCM format limit")?;
    let block_align = format
        .channels
        .checked_mul(2)
        .context("WAV channel count exceeds PCM limits")?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36u32.saturating_add(data_bytes)).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&format.channels.to_le_bytes())?;
    file.write_all(&format.sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

fn write_metadata_atomic(path: &Path, metadata: &PartialTakeMetadata) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(metadata)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).context("creating voiceover metadata stage")?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .context("atomically publishing voiceover recovery metadata")?;
    Ok(())
}

pub fn recoverable_recordings(directory: &Path) -> Result<Vec<RecoveredTake>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut takes = Vec::new();
    for entry in fs::read_dir(directory).with_context(|| {
        format!(
            "scanning voiceover recovery directory {}",
            directory.display()
        )
    })? {
        if takes.len() >= RECOVERY_SCAN_LIMIT {
            break;
        }
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".wav.partial") || !path.is_file() {
            continue;
        }
        let (format, _frames) = match inspect_partial_wav(&path) {
            Ok(value) if value.1 > 0 => value,
            _ => continue,
        };
        let mut metadata_name = path.as_os_str().to_os_string();
        metadata_name.push(".json");
        let metadata_path = PathBuf::from(metadata_name);
        if !matches!(
            fs::metadata(&metadata_path),
            Ok(metadata) if metadata.is_file() && metadata.len() <= 64 * 1024
        ) {
            continue;
        }
        let metadata: PartialTakeMetadata = match fs::read(&metadata_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PartialTakeMetadata>(&bytes).ok())
        {
            Some(metadata)
                if metadata.schema_version == 1
                    && metadata.timeline_start_seconds.is_finite()
                    && metadata.timeline_start_seconds >= 0.0 =>
            {
                metadata
            }
            _ => continue,
        };
        if format.sample_rate != metadata.sample_rate || format.channels != metadata.channels {
            continue;
        }
        takes.push(RecoveredTake {
            partial_path: path,
            timeline_start_seconds: metadata.timeline_start_seconds,
            sample_rate: format.sample_rate,
            channels: format.channels,
        });
    }
    takes.sort_by(|a, b| a.partial_path.cmp(&b.partial_path));
    Ok(takes)
}

pub fn recover_partial(take: &RecoveredTake) -> Result<RecordedAudio> {
    let (format, frames) = repair_partial_wav(&take.partial_path)?;
    if frames == 0 {
        bail!("partial recording contains no complete audio frames");
    }
    let file_name = take.partial_path.to_string_lossy();
    let final_path = PathBuf::from(
        file_name
            .strip_suffix(".partial")
            .context("partial recording filename has no .partial suffix")?,
    );
    if final_path.exists() {
        bail!(
            "recovered recording destination already exists: {}",
            final_path.display()
        );
    }
    fs::rename(&take.partial_path, &final_path)
        .with_context(|| format!("publishing recovered audio {}", final_path.display()))?;
    let (_, metadata_path) = stage_paths(&final_path);
    let _ = fs::remove_file(metadata_path);
    Ok(RecordedAudio {
        path: final_path,
        timeline_start_seconds: take.timeline_start_seconds,
        sample_rate: format.sample_rate,
        channels: format.channels,
        frames,
        warning: Some(
            "Recovered from an interrupted recording. Check the final boundary before use.".into(),
        ),
    })
}

fn repair_partial_wav(path: &Path) -> Result<(WavFormat, u64)> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < PCM_WAV_HEADER_BYTES {
        bail!("partial recording does not contain a complete WAV header");
    }
    let mut header = [0u8; PCM_WAV_HEADER_BYTES as usize];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        bail!("partial recording has an invalid WAV header");
    }
    let format = WavFormat {
        channels: u16::from_le_bytes([header[22], header[23]]),
        sample_rate: u32::from_le_bytes(header[24..28].try_into().unwrap()),
    };
    if u16::from_le_bytes([header[20], header[21]]) != 1
        || u16::from_le_bytes([header[34], header[35]]) != 16
        || format.channels == 0
        || format.sample_rate == 0
    {
        bail!("partial recording is not a supported PCM WAV");
    }
    let alignment = u64::from(format.channels) * 2;
    let data_bytes = file_len
        .saturating_sub(PCM_WAV_HEADER_BYTES)
        .min(MAX_WAV_DATA_BYTES);
    let data_bytes = data_bytes - data_bytes % alignment;
    file.set_len(PCM_WAV_HEADER_BYTES + data_bytes)?;
    update_wav_header(&mut file, data_bytes, &format)?;
    file.sync_all()?;
    Ok((format, data_bytes / alignment))
}

fn inspect_partial_wav(path: &Path) -> Result<(WavFormat, u64)> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < PCM_WAV_HEADER_BYTES {
        bail!("partial recording does not contain a complete WAV header");
    }
    let mut header = [0u8; PCM_WAV_HEADER_BYTES as usize];
    let mut file = file;
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        bail!("partial recording has an invalid WAV header");
    }
    let format = WavFormat {
        channels: u16::from_le_bytes([header[22], header[23]]),
        sample_rate: u32::from_le_bytes(header[24..28].try_into().unwrap()),
    };
    if u16::from_le_bytes([header[20], header[21]]) != 1
        || u16::from_le_bytes([header[34], header[35]]) != 16
        || format.channels == 0
        || format.sample_rate == 0
    {
        bail!("partial recording is not a supported PCM WAV");
    }
    let alignment = u64::from(format.channels) * 2;
    let data_bytes = file_len
        .saturating_sub(PCM_WAV_HEADER_BYTES)
        .min(MAX_WAV_DATA_BYTES);
    Ok((format, data_bytes / alignment))
}

pub fn recording_directory(project_path: Option<&Path>) -> PathBuf {
    match project_path {
        Some(path) => path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("media")
            .join("recordings"),
        None => crate::store_paths::user_data_root().join("recordings"),
    }
}

pub fn new_take_path(directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(directory)
        .with_context(|| format!("creating recording directory {}", directory.display()))?;
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    let id = NEXT_TAKE_ID.fetch_add(1, Ordering::Relaxed);
    Ok(directory.join(format!("voiceover-{time}-{id}.wav")))
}

pub fn load_preferences() -> Result<AudioInputPreferences> {
    let path = preferences_path();
    if !path.exists() {
        return Ok(AudioInputPreferences::default());
    }
    let metadata = fs::metadata(&path)
        .with_context(|| format!("reading microphone preferences {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        bail!("microphone preferences are not a small regular file");
    }
    let value: AudioInputPreferences = serde_json::from_slice(&fs::read(path)?)?;
    validate_preferences(&value)?;
    Ok(value)
}

pub fn save_preferences(preferences: &AudioInputPreferences) -> Result<()> {
    validate_preferences(preferences)?;
    let path = preferences_path();
    let parent = path
        .parent()
        .context("microphone preferences have no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating settings directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(preferences)?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).context("creating microphone settings stage")?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(&path)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "atomically saving microphone preferences {}",
                path.display()
            )
        })?;
    Ok(())
}

fn validate_preferences(preferences: &AudioInputPreferences) -> Result<()> {
    if preferences.schema_version != PREFERENCES_SCHEMA
        || preferences
            .selected_device_id
            .as_ref()
            .is_some_and(|id| id.len() > 4096)
    {
        bail!("microphone preferences are invalid or use an unsupported schema version");
    }
    Ok(())
}

fn preferences_path() -> PathBuf {
    crate::store_paths::config_root().join("presentation-audio-input.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pcm_writer_checkpoints_and_finalizes_a_recoverable_wav() {
        let dir = tempdir().unwrap();
        let final_path = dir.path().join("take.wav");
        let (stage, metadata) = stage_paths(&final_path);
        let format = WavFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        let file = create_partial_wav(&stage, &format).unwrap();
        let state = Arc::new(SharedCaptureState::default());
        let (tx, rx) = mpsc::sync_channel(4);
        tx.try_send(vec![100, -100, 200, -200]).unwrap();
        drop(tx);
        let frames = write_audio_worker(file, rx, state, format.clone()).unwrap();
        assert_eq!(frames, 4);
        let (read_format, read_frames) = crate::media_audio::read_pcm_wav_format(&stage).unwrap();
        assert_eq!(read_format.sample_rate, 48_000);
        assert_eq!(read_format.channels, 1);
        assert_eq!(read_frames, 4);
        let peaks = waveform_peaks(&stage, 4).unwrap();
        assert_eq!(peaks.len(), 4);
        assert!(peaks[3] > peaks[0]);

        write_metadata_atomic(
            &metadata,
            &PartialTakeMetadata {
                schema_version: 1,
                timeline_start_seconds: 2.5,
                sample_rate: 48_000,
                channels: 1,
            },
        )
        .unwrap();
        let takes = recoverable_recordings(dir.path()).unwrap();
        assert_eq!(takes.len(), 1);
        let recovered = recover_partial(&takes[0]).unwrap();
        assert_eq!(recovered.timeline_start_seconds, 2.5);
        assert_eq!(recovered.frames, 4);
        assert!(recovered.path.is_file());
        assert!(!stage.exists());
    }

    #[test]
    fn incomplete_trailing_sample_is_removed_during_recovery() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("take.wav.partial");
        let format = WavFormat {
            sample_rate: 44_100,
            channels: 2,
        };
        let mut file = create_partial_wav(&path, &format).unwrap();
        file.write_all(&[1, 0, 2]).unwrap();
        file.sync_all().unwrap();
        let (_, frames) = repair_partial_wav(&path).unwrap();
        assert_eq!(frames, 0);
        assert_eq!(fs::metadata(path).unwrap().len(), PCM_WAV_HEADER_BYTES);
    }

    #[test]
    fn finalized_take_becomes_an_ordinary_project_audio_clip() {
        let dir = tempdir().unwrap();
        let media_dir = dir.path().join("media").join("recordings");
        fs::create_dir_all(&media_dir).unwrap();
        let final_path = media_dir.join("voiceover.wav");
        let (stage, metadata) = stage_paths(&final_path);
        let format = WavFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        let file = create_partial_wav(&stage, &format).unwrap();
        let (tx, rx) = mpsc::sync_channel(2);
        tx.try_send(
            (0..4_800)
                .map(|n| if n % 80 < 20 { 8000 } else { 0 })
                .collect(),
        )
        .unwrap();
        drop(tx);
        write_audio_worker(file, rx, Arc::new(SharedCaptureState::default()), format).unwrap();
        fs::rename(&stage, &final_path).unwrap();
        let _ = fs::remove_file(metadata);

        let mut project = crate::composition::CompositionProject::new();
        let source_id = project.add_recorded_source(&final_path).unwrap();
        let clip_id = project
            .add_clip(&source_id, crate::composition::ClipKind::Audio, 3.5)
            .unwrap();
        let clip = project
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .unwrap();
        let source = project
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .unwrap();
        assert_eq!(clip.timeline_start, 3.5);
        assert_eq!(clip.kind, crate::composition::ClipKind::Audio);
        assert!(source.has_audio && !source.has_video);
        assert_eq!(source.duration, 0.1);
        assert_eq!(source.waveform_peaks.len(), 512.min(4_800));

        let project_path = dir.path().join("voiceover.sonocomp");
        project.save(&project_path).unwrap();
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&project_path).unwrap()).unwrap();
        assert!(stored["sources"][0]["path"]
            .as_str()
            .is_some_and(|path| path.starts_with("media/")));
        let reopened = crate::composition::CompositionProject::load(project_path).unwrap();
        assert_eq!(reopened.clips.len(), 1);
        assert!(reopened.sources[0].path.is_file());
        assert!(reopened.sources[0].path.is_absolute());
    }

    #[test]
    fn recovery_detects_recent_audio_even_before_a_two_second_checkpoint() {
        let dir = tempdir().unwrap();
        let final_path = dir.path().join("short-take.wav");
        let (stage, metadata) = stage_paths(&final_path);
        let format = WavFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        let mut file = create_partial_wav(&stage, &format).unwrap();
        file.write_all(&[20, 0, 40, 0]).unwrap();
        file.sync_all().unwrap();
        write_metadata_atomic(
            &metadata,
            &PartialTakeMetadata {
                schema_version: 1,
                timeline_start_seconds: 7.0,
                sample_rate: format.sample_rate,
                channels: format.channels,
            },
        )
        .unwrap();
        let found = recoverable_recordings(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        let recovered = recover_partial(&found[0]).unwrap();
        assert_eq!(recovered.frames, 2);
        assert_eq!(recovered.timeline_start_seconds, 7.0);
        assert_eq!(
            crate::media_audio::read_pcm_wav_format(&recovered.path)
                .unwrap()
                .1,
            2
        );
    }

    #[test]
    fn preferences_keep_stable_device_identity_and_reject_unknown_schema() {
        let preferences = AudioInputPreferences {
            selected_device_id: Some("host/device-stable-id".into()),
            ..Default::default()
        };
        let json = serde_json::to_vec(&preferences).unwrap();
        let decoded: AudioInputPreferences = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded, preferences);
        let invalid = AudioInputPreferences {
            schema_version: PREFERENCES_SCHEMA + 1,
            ..Default::default()
        };
        assert!(serde_json::to_vec(&invalid).is_ok());
        assert!(validate_preferences(&invalid).is_err());
    }

    #[test]
    fn input_enumeration_is_safe_when_the_host_has_no_devices_or_permission() {
        match enumerate_input_devices() {
            Ok(devices) => {
                let mut ids = std::collections::HashSet::new();
                for device in &devices {
                    assert!(!device.id.is_empty());
                    assert!(ids.insert(device.id.as_str()));
                    assert!(device.name.chars().count() <= 512);
                }
                println!("audio input backend enumerated {} device(s)", devices.len());
            }
            Err(error) => {
                println!("audio input enumeration unavailable in this environment: {error:#}")
            }
        }
    }
}
