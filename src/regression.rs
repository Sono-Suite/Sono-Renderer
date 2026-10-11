//! Opt-in, layered regression captures for real renderer output.
//!
//! Ordinary renders do not create regression artifacts. A capture runs the
//! same `RenderConfig::render_frame` pipeline used by preview, records the
//! Watch/display/UI/event state beside lossless RGB checkpoints, and compares
//! those layers independently from optional encoded-media metadata.

use anyhow::{bail, Context, Result};
use image::{ImageFormat, RgbImage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const REGRESSION_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_RUN_RETENTION: usize = 20;
pub const MAX_CHECKPOINTS: usize = 16;
pub const MAX_RESOLUTIONS: usize = 4;
pub const MAX_CAPTURE_FRAMES: usize = 32;
pub const MAX_TIMING_SAMPLES: usize = 5;
pub const MAX_PIXEL_CHANNEL_TOLERANCE: u8 = 2;
const MAX_FRAME_PIXELS: u64 = 8_294_400;
const MAX_CAPTURE_PIXELS: u64 = 64_000_000;
const TIMING_NOISE_ABSOLUTE_MS: f64 = 2.0;
const TIMING_NOISE_RELATIVE: f64 = 0.05;
const SEMANTIC_FLOAT_DECIMAL_PLACES: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionEnvironment {
    pub renderer_version: String,
    pub renderer_commit: Option<String>,
    pub operating_system: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegressionWorkload {
    pub engine_sha1: String,
    pub resources_sha1: String,
    pub level_sha1: String,
    pub music_sha1: Option<String>,
    pub effective_config_sha1: String,
    pub fps: u32,
    pub start_time_micros: i64,
    pub duration_micros: i64,
    /// Selected dimensions, in capture order. One reference can exercise
    /// landscape, portrait, and 4:3 layouts without multiplying fixtures.
    pub resolutions: Vec<[u32; 2]>,
    pub pixel_backend: String,
    pub gameplay_ui_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionReference {
    pub schema_version: u32,
    pub fixture_id: String,
    pub created_unix_time_ms: u128,
    pub environment: RegressionEnvironment,
    pub workload: RegressionWorkload,
    pub checkpoint_times: Vec<f64>,
    pub timing_samples_requested: usize,
    pub modes: Vec<RegressionModeCapture>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionModeCapture {
    pub execution_mode: String,
    pub resolution: [u32; 2],
    pub gpu_adapter: Option<String>,
    pub native_regions_executed: u64,
    pub frames: Vec<RegressionFrameCapture>,
    pub media: Option<RegressionMediaCapture>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionFrameCapture {
    pub checkpoint_id: String,
    pub requested_time: f64,
    pub rendered_timeline_time: Option<f64>,
    pub frame_index: u64,
    pub resolution: [u32; 2],
    pub native_regions_executed: u64,
    /// Callback order, runtime state, transforms, and other stable Watch
    /// observations. Display lists and effect streams are split into their
    /// own comparison layers below.
    pub runtime_semantics: Value,
    pub watch_operation_counts: Value,
    pub display_list: Value,
    pub audio_events: Value,
    pub particle_events: Value,
    pub gameplay_ui_state: Value,
    pub watermark_pose: Option<crate::watermark::WatermarkPose>,
    pub rgb_sha1: String,
    pub png_file: String,
    pub timing_samples: Vec<FrameTimingSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameTimingSample {
    /// `FrameStageProfile::total` for this requested checkpoint. It includes
    /// Watch advancement since the preceding checkpoint (and pre-roll to the
    /// first checkpoint), followed by the real render/compositor work.
    pub checkpoint_step_ms: f64,
    pub watch_ms: f64,
    pub preparation_ms: f64,
    pub backend_ms: f64,
    pub gameplay_ui_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionMediaCapture {
    pub submitted_frames: u64,
    pub fps: u32,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub bgm_offset: f64,
    pub audio_window: Value,
    pub bgm_pcm_sample_frames: Option<u64>,
    pub sfx_event_sha1: String,
    pub sfx_pcm_sha1: Option<String>,
    pub sfx_event_counts: Value,
    pub media_metadata: Value,
}

#[derive(Debug, Clone)]
pub struct RegressionOptions {
    pub fixture_id: String,
    pub checkpoint_times: Vec<f64>,
    pub resolutions: Vec<[u32; 2]>,
    pub compare_vm_gcc: bool,
    pub timing_samples: usize,
    pub include_media: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayerDiff {
    pub name: String,
    pub changed: bool,
    pub expected_sha1: Option<String>,
    pub current_sha1: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PixelDiffStats {
    pub dimensions_match: bool,
    pub width: u32,
    pub height: u32,
    pub compared_pixels: u64,
    pub changed_pixels: u64,
    pub changed_ratio: f64,
    pub max_channel_delta: u8,
    pub mean_absolute_channel_delta: f64,
    pub changed_bounds: Option<[u32; 4]>,
    pub channel_tolerance: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameTimingDiff {
    pub expected_median_ms: Option<f64>,
    pub current_median_ms: Option<f64>,
    pub absolute_difference_ms: Option<f64>,
    pub percentage_difference: Option<f64>,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionFrameComparison {
    pub execution_mode: String,
    pub resolution: [u32; 2],
    pub checkpoint_id: String,
    pub frame_index: u64,
    pub runtime_semantics: LayerDiff,
    pub watch_operation_counts: LayerDiff,
    pub display_list: LayerDiff,
    pub audio_events: LayerDiff,
    pub particle_events: LayerDiff,
    pub gameplay_ui_state: LayerDiff,
    pub watermark_pose: LayerDiff,
    pub pixels: LayerDiff,
    pub pixel_stats: Option<PixelDiffStats>,
    pub timing: FrameTimingDiff,
    pub reference_png: PathBuf,
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionModeComparison {
    pub execution_mode: String,
    pub resolution: [u32; 2],
    pub checkpoint_id: String,
    pub comparable: bool,
    pub reason: Option<String>,
    pub changed_layers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegressionReport {
    pub schema_version: u32,
    pub fixture_id: String,
    pub reference_directory: PathBuf,
    pub current_capture_directory: PathBuf,
    pub comparable: bool,
    pub passed: bool,
    pub workload_differences: Vec<String>,
    pub coverage_differences: Vec<String>,
    pub environment_differences: Vec<String>,
    pub frames: Vec<RegressionFrameComparison>,
    pub media: Vec<LayerDiff>,
    pub vm_vs_gcc: Vec<RegressionModeComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReferenceUpdateReport {
    pub schema_version: u32,
    pub fixture_id: String,
    pub reference_directory: PathBuf,
    pub replaced_existing: bool,
    pub changed_layers: Vec<String>,
    pub previous_reference_error: Option<String>,
    pub captured_modes: usize,
    pub captured_frames: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegressionExit {
    Passed,
    Failed,
    NotComparable,
}

impl RegressionReport {
    pub fn exit_kind(&self) -> RegressionExit {
        if !self.comparable
            || !self.coverage_differences.is_empty()
            || self
                .vm_vs_gcc
                .iter()
                .any(|comparison| !comparison.comparable)
        {
            RegressionExit::NotComparable
        } else if self.passed {
            RegressionExit::Passed
        } else {
            RegressionExit::Failed
        }
    }
}

/// Capture a reference only through the explicit update operation. Replacing
/// a reference requires `replace_existing=true`; the old bundle is moved aside
/// until the staged replacement has been published successfully.
pub fn update_reference(
    config: &crate::render::RenderConfig,
    options: &RegressionOptions,
    reference_directory: impl AsRef<Path>,
    replace_existing: bool,
) -> Result<ReferenceUpdateReport> {
    validate_options(options)?;
    let reference_directory = reference_directory.as_ref();
    let existed = reference_directory.exists();
    if existed && !replace_existing {
        bail!(
            "reference already exists at {}; use the explicit --replace option after reviewing its diff",
            reference_directory.display()
        );
    }
    if existed
        && (!reference_directory.is_dir()
            || !reference_directory
                .join(".sono-regression-reference")
                .is_file())
    {
        bail!(
            "refusing to replace a path that is not a Sono-Renderer regression reference: {}",
            reference_directory.display()
        );
    }
    let parent = reference_directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating reference parent {}", parent.display()))?;
    let stage = tempfile::Builder::new()
        .prefix(".sono-regression-reference-")
        .tempdir_in(parent)
        .context("creating staged regression reference")?;
    let stage_path = stage.path().to_path_buf();
    let reference = capture_reference(config, options, &stage_path)?;
    write_json_atomic(&stage_path.join("reference.json"), &reference)?;
    fs::write(stage_path.join(".sono-regression-reference"), b"schema=1\n")?;

    let mut changed_layers = Vec::new();
    let mut previous_reference_error = None;
    if existed {
        match load_reference(reference_directory) {
            Ok(previous) => {
                changed_layers = summarize_reference_changes(&previous, &reference);
            }
            Err(error) => previous_reference_error = Some(format!("{error:#}")),
        }
        let backup = unique_sibling(reference_directory, "replaced");
        fs::rename(reference_directory, &backup).with_context(|| {
            format!(
                "moving old reference {} to backup {}",
                reference_directory.display(),
                backup.display()
            )
        })?;
        if let Err(error) = fs::rename(&stage_path, reference_directory) {
            let rollback = fs::rename(&backup, reference_directory);
            return Err(error).with_context(|| {
                if rollback.is_ok() {
                    format!("publishing updated reference {}", reference_directory.display())
                } else {
                    format!(
                        "publishing updated reference {} (also failed to restore prior reference at {})",
                        reference_directory.display(),
                        backup.display()
                    )
                }
            });
        }
        fs::remove_dir_all(&backup)
            .with_context(|| format!("removing replaced reference backup {}", backup.display()))?;
    } else {
        fs::rename(&stage_path, reference_directory)
            .with_context(|| format!("publishing reference {}", reference_directory.display()))?;
    }
    let captured_frames = reference.modes.iter().map(|mode| mode.frames.len()).sum();
    Ok(ReferenceUpdateReport {
        schema_version: REGRESSION_SCHEMA_VERSION,
        fixture_id: reference.fixture_id,
        reference_directory: reference_directory.to_path_buf(),
        replaced_existing: existed,
        changed_layers,
        previous_reference_error,
        captured_modes: reference.modes.len(),
        captured_frames,
    })
}

/// Render the checkpoint set through the real renderer and compare against a
/// versioned reference. Current PNGs are kept; diff and heatmap images are
/// written only for failing pixel comparisons.
pub fn run_regression(
    config: &crate::render::RenderConfig,
    reference_directory: impl AsRef<Path>,
    output_directory: Option<&Path>,
    pixel_channel_tolerance: u8,
) -> Result<RegressionReport> {
    if pixel_channel_tolerance > MAX_PIXEL_CHANNEL_TOLERANCE {
        bail!("pixel channel tolerance must be at most {MAX_PIXEL_CHANNEL_TOLERANCE}");
    }
    let reference_directory = reference_directory.as_ref();
    let reference = load_reference(reference_directory)?;
    let options = RegressionOptions {
        fixture_id: reference.fixture_id.clone(),
        checkpoint_times: reference.checkpoint_times.clone(),
        resolutions: reference.workload.resolutions.clone(),
        compare_vm_gcc: reference
            .modes
            .iter()
            .any(|mode| mode.execution_mode == "sono_gcc"),
        timing_samples: reference.timing_samples_requested,
        include_media: reference.modes.iter().any(|mode| mode.media.is_some()),
    };
    validate_options(&options)?;
    let output_directory = match output_directory {
        Some(path) => {
            if path.exists() {
                bail!("regression output already exists: {}", path.display());
            }
            fs::create_dir_all(path)
                .with_context(|| format!("creating output directory {}", path.display()))?;
            path.to_path_buf()
        }
        None => create_default_run_directory()?,
    };
    fs::write(output_directory.join(".sono-regression-run"), b"schema=1\n")?;
    let current_directory = output_directory.join("current");
    fs::create_dir_all(&current_directory)?;
    let current = capture_reference(config, &options, &current_directory)?;
    write_json_atomic(&current_directory.join("capture.json"), &current)?;
    let report = compare_references(
        &reference,
        &current,
        reference_directory,
        &current_directory,
        &output_directory,
        pixel_channel_tolerance,
    )?;
    write_json_atomic(&output_directory.join("comparison.json"), &report)?;
    if output_directory == default_run_directory()
        || output_directory.starts_with(default_run_directory())
    {
        if let Some(root) = output_directory.parent() {
            prune_old_run_directories(root, DEFAULT_RUN_RETENTION)?;
        }
    }
    Ok(report)
}

pub fn load_reference(directory: impl AsRef<Path>) -> Result<RegressionReference> {
    let directory = directory.as_ref();
    if !directory.join(".sono-regression-reference").is_file() {
        bail!(
            "{} is not marked as a Sono-Renderer regression reference",
            directory.display()
        );
    }
    let manifest_path = directory.join("reference.json");
    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("reading regression reference {}", manifest_path.display()))?;
    let reference: RegressionReference = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing regression reference {}", manifest_path.display()))?;
    if reference.schema_version != REGRESSION_SCHEMA_VERSION {
        bail!(
            "unsupported regression reference schema {}; this renderer supports schema {}",
            reference.schema_version,
            REGRESSION_SCHEMA_VERSION
        );
    }
    validate_reference(&reference, directory)?;
    Ok(reference)
}

pub fn save_report_atomic(path: impl AsRef<Path>, report: &RegressionReport) -> Result<()> {
    write_json_atomic(path.as_ref(), report)
}

pub fn default_run_directory() -> PathBuf {
    PathBuf::from("artifacts/regression-lab/runs")
}

fn capture_reference(
    config: &crate::render::RenderConfig,
    options: &RegressionOptions,
    output_directory: &Path,
) -> Result<RegressionReference> {
    validate_options(options)?;
    let base_workload = workload_identity(config, options.resolutions.clone())?;
    let modes = if options.compare_vm_gcc {
        vec![
            crate::sono_gcc::WatchExecutionMode::Interpreter,
            crate::sono_gcc::WatchExecutionMode::SonoGcc,
        ]
    } else {
        vec![config.execution_mode]
    };
    let mut captures = Vec::with_capacity(modes.len() * options.resolutions.len());
    for mode in modes {
        for resolution in &options.resolutions {
            let mut mode_config = config.clone();
            mode_config.execution_mode = mode;
            mode_config.width = resolution[0];
            mode_config.height = resolution[1];
            mode_config.profile = true;
            mode_config.profile_frames = false;
            mode_config.validate_determinism = false;
            mode_config.notifications = crate::notifications::NotificationConfig::default();
            mode_config.job_manager = crate::job_manager::JobManagerConfig::default();
            let variant = format!(
                "{}-{}x{}",
                execution_mode_name(mode),
                resolution[0],
                resolution[1]
            );
            let mode_directory = output_directory.join(&variant);
            fs::create_dir_all(&mode_directory).with_context(|| {
                format!(
                    "creating regression capture directory {}",
                    mode_directory.display()
                )
            })?;
            let mut frame_captures = Vec::with_capacity(options.checkpoint_times.len());
            let mut gpu_adapter = None;
            let mut native_regions_executed = 0_u64;
            let mut frame_timing_samples =
                vec![Vec::with_capacity(options.timing_samples); options.checkpoint_times.len()];
            let mut rendered_frames = std::iter::repeat_with(|| None)
                .take(options.checkpoint_times.len())
                .collect::<Vec<Option<crate::offline::RenderedFrame>>>();
            for sample_index in 0..options.timing_samples {
                let frames = mode_config
                    .render_frames(&options.checkpoint_times)
                    .with_context(|| {
                        format!(
                            "rendering {} checkpoint set (sample {} of {}) in {} mode",
                            options.fixture_id,
                            sample_index + 1,
                            options.timing_samples,
                            execution_mode_name(mode)
                        )
                    })?;
                for (checkpoint_index, frame) in frames.into_iter().enumerate() {
                    let Some(profile) = frame.profile.as_ref() else {
                        bail!("regression capture requested profiling but frame profile is unavailable");
                    };
                    frame_timing_samples[checkpoint_index].push(FrameTimingSample {
                        checkpoint_step_ms: duration_ms(profile.total),
                        watch_ms: duration_ms(profile.vm),
                        preparation_ms: duration_ms(profile.preparation),
                        backend_ms: duration_ms(profile.backend_wall),
                        gameplay_ui_ms: duration_ms(profile.runtime_ui),
                    });
                    gpu_adapter = profile.adapter.clone().or(gpu_adapter);
                    native_regions_executed = frame
                        .report
                        .runtime_profile
                        .sono_gcc_regions_executed
                        .max(native_regions_executed);
                    rendered_frames[checkpoint_index] = Some(frame);
                }
            }
            for (checkpoint_index, requested_time) in
                options.checkpoint_times.iter().copied().enumerate()
            {
                let frame_index = (requested_time * f64::from(mode_config.fps)).ceil() as u64;
                let checkpoint_id = format!("frame-{frame_index:08}");
                let frame = rendered_frames[checkpoint_index]
                    .take()
                    .context("no regression frame was rendered")?;
                let png_name =
                    format!("checkpoint-{checkpoint_index:02}-frame-{frame_index:08}.png");
                // `RenderedFrame` carries tightly packed RGB; dimensions come
                // from the captured mode, not from runtime entity counters.
                let image_path = mode_directory.join(&png_name);
                save_rgb_png(&image_path, resolution[0], resolution[1], &frame.rgb)?;

                let mut report_value = serde_json::to_value(&frame.report)?;
                let report = report_value
                    .as_object_mut()
                    .context("serialized Watch frame report is not an object")?;
                let display_list = report.remove("display_list").unwrap_or(Value::Null);
                let audio_events = json!({
                    "audio_events": report.remove("audio_events").unwrap_or_else(|| json!([])),
                    "scheduled_effects": report.remove("scheduled_effects").unwrap_or_else(|| json!([])),
                    "scheduled_looped_effects": report.remove("scheduled_looped_effects").unwrap_or_else(|| json!([])),
                    "scheduled_looped_effect_stops": report.remove("scheduled_looped_effect_stops").unwrap_or_else(|| json!([])),
                });
                let particle_events = json!({
                    "particle_events": report.remove("particle_events").unwrap_or_else(|| json!([])),
                    "destroyed_particle_effects": report.remove("destroyed_particle_effects").unwrap_or_else(|| json!([])),
                });
                let watch_operation_counts = json!({
                    "function_counts": report.remove("function_counts").unwrap_or_else(|| json!({})),
                    "vm_evaluations": report.remove("vm_evaluations").unwrap_or(Value::Null),
                    "spawn_requests_produced": report.remove("spawn_requests_produced").unwrap_or(Value::Null),
                    "sono_gcc_regions_executed": frame.report.runtime_profile.sono_gcc_regions_executed,
                    "sono_gcc_vm_to_native_cut_calls": frame.report.runtime_profile.sono_gcc_vm_to_native_cut_calls,
                });
                // Generic UI providers can consume scores/events as well as
                // runtime counters. Keep the rendered adapter state visible
                // as a separate semantic layer; no result is inferred here.
                let ui_state = &frame.ui_state;
                let gameplay_ui_state = json!({
                    "time": ui_state.time,
                    "delta_time": ui_state.delta_time,
                    "progress": ui_state.progress,
                    "runtime_entities": ui_state.runtime_entities,
                    "active_entities": ui_state.active_entities,
                    "callbacks": ui_state.callbacks,
                    "draws": ui_state.draws,
                    "scheduled_events": ui_state.scheduled_events,
                    "gameplay": ui_state.gameplay,
                });
                let watermark_pose = crate::watermark::sample_pose(
                    &mode_config.watermark,
                    frame.report.timeline.unwrap_or(requested_time),
                    resolution[0],
                    resolution[1],
                )?;
                frame_captures.push(RegressionFrameCapture {
                    checkpoint_id,
                    requested_time,
                    rendered_timeline_time: frame.report.timeline,
                    frame_index,
                    resolution: *resolution,
                    native_regions_executed: frame.report.runtime_profile.sono_gcc_regions_executed,
                    runtime_semantics: Value::Object(report.clone()),
                    watch_operation_counts,
                    display_list,
                    audio_events,
                    particle_events,
                    gameplay_ui_state,
                    watermark_pose,
                    rgb_sha1: sha1_hex(&frame.rgb),
                    png_file: format!("{variant}/{png_name}"),
                    timing_samples: std::mem::take(&mut frame_timing_samples[checkpoint_index]),
                });
            }
            let media = if options.include_media {
                Some(capture_media(&mode_config).with_context(|| {
                    format!("capturing bounded media/audio metadata for {variant}")
                })?)
            } else {
                None
            };
            captures.push(RegressionModeCapture {
                execution_mode: execution_mode_name(mode).to_owned(),
                resolution: *resolution,
                gpu_adapter,
                native_regions_executed,
                frames: frame_captures,
                media,
            });
        }
    }
    Ok(RegressionReference {
        schema_version: REGRESSION_SCHEMA_VERSION,
        fixture_id: options.fixture_id.clone(),
        created_unix_time_ms: unix_time_ms(),
        environment: environment(),
        workload: base_workload,
        checkpoint_times: options.checkpoint_times.clone(),
        timing_samples_requested: options.timing_samples,
        modes: captures,
    })
}

fn capture_media(config: &crate::render::RenderConfig) -> Result<RegressionMediaCapture> {
    if config.whole_chart {
        bail!("--include-media requires a finite configured duration; whole-chart duration is unbounded");
    }
    if !config.duration.is_finite() || config.duration <= 0.0 || config.duration > 120.0 {
        bail!("--include-media is bounded to finite configured durations of 120 seconds or less");
    }
    let scratch = tempfile::tempdir().context("creating temporary media regression output")?;
    let mut media_config = config.clone();
    media_config.output = Some(scratch.path().join("regression.mp4"));
    media_config.notifications = crate::notifications::NotificationConfig::default();
    let report = media_config.render_video_unrecorded()?;
    Ok(RegressionMediaCapture {
        submitted_frames: report.submitted_frames,
        fps: report.fps,
        timeline_start: report.timeline_start,
        timeline_end: report.timeline_end,
        bgm_offset: report.bgm_offset,
        audio_window: serde_json::to_value(report.audio_window)?,
        bgm_pcm_sample_frames: report.bgm_pcm_sample_frames,
        sfx_event_sha1: report.sfx_event_sha1,
        sfx_pcm_sha1: report.sfx_pcm_sha1,
        sfx_event_counts: serde_json::to_value(report.sfx_event_counts)?,
        media_metadata: stable_media_metadata(&report.probe),
    })
}

fn stable_media_metadata(probe: &Value) -> Value {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|stream| {
            let fields = [
                "codec_type",
                "codec_name",
                "profile",
                "width",
                "height",
                "avg_frame_rate",
                "r_frame_rate",
                "sample_rate",
                "channels",
                "channel_layout",
                "duration",
                "nb_frames",
                "pix_fmt",
            ];
            let mut selected = serde_json::Map::new();
            for field in fields {
                if let Some(value) = stream.get(field) {
                    selected.insert(field.to_owned(), value.clone());
                }
            }
            Value::Object(selected)
        })
        .collect::<Vec<_>>();
    let format = probe.get("format");
    json!({
        "format_name": format.and_then(|value| value.get("format_name")),
        "duration": format.and_then(|value| value.get("duration")),
        "streams": streams,
    })
}

fn save_rgb_png(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    let image = RgbImage::from_raw(width, height, rgb.to_vec())
        .with_context(|| format!("RGB buffer is not {width}x{height} at {}", path.display()))?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png)?;
    write_bytes_atomic(path, bytes.get_ref())
}

fn validate_options(options: &RegressionOptions) -> Result<()> {
    if options.fixture_id.trim().is_empty() || options.fixture_id.len() > 120 {
        bail!("fixture ID must contain 1..=120 non-whitespace characters");
    }
    if options.checkpoint_times.is_empty() || options.checkpoint_times.len() > MAX_CHECKPOINTS {
        bail!("regression requires 1..={MAX_CHECKPOINTS} checkpoint times");
    }
    if options
        .checkpoint_times
        .iter()
        .any(|time| !time.is_finite() || *time < 0.0)
    {
        bail!("checkpoint times must be finite and nonnegative");
    }
    let mut times = options.checkpoint_times.clone();
    times.sort_by(f64::total_cmp);
    if times.windows(2).any(|pair| pair[0] == pair[1]) {
        bail!("checkpoint times must be unique");
    }
    if options.resolutions.is_empty() || options.resolutions.len() > MAX_RESOLUTIONS {
        bail!("regression requires 1..={MAX_RESOLUTIONS} resolutions");
    }
    let mut resolutions = BTreeSet::new();
    for [width, height] in &options.resolutions {
        if *width == 0 || *height == 0 || *width > 8192 || *height > 8192 {
            bail!("regression resolution must be within 1..=8192 on each axis");
        }
        if !resolutions.insert((*width, *height)) {
            bail!("regression resolutions must be unique");
        }
    }
    let modes = if options.compare_vm_gcc { 2 } else { 1 };
    if modes * options.resolutions.len() * options.checkpoint_times.len() > MAX_CAPTURE_FRAMES {
        bail!("regression capture is limited to {MAX_CAPTURE_FRAMES} rendered checkpoints across modes and resolutions");
    }
    let pixels = options
        .resolutions
        .iter()
        .map(|[width, height]| u64::from(*width) * u64::from(*height))
        .sum::<u64>()
        .saturating_mul(options.checkpoint_times.len() as u64)
        .saturating_mul(modes as u64);
    if options
        .resolutions
        .iter()
        .any(|[width, height]| u64::from(*width) * u64::from(*height) > MAX_FRAME_PIXELS)
        || pixels > MAX_CAPTURE_PIXELS
    {
        bail!(
            "regression image budget exceeded ({} pixels per frame; {} total pixels max)",
            MAX_FRAME_PIXELS,
            MAX_CAPTURE_PIXELS
        );
    }
    if !(1..=MAX_TIMING_SAMPLES).contains(&options.timing_samples) {
        bail!("timing sample count must be in 1..={MAX_TIMING_SAMPLES}");
    }
    Ok(())
}

fn workload_identity(
    config: &crate::render::RenderConfig,
    resolutions: Vec<[u32; 2]>,
) -> Result<RegressionWorkload> {
    let mut normalized = serde_json::to_value(config)?;
    let object = normalized
        .as_object_mut()
        .context("serialized render configuration is not an object")?;
    object.insert("engine".into(), Value::String("<engine-content>".into()));
    object.insert(
        "resources".into(),
        Value::String("<resources-content>".into()),
    );
    object.insert("level".into(), Value::String("<level-content>".into()));
    object.insert("music".into(), Value::String("<music-content>".into()));
    object.insert("mv".into(), Value::String("<mv-content>".into()));
    object.insert("output".into(), Value::Null);
    object.insert("width".into(), json!(0));
    object.insert("height".into(), json!(0));
    object.insert(
        "execution_mode".into(),
        json!("normalized-for-backend-pair"),
    );
    object.insert("profile".into(), json!(false));
    object.insert("profile_frames".into(), json!(false));
    object.insert("validate_determinism".into(), json!(false));
    object.insert(
        "notifications".into(),
        serde_json::to_value(crate::notifications::NotificationConfig::default())?,
    );
    object.insert(
        "job_manager".into(),
        serde_json::to_value(crate::job_manager::JobManagerConfig::default())?,
    );
    normalize_config_asset_paths(
        &mut normalized,
        "resource_overrides",
        &["particles", "effects", "background"],
    )?;
    normalize_config_asset_paths(&mut normalized, "ui", &["events_file"])?;
    normalize_config_asset_map(&mut normalized, "ui", "assets")?;
    normalize_config_asset_map(&mut normalized, "watermark", "assets")?;
    let effective_config_sha1 = sha1_hex(&serde_json::to_vec(&normalized)?);
    Ok(RegressionWorkload {
        engine_sha1: fingerprint_path(&config.engine)?,
        resources_sha1: fingerprint_path(&config.resources)?,
        level_sha1: fingerprint_path(&config.level)?,
        music_sha1: config.music.as_deref().map(fingerprint_path).transpose()?,
        effective_config_sha1,
        fps: config.fps,
        start_time_micros: seconds_micros(config.start_time),
        duration_micros: seconds_micros(config.duration),
        resolutions,
        pixel_backend: backend_name(config.backend).to_owned(),
        gameplay_ui_mode: format!("{:?}", config.ui.mode).to_ascii_lowercase(),
    })
}

fn normalize_config_asset_paths(config: &mut Value, section: &str, fields: &[&str]) -> Result<()> {
    let Some(object) = config.get_mut(section).and_then(Value::as_object_mut) else {
        return Ok(());
    };
    for field in fields {
        if let Some(value) = object.get_mut(*field) {
            if let Some(path) = value.as_str().filter(|path| !path.is_empty()) {
                *value = json!(fingerprint_path(Path::new(path))?);
            }
        }
    }
    Ok(())
}

fn normalize_config_asset_map(config: &mut Value, section: &str, field: &str) -> Result<()> {
    let Some(values) = config
        .get_mut(section)
        .and_then(Value::as_object_mut)
        .and_then(|object| object.get_mut(field))
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    for value in values.values_mut() {
        if let Some(path) = value.as_str() {
            *value = json!(fingerprint_path(Path::new(path))?);
        }
    }
    Ok(())
}

fn fingerprint_path(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("reading regression input metadata {}", path.display()))?;
    let mut hash = Sha1::new();
    if metadata.is_file() {
        hash_file_into(path, &mut hash)?;
    } else if metadata.is_dir() {
        let mut files = Vec::new();
        collect_files(path, path, &mut files)?;
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (relative, full) in files {
            hash.update(
                (relative.as_os_str().to_string_lossy().as_bytes())
                    .len()
                    .to_le_bytes(),
            );
            hash.update(relative.as_os_str().to_string_lossy().as_bytes());
            hash_file_into(&full, &mut hash)?;
        }
    } else {
        bail!(
            "regression input is not a file or directory: {}",
            path.display()
        );
    }
    Ok(format!("sha1:{:x}", hash.finalize()))
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    let entries = fs::read_dir(current)
        .with_context(|| format!("reading regression input directory {}", current.display()))?;
    for entry in entries {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            collect_files(root, &path, files)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root)?.to_path_buf();
            files.push((relative, path));
        }
    }
    Ok(())
}

fn hash_file_into(path: &Path, hash: &mut Sha1) -> Result<()> {
    let mut file =
        File::open(path).with_context(|| format!("opening regression input {}", path.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(())
}

fn validate_reference(reference: &RegressionReference, directory: &Path) -> Result<()> {
    if reference.fixture_id.trim().is_empty() {
        bail!("regression reference fixture ID is empty");
    }
    if reference.checkpoint_times.is_empty()
        || reference.checkpoint_times.len() > MAX_CHECKPOINTS
        || reference.modes.is_empty()
        || reference.modes.len() > MAX_RESOLUTIONS * 2
    {
        bail!("regression reference contains no modes or checkpoints");
    }
    if !(1..=MAX_TIMING_SAMPLES).contains(&reference.timing_samples_requested) {
        bail!("regression reference timing sample count is outside supported bounds");
    }
    let unique_times = reference
        .checkpoint_times
        .iter()
        .map(|time| time.to_bits())
        .collect::<BTreeSet<_>>();
    if unique_times.len() != reference.checkpoint_times.len()
        || reference
            .checkpoint_times
            .iter()
            .any(|time| !time.is_finite() || *time < 0.0)
    {
        bail!("regression reference has invalid or duplicate checkpoint times");
    }
    let total_frames = reference
        .modes
        .iter()
        .map(|mode| mode.frames.len())
        .sum::<usize>();
    if total_frames == 0 || total_frames > MAX_CAPTURE_FRAMES {
        bail!("regression reference frame count exceeds the supported limit");
    }
    let mut unique = BTreeSet::new();
    for mode in &reference.modes {
        if !matches!(mode.execution_mode.as_str(), "interpreter" | "sono_gcc")
            || mode.resolution[0] == 0
            || mode.resolution[1] == 0
            || u64::from(mode.resolution[0]) * u64::from(mode.resolution[1]) > MAX_FRAME_PIXELS
        {
            bail!("regression reference contains an unsupported execution mode or resolution");
        }
        if mode.frames.is_empty() {
            bail!(
                "regression mode {:?} contains no frame checkpoints",
                mode.execution_mode
            );
        }
        for frame in &mode.frames {
            if frame.resolution != mode.resolution
                || !(1..=MAX_TIMING_SAMPLES).contains(&frame.timing_samples.len())
                || frame.timing_samples.iter().any(|sample| {
                    !sample.checkpoint_step_ms.is_finite() || sample.checkpoint_step_ms < 0.0
                })
                || frame
                    .rendered_timeline_time
                    .is_some_and(|time| !time.is_finite())
            {
                bail!("regression checkpoint contains invalid dimensions, timeline, or timing samples");
            }
            validate_relative_artifact(&frame.png_file)?;
            let image_path = directory.join(&frame.png_file);
            let decoded = image::open(&image_path)
                .with_context(|| format!("decoding reference image {}", image_path.display()))?
                .to_rgb8();
            if decoded.dimensions() != (frame.resolution[0], frame.resolution[1]) {
                bail!(
                    "reference PNG {} has dimensions {:?}, expected {}x{}",
                    image_path.display(),
                    decoded.dimensions(),
                    frame.resolution[0],
                    frame.resolution[1]
                );
            }
            let raw = decoded.into_raw();
            if sha1_hex(&raw) != frame.rgb_sha1 {
                bail!(
                    "reference PNG {} failed its RGB content hash",
                    image_path.display()
                );
            }
            let key = (
                mode.execution_mode.clone(),
                mode.resolution,
                frame.frame_index,
            );
            if !unique.insert(key) {
                bail!("regression reference contains duplicate mode/resolution/frame keys");
            }
        }
    }
    Ok(())
}

fn validate_relative_artifact(path: &str) -> Result<()> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("regression artifact path is not a safe relative path");
    }
    Ok(())
}

fn environment() -> RegressionEnvironment {
    RegressionEnvironment {
        renderer_version: env!("CARGO_PKG_VERSION").to_owned(),
        renderer_commit: std::env::var("SONO_RENDERER_GIT_COMMIT")
            .ok()
            .filter(|value| !value.trim().is_empty()),
        operating_system: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
    }
}

fn compare_references(
    expected: &RegressionReference,
    current: &RegressionReference,
    reference_directory: &Path,
    current_directory: &Path,
    output_directory: &Path,
    pixel_tolerance: u8,
) -> Result<RegressionReport> {
    let workload_differences = workload_differences(&expected.workload, &current.workload);
    let mut environment_differences = Vec::new();
    if expected.environment.operating_system != current.environment.operating_system {
        environment_differences.push("operating system differs".into());
    }
    if expected.environment.architecture != current.environment.architecture {
        environment_differences.push("architecture differs".into());
    }
    if expected.environment.renderer_version != current.environment.renderer_version {
        environment_differences.push(format!(
            "renderer version {} -> {}",
            expected.environment.renderer_version, current.environment.renderer_version
        ));
    }
    if expected.environment.renderer_commit != current.environment.renderer_commit {
        environment_differences.push("renderer commit differs".into());
    }
    for expected_mode in &expected.modes {
        if let Some(current_mode) = current.modes.iter().find(|mode| {
            mode.execution_mode == expected_mode.execution_mode
                && mode.resolution == expected_mode.resolution
        }) {
            if expected_mode.gpu_adapter != current_mode.gpu_adapter {
                environment_differences.push(format!(
                    "GPU adapter differs for {} {}x{}",
                    expected_mode.execution_mode,
                    expected_mode.resolution[0],
                    expected_mode.resolution[1]
                ));
            }
        }
    }
    let comparable = workload_differences.is_empty();
    let mut frames = Vec::new();
    let mut media = Vec::new();
    let mut coverage_differences = Vec::new();
    let mut passed = comparable;
    if comparable {
        for expected_mode in &expected.modes {
            let Some(current_mode) = current.modes.iter().find(|mode| {
                mode.execution_mode == expected_mode.execution_mode
                    && mode.resolution == expected_mode.resolution
            }) else {
                passed = false;
                coverage_differences.push(format!(
                    "missing execution mode {} at {}x{}",
                    expected_mode.execution_mode,
                    expected_mode.resolution[0],
                    expected_mode.resolution[1]
                ));
                continue;
            };
            for expected_frame in &expected_mode.frames {
                let Some(current_frame) = current_mode.frames.iter().find(|frame| {
                    frame.frame_index == expected_frame.frame_index
                        && frame.checkpoint_id == expected_frame.checkpoint_id
                }) else {
                    passed = false;
                    coverage_differences.push(format!(
                        "missing checkpoint {} for {} at {}x{}",
                        expected_frame.checkpoint_id,
                        expected_mode.execution_mode,
                        expected_mode.resolution[0],
                        expected_mode.resolution[1]
                    ));
                    continue;
                };
                let comparison = compare_frame(
                    expected_mode,
                    expected_frame,
                    current_frame,
                    reference_directory,
                    current_directory,
                    output_directory,
                    pixel_tolerance,
                )?;
                if frame_has_regression(&comparison) {
                    passed = false;
                }
                frames.push(comparison);
            }
            if let (Some(old), Some(new)) = (&expected_mode.media, &current_mode.media) {
                let diffs = compare_media(old, new)?;
                if diffs.iter().any(|diff| diff.changed) {
                    passed = false;
                }
                media.extend(diffs);
            } else if expected_mode.media.is_some() != current_mode.media.is_some() {
                passed = false;
                media.push(LayerDiff {
                    name: "audio_and_media".into(),
                    changed: true,
                    expected_sha1: expected_mode.media.as_ref().map(|_| "present".into()),
                    current_sha1: current_mode.media.as_ref().map(|_| "present".into()),
                    detail: Some("media/audio capture presence differs".into()),
                });
            }
        }
        for current_mode in &current.modes {
            if !expected.modes.iter().any(|mode| {
                mode.execution_mode == current_mode.execution_mode
                    && mode.resolution == current_mode.resolution
            }) {
                passed = false;
                coverage_differences.push(format!(
                    "unexpected execution mode {} at {}x{}",
                    current_mode.execution_mode,
                    current_mode.resolution[0],
                    current_mode.resolution[1]
                ));
            }
        }
    }
    let vm_vs_gcc = compare_vm_gcc(&current, current_directory, output_directory)?;
    if vm_vs_gcc
        .iter()
        .any(|comparison| comparison.comparable && !comparison.changed_layers.is_empty())
    {
        passed = false;
    }
    if vm_vs_gcc.iter().any(|comparison| !comparison.comparable) {
        // Unsupported/fallback GCC is disclosed as unavailable rather than
        // counted as a mismatch or silently treated as a pass.
    }
    Ok(RegressionReport {
        schema_version: REGRESSION_SCHEMA_VERSION,
        fixture_id: expected.fixture_id.clone(),
        reference_directory: reference_directory.to_path_buf(),
        current_capture_directory: current_directory.to_path_buf(),
        comparable,
        passed,
        workload_differences,
        coverage_differences,
        environment_differences,
        frames,
        media,
        vm_vs_gcc,
    })
}

fn compare_media(
    expected: &RegressionMediaCapture,
    current: &RegressionMediaCapture,
) -> Result<Vec<LayerDiff>> {
    Ok(vec![
        compare_json_layer(
            "sfx_event_sequence",
            &json!({
                "sha1": expected.sfx_event_sha1,
                "counts": expected.sfx_event_counts,
            }),
            &json!({
                "sha1": current.sfx_event_sha1,
                "counts": current.sfx_event_counts,
            }),
        ),
        compare_json_layer(
            "sfx_mixed_pcm",
            &json!({"sha1": expected.sfx_pcm_sha1}),
            &json!({"sha1": current.sfx_pcm_sha1}),
        ),
        compare_json_layer(
            "audio_timeline",
            &json!({
                "submitted_frames": expected.submitted_frames,
                "fps": expected.fps,
                "timeline_start": expected.timeline_start,
                "timeline_end": expected.timeline_end,
                "bgm_offset": expected.bgm_offset,
                "audio_window": expected.audio_window,
                "bgm_pcm_sample_frames": expected.bgm_pcm_sample_frames,
            }),
            &json!({
                "submitted_frames": current.submitted_frames,
                "fps": current.fps,
                "timeline_start": current.timeline_start,
                "timeline_end": current.timeline_end,
                "bgm_offset": current.bgm_offset,
                "audio_window": current.audio_window,
                "bgm_pcm_sample_frames": current.bgm_pcm_sample_frames,
            }),
        ),
        compare_json_layer(
            "encoded_media_metadata",
            &expected.media_metadata,
            &current.media_metadata,
        ),
    ])
}

fn compare_frame(
    expected_mode: &RegressionModeCapture,
    expected: &RegressionFrameCapture,
    current: &RegressionFrameCapture,
    reference_directory: &Path,
    current_directory: &Path,
    output_directory: &Path,
    pixel_tolerance: u8,
) -> Result<RegressionFrameComparison> {
    let expected_png = reference_directory.join(&expected.png_file);
    let current_png = current_directory.join(&current.png_file);
    let expected_image = image::open(&expected_png)
        .with_context(|| format!("decoding reference PNG {}", expected_png.display()))?
        .to_rgb8();
    let current_image = image::open(&current_png)
        .with_context(|| format!("decoding current PNG {}", current_png.display()))?
        .to_rgb8();
    let (stats, diff_rgb, heat_rgb) =
        pixel_difference(&expected_image, &current_image, pixel_tolerance);
    let pixels_changed = !stats.dimensions_match || stats.changed_pixels > 0;
    let timing = compare_timing(&expected.timing_samples, &current.timing_samples);
    let mut artifacts = vec![current_png.display().to_string()];
    if pixels_changed {
        let artifact_dir = output_directory.join("diffs");
        fs::create_dir_all(&artifact_dir)?;
        let prefix = format!(
            "{}-{}x{}-{}",
            expected_mode.execution_mode,
            expected_mode.resolution[0],
            expected_mode.resolution[1],
            expected.checkpoint_id
        );
        let diff_path = artifact_dir.join(format!("{prefix}-diff.png"));
        let heat_path = artifact_dir.join(format!("{prefix}-heatmap.png"));
        if let Some(image) = RgbImage::from_raw(stats.width, stats.height, diff_rgb) {
            image.save(&diff_path)?;
        }
        if let Some(image) = RgbImage::from_raw(stats.width, stats.height, heat_rgb) {
            image.save(&heat_path)?;
        }
        artifacts.push(diff_path.display().to_string());
        artifacts.push(heat_path.display().to_string());
    }
    let mut by_name = BTreeMap::new();
    for diff in [
        compare_json_layer(
            "runtime_semantics",
            &expected.runtime_semantics,
            &current.runtime_semantics,
        ),
        compare_json_layer(
            "watch_operation_counts",
            &expected.watch_operation_counts,
            &current.watch_operation_counts,
        ),
        compare_json_layer(
            "display_list",
            &expected.display_list,
            &current.display_list,
        ),
        compare_json_layer(
            "audio_events",
            &expected.audio_events,
            &current.audio_events,
        ),
        compare_json_layer(
            "particle_events",
            &expected.particle_events,
            &current.particle_events,
        ),
        compare_json_layer(
            "gameplay_ui_state",
            &expected.gameplay_ui_state,
            &current.gameplay_ui_state,
        ),
        compare_json_layer(
            "watermark_pose",
            &serde_json::to_value(expected.watermark_pose)?,
            &serde_json::to_value(current.watermark_pose)?,
        ),
    ] {
        by_name.insert(diff.name.clone(), diff);
    }
    let runtime_semantics = by_name.remove("runtime_semantics").unwrap();
    let watch_operation_counts = by_name.remove("watch_operation_counts").unwrap();
    let display_list = by_name.remove("display_list").unwrap();
    let audio_events = by_name.remove("audio_events").unwrap();
    let particle_events = by_name.remove("particle_events").unwrap();
    let gameplay_ui_state = by_name.remove("gameplay_ui_state").unwrap();
    let watermark_pose = by_name.remove("watermark_pose").unwrap();
    Ok(RegressionFrameComparison {
        execution_mode: expected_mode.execution_mode.clone(),
        resolution: expected.resolution,
        checkpoint_id: expected.checkpoint_id.clone(),
        frame_index: expected.frame_index,
        runtime_semantics,
        watch_operation_counts,
        display_list,
        audio_events,
        particle_events,
        gameplay_ui_state,
        watermark_pose,
        pixels: LayerDiff {
            name: "pixels".into(),
            changed: pixels_changed,
            expected_sha1: Some(expected.rgb_sha1.clone()),
            current_sha1: Some(current.rgb_sha1.clone()),
            detail: Some(format!(
                "{} changed pixels / {}; tolerance {}, max channel delta {}, bounds {:?}",
                stats.changed_pixels,
                stats.compared_pixels,
                stats.channel_tolerance,
                stats.max_channel_delta,
                stats.changed_bounds
            )),
        },
        pixel_stats: Some(stats),
        timing,
        reference_png: expected_png,
        artifacts,
    })
}

fn frame_has_regression(frame: &RegressionFrameComparison) -> bool {
    frame.runtime_semantics.changed
        || frame.watch_operation_counts.changed
        || frame.display_list.changed
        || frame.audio_events.changed
        || frame.particle_events.changed
        || frame.gameplay_ui_state.changed
        || frame.watermark_pose.changed
        || frame.pixels.changed
        || frame.timing.classification == "changed"
}

fn compare_json_layer(name: &str, expected: &Value, current: &Value) -> LayerDiff {
    let expected_value = canonicalize_semantic_json(expected);
    let current_value = canonicalize_semantic_json(current);
    let expected_bytes = serde_json::to_vec(&expected_value).unwrap_or_default();
    let current_bytes = serde_json::to_vec(&current_value).unwrap_or_default();
    let changed = expected_bytes != current_bytes;
    let detail = changed.then(|| {
        let differences = json_difference_paths(&expected_value, &current_value, 12);
        if differences.is_empty() {
            "JSON content differs".to_owned()
        } else {
            format!("changed fields: {}", differences.join(", "))
        }
    });
    LayerDiff {
        name: name.to_owned(),
        changed,
        expected_sha1: Some(sha1_hex(&expected_bytes)),
        current_sha1: Some(sha1_hex(&current_bytes)),
        detail,
    }
}

fn json_layer_equal(expected: &Value, current: &Value) -> bool {
    canonicalize_semantic_json(expected) == canonicalize_semantic_json(current)
}

/// JSON semantic layers contain derived `f64` geometry and event times. Their
/// final bits can vary across equivalent Watch evaluation paths even when the
/// rendered RGB and media outputs are identical. Canonicalize floats to 1e-9
/// absolute precision; integers and all nonnumeric values remain exact. Pixel
/// comparison is still independently exact unless the caller opts into the
/// small channel tolerance.
fn canonicalize_semantic_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.iter().map(canonicalize_semantic_json).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize_semantic_json(value)))
                .collect(),
        ),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Value::Number(number.clone())
        }
        Value::Number(number) => {
            let Some(value) = number.as_f64().filter(|value| value.is_finite()) else {
                return Value::Number(number.clone());
            };
            let rounded =
                (value * SEMANTIC_FLOAT_DECIMAL_PLACES).round() / SEMANTIC_FLOAT_DECIMAL_PLACES;
            if rounded == 0.0 {
                return Value::Number(serde_json::Number::from(0));
            }
            if rounded.fract() == 0.0
                && rounded >= -9_223_372_036_854_775_808.0
                && rounded < 9_223_372_036_854_775_808.0
            {
                return Value::Number(serde_json::Number::from(rounded as i64));
            }
            if rounded.fract() == 0.0 && rounded >= 0.0 && rounded < 18_446_744_073_709_551_616.0 {
                return Value::Number(serde_json::Number::from(rounded as u64));
            }
            serde_json::Number::from_f64(rounded)
                .map(Value::Number)
                .unwrap_or_else(|| Value::Number(number.clone()))
        }
        _ => value.clone(),
    }
}

fn json_difference_paths(expected: &Value, current: &Value, limit: usize) -> Vec<String> {
    fn visit(expected: &Value, current: &Value, path: &str, left: &mut Vec<String>, limit: usize) {
        if left.len() >= limit || expected == current {
            return;
        }
        match (expected, current) {
            (Value::Object(a), Value::Object(b)) => {
                let keys = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
                for key in keys {
                    let child = if path.is_empty() {
                        key.as_str().to_owned()
                    } else {
                        format!("{path}.{key}")
                    };
                    match (a.get(key), b.get(key)) {
                        (Some(x), Some(y)) => visit(x, y, &child, left, limit),
                        _ => left.push(child),
                    }
                    if left.len() >= limit {
                        break;
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    left.push(format!("{path}.length"));
                }
                for index in 0..a.len().min(b.len()) {
                    visit(
                        &a[index],
                        &b[index],
                        &format!("{path}[{index}]"),
                        left,
                        limit,
                    );
                    if left.len() >= limit {
                        break;
                    }
                }
            }
            _ => left.push(if path.is_empty() {
                "<root>".into()
            } else {
                path.into()
            }),
        }
    }
    let mut output = Vec::new();
    visit(expected, current, "", &mut output, limit);
    output
}

fn pixel_difference(
    expected: &RgbImage,
    current: &RgbImage,
    tolerance: u8,
) -> (PixelDiffStats, Vec<u8>, Vec<u8>) {
    let (width, height) = current.dimensions();
    let dimensions_match = expected.dimensions() == current.dimensions();
    let count = width as usize * height as usize;
    let mut diff = vec![0_u8; count * 3];
    let mut heat = vec![0_u8; count * 3];
    let mut changed = 0_u64;
    let mut total_delta = 0_u64;
    let mut max_delta = 0_u8;
    let mut bounds: Option<[u32; 4]> = None;
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 3) as usize;
            let actual = current.get_pixel(x, y).0;
            let reference = if dimensions_match {
                expected.get_pixel(x, y).0
            } else {
                [0, 0, 0]
            };
            let mut pixel_changed = !dimensions_match;
            let mut max_pixel = 0_u8;
            for channel in 0..3 {
                let delta = actual[channel].abs_diff(reference[channel]);
                total_delta += u64::from(delta);
                max_delta = max_delta.max(delta);
                max_pixel = max_pixel.max(delta);
                pixel_changed |= delta > tolerance;
                diff[index + channel] = delta;
            }
            if pixel_changed {
                changed += 1;
                bounds = Some(match bounds {
                    Some([left, top, right, bottom]) => {
                        [left.min(x), top.min(y), right.max(x), bottom.max(y)]
                    }
                    None => [x, y, x, y],
                });
            }
            heat[index] = max_pixel;
            heat[index + 1] = max_pixel.saturating_mul(2);
            heat[index + 2] = max_pixel.saturating_mul(4);
        }
    }
    let compared_pixels = u64::from(width) * u64::from(height);
    let stats = PixelDiffStats {
        dimensions_match,
        width,
        height,
        compared_pixels,
        changed_pixels: changed,
        changed_ratio: changed as f64 / compared_pixels.max(1) as f64,
        max_channel_delta: max_delta,
        mean_absolute_channel_delta: total_delta as f64 / (compared_pixels.max(1) * 3) as f64,
        changed_bounds: bounds,
        channel_tolerance: tolerance,
    };
    (stats, diff, heat)
}

fn compare_timing(
    expected: &[FrameTimingSample],
    current: &[FrameTimingSample],
) -> FrameTimingDiff {
    let old = median(
        expected
            .iter()
            .map(|sample| sample.checkpoint_step_ms)
            .collect(),
    );
    let new = median(
        current
            .iter()
            .map(|sample| sample.checkpoint_step_ms)
            .collect(),
    );
    let Some((old_value, new_value)) = old.zip(new) else {
        return FrameTimingDiff {
            expected_median_ms: old,
            current_median_ms: new,
            absolute_difference_ms: None,
            percentage_difference: None,
            classification: "unavailable".into(),
        };
    };
    let delta = new_value - old_value;
    let absolute = delta.abs();
    let percentage = (old_value > 0.0).then_some(delta / old_value * 100.0);
    let changed = absolute > TIMING_NOISE_ABSOLUTE_MS
        && percentage.is_some_and(|value| value.abs() >= TIMING_NOISE_RELATIVE * 100.0);
    FrameTimingDiff {
        expected_median_ms: Some(old_value),
        current_median_ms: Some(new_value),
        absolute_difference_ms: Some(delta),
        percentage_difference: percentage,
        classification: if changed { "changed" } else { "within_noise" }.into(),
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(if values.len() % 2 == 0 {
        (values[values.len() / 2 - 1] + values[values.len() / 2]) * 0.5
    } else {
        values[values.len() / 2]
    })
}

fn compare_vm_gcc(
    current: &RegressionReference,
    capture_directory: &Path,
    output_directory: &Path,
) -> Result<Vec<RegressionModeComparison>> {
    let Some(vm) = current
        .modes
        .iter()
        .find(|mode| mode.execution_mode == "interpreter")
    else {
        return Ok(Vec::new());
    };
    let Some(gcc) = current
        .modes
        .iter()
        .find(|mode| mode.execution_mode == "sono_gcc")
    else {
        return Ok(Vec::new());
    };
    let mut comparisons = Vec::new();
    for vm_frame in &vm.frames {
        let Some(gcc_frame) = gcc.frames.iter().find(|frame| {
            frame.frame_index == vm_frame.frame_index && frame.resolution == vm_frame.resolution
        }) else {
            continue;
        };
        let reason = if gcc_frame.native_regions_executed == 0 {
            Some("Sono-GCC did not execute a native region in this workload; parity is unavailable, not passed".to_owned())
        } else {
            None
        };
        let mut changed_layers = Vec::new();
        if reason.is_none() {
            for (name, old, new) in [
                (
                    "runtime_semantics",
                    &vm_frame.runtime_semantics,
                    &gcc_frame.runtime_semantics,
                ),
                (
                    "display_list",
                    &vm_frame.display_list,
                    &gcc_frame.display_list,
                ),
                (
                    "audio_events",
                    &vm_frame.audio_events,
                    &gcc_frame.audio_events,
                ),
                (
                    "particle_events",
                    &vm_frame.particle_events,
                    &gcc_frame.particle_events,
                ),
                (
                    "gameplay_ui_state",
                    &vm_frame.gameplay_ui_state,
                    &gcc_frame.gameplay_ui_state,
                ),
                (
                    "watermark_pose",
                    &serde_json::to_value(vm_frame.watermark_pose)?,
                    &serde_json::to_value(gcc_frame.watermark_pose)?,
                ),
            ] {
                if old != new {
                    changed_layers.push(name.to_owned());
                }
            }
            let vm_image = image::open(capture_directory.join(&vm_frame.png_file))?.to_rgb8();
            let gcc_image = image::open(capture_directory.join(&gcc_frame.png_file))?.to_rgb8();
            if pixel_difference(&vm_image, &gcc_image, 0).0.changed_pixels != 0 {
                changed_layers.push("pixels".into());
                if let Some(parent) = output_directory.parent() {
                    let _ = parent;
                }
            }
        }
        comparisons.push(RegressionModeComparison {
            execution_mode: "sono_vm_vs_sono_gcc".into(),
            resolution: vm_frame.resolution,
            checkpoint_id: vm_frame.checkpoint_id.clone(),
            comparable: reason.is_none(),
            reason,
            changed_layers,
        });
    }
    Ok(comparisons)
}

fn workload_differences(
    expected: &RegressionWorkload,
    current: &RegressionWorkload,
) -> Vec<String> {
    let mut differences = Vec::new();
    macro_rules! compare {
        ($field:ident, $label:literal) => {
            if expected.$field != current.$field {
                differences.push($label.into());
            }
        };
    }
    compare!(engine_sha1, "engine content");
    compare!(resources_sha1, "resource content");
    compare!(level_sha1, "level content");
    compare!(music_sha1, "music content");
    compare!(
        effective_config_sha1,
        "effective render/UI/watermark configuration"
    );
    compare!(fps, "FPS");
    compare!(start_time_micros, "start time");
    compare!(duration_micros, "duration");
    compare!(resolutions, "resolution set");
    compare!(pixel_backend, "pixel backend");
    compare!(gameplay_ui_mode, "gameplay UI mode");
    differences
}

fn summarize_reference_changes(
    old: &RegressionReference,
    new: &RegressionReference,
) -> Vec<String> {
    let mut changed = BTreeSet::new();
    if old.checkpoint_times != new.checkpoint_times {
        changed.insert("checkpoint schedule changed".into());
    }
    if old.timing_samples_requested != new.timing_samples_requested {
        changed.insert(format!(
            "timing sample count changed: {} -> {}",
            old.timing_samples_requested, new.timing_samples_requested
        ));
    }
    if old.environment != new.environment {
        changed.insert("capture environment/build metadata changed".into());
    }
    for new_mode in &new.modes {
        let old_mode = old.modes.iter().find(|mode| {
            mode.execution_mode == new_mode.execution_mode && mode.resolution == new_mode.resolution
        });
        let Some(old_mode) = old_mode else {
            changed.insert(format!(
                "added {} at {}x{}",
                new_mode.execution_mode, new_mode.resolution[0], new_mode.resolution[1]
            ));
            continue;
        };
        for new_frame in &new_mode.frames {
            let old_frame = old_mode
                .frames
                .iter()
                .find(|frame| frame.frame_index == new_frame.frame_index);
            let Some(old_frame) = old_frame else {
                changed.insert(format!(
                    "added {} checkpoint {}",
                    new_mode.execution_mode, new_frame.checkpoint_id
                ));
                continue;
            };
            for (name, a, b) in [
                (
                    "runtime_semantics",
                    &old_frame.runtime_semantics,
                    &new_frame.runtime_semantics,
                ),
                (
                    "watch_operation_counts",
                    &old_frame.watch_operation_counts,
                    &new_frame.watch_operation_counts,
                ),
                (
                    "display_list",
                    &old_frame.display_list,
                    &new_frame.display_list,
                ),
                (
                    "audio_events",
                    &old_frame.audio_events,
                    &new_frame.audio_events,
                ),
                (
                    "particle_events",
                    &old_frame.particle_events,
                    &new_frame.particle_events,
                ),
                (
                    "gameplay_ui_state",
                    &old_frame.gameplay_ui_state,
                    &new_frame.gameplay_ui_state,
                ),
                (
                    "watermark_pose",
                    &serde_json::to_value(old_frame.watermark_pose).unwrap_or(Value::Null),
                    &serde_json::to_value(new_frame.watermark_pose).unwrap_or(Value::Null),
                ),
            ] {
                if !json_layer_equal(a, b) {
                    changed.insert(format!(
                        "{} {} {} changed",
                        new_mode.execution_mode, new_frame.checkpoint_id, name
                    ));
                }
            }
            if old_frame.rgb_sha1 != new_frame.rgb_sha1 {
                changed.insert(format!(
                    "{} {} pixels changed",
                    new_mode.execution_mode, new_frame.checkpoint_id
                ));
            }
            if compare_timing(&old_frame.timing_samples, &new_frame.timing_samples).classification
                == "changed"
            {
                changed.insert(format!(
                    "{} {} frame timing changed beyond noise policy",
                    new_mode.execution_mode, new_frame.checkpoint_id
                ));
            }
        }
        if old_mode.media != new_mode.media {
            changed.insert(format!(
                "{} media/audio metadata changed",
                new_mode.execution_mode
            ));
        }
    }
    for old_mode in &old.modes {
        if !new.modes.iter().any(|mode| {
            mode.execution_mode == old_mode.execution_mode && mode.resolution == old_mode.resolution
        }) {
            changed.insert(format!(
                "removed {} at {}x{}",
                old_mode.execution_mode, old_mode.resolution[0], old_mode.resolution[1]
            ));
            continue;
        }
        let new_mode = new.modes.iter().find(|mode| {
            mode.execution_mode == old_mode.execution_mode && mode.resolution == old_mode.resolution
        });
        if let Some(new_mode) = new_mode {
            for old_frame in &old_mode.frames {
                if !new_mode
                    .frames
                    .iter()
                    .any(|frame| frame.frame_index == old_frame.frame_index)
                {
                    changed.insert(format!(
                        "removed {} checkpoint {}",
                        old_mode.execution_mode, old_frame.checkpoint_id
                    ));
                }
            }
        }
    }
    for difference in workload_differences(&old.workload, &new.workload) {
        changed.insert(format!("workload changed: {difference}"));
    }
    if changed.is_empty() {
        changed.insert("no layer differences observed".into());
    }
    changed.into_iter().collect()
}

fn create_default_run_directory() -> Result<PathBuf> {
    let root = default_run_directory();
    fs::create_dir_all(&root)
        .with_context(|| format!("creating regression run root {}", root.display()))?;
    let path = unique_sibling(&root.join("run"), "capture");
    fs::create_dir(&path).with_context(|| format!("creating regression run {}", path.display()))?;
    prune_old_run_directories(&root, DEFAULT_RUN_RETENTION)?;
    Ok(path)
}

fn prune_old_run_directories(root: &Path, retain: usize) -> Result<()> {
    let mut runs = fs::read_dir(root)?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_dir()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("run-capture-")
                && path.join(".sono-regression-run").is_file())
            .then(|| {
                (
                    entry.metadata().ok().and_then(|meta| meta.modified().ok()),
                    path,
                )
            })
        })
        .collect::<Vec<_>>();
    runs.sort_by_key(|(modified, _)| *modified);
    let remove_count = runs.len().saturating_sub(retain);
    for (_, path) in runs.into_iter().take(remove_count) {
        fs::remove_dir_all(&path)
            .with_context(|| format!("pruning regression run {}", path.display()))?;
    }
    Ok(())
}

fn unique_sibling(path: &Path, label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let counter = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("regression");
    parent.join(format!("{name}-{label}-{}-{counter}", std::process::id()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_bytes_atomic(path, &bytes)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::Builder::new()
        .prefix(".regression-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("publishing regression artifact {}", path.display()))?;
    Ok(())
}

fn sha1_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}
fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn seconds_micros(seconds: f64) -> i64 {
    (seconds * 1_000_000.0).round() as i64
}
fn duration_ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
fn execution_mode_name(mode: crate::sono_gcc::WatchExecutionMode) -> &'static str {
    match mode {
        crate::sono_gcc::WatchExecutionMode::Interpreter => "interpreter",
        crate::sono_gcc::WatchExecutionMode::SonoGcc => "sono_gcc",
    }
}
fn backend_name(backend: crate::render::RenderBackend) -> &'static str {
    match backend {
        crate::render::RenderBackend::Cpu => "cpu",
        crate::render::RenderBackend::Wgpu => "wgpu",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn timing(value: f64) -> FrameTimingSample {
        FrameTimingSample {
            checkpoint_step_ms: value,
            watch_ms: value * 0.5,
            preparation_ms: value * 0.1,
            backend_ms: value * 0.3,
            gameplay_ui_ms: value * 0.1,
        }
    }

    fn sample_reference(directory: &Path, schema_version: u32) -> RegressionReference {
        let image = RgbImage::from_raw(1, 1, vec![10, 20, 30]).unwrap();
        image.save(directory.join("vm-checkpoint.png")).unwrap();
        RegressionReference {
            schema_version,
            fixture_id: "unit-fixture".into(),
            created_unix_time_ms: 1,
            environment: RegressionEnvironment {
                renderer_version: "test".into(),
                renderer_commit: None,
                operating_system: "test-os".into(),
                architecture: "test-arch".into(),
            },
            workload: RegressionWorkload {
                engine_sha1: "engine".into(),
                resources_sha1: "resources".into(),
                level_sha1: "level".into(),
                music_sha1: None,
                effective_config_sha1: "config".into(),
                fps: 60,
                start_time_micros: 0,
                duration_micros: 1_000_000,
                resolutions: vec![[1, 1]],
                pixel_backend: "cpu".into(),
                gameplay_ui_mode: "custom".into(),
            },
            checkpoint_times: vec![0.0],
            timing_samples_requested: 1,
            modes: vec![RegressionModeCapture {
                execution_mode: "interpreter".into(),
                resolution: [1, 1],
                gpu_adapter: None,
                native_regions_executed: 0,
                frames: vec![RegressionFrameCapture {
                    checkpoint_id: "frame-00000000".into(),
                    requested_time: 0.0,
                    rendered_timeline_time: Some(0.0),
                    frame_index: 0,
                    resolution: [1, 1],
                    native_regions_executed: 0,
                    runtime_semantics: json!({"callbacks": []}),
                    watch_operation_counts: json!({"draw": 1}),
                    display_list: json!({"sprites": []}),
                    audio_events: json!({"audio_events": []}),
                    particle_events: json!({"particle_events": []}),
                    gameplay_ui_state: json!({"combo": 0}),
                    watermark_pose: None,
                    rgb_sha1: sha1_hex(&[10, 20, 30]),
                    png_file: "interpreter-1x1/vm-checkpoint.png".into(),
                    timing_samples: vec![timing(10.0)],
                }],
                media: None,
            }],
        }
    }

    fn save_sample_reference(directory: &Path, reference: &RegressionReference) {
        fs::create_dir_all(directory.join("interpreter-1x1")).unwrap();
        let source = directory.join("vm-checkpoint.png");
        let destination = directory.join(&reference.modes[0].frames[0].png_file);
        fs::rename(source, destination).unwrap();
        fs::write(directory.join(".sono-regression-reference"), b"schema=1\n").unwrap();
        fs::write(
            directory.join("reference.json"),
            serde_json::to_vec_pretty(reference).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn schema_round_trip_and_version_gate_are_explicit() {
        let dir = tempdir().unwrap();
        let reference = sample_reference(dir.path(), REGRESSION_SCHEMA_VERSION);
        save_sample_reference(dir.path(), &reference);
        let loaded = load_reference(dir.path()).unwrap();
        assert_eq!(loaded, reference);

        let unsupported = RegressionReference {
            schema_version: REGRESSION_SCHEMA_VERSION + 1,
            ..reference
        };
        fs::write(
            dir.path().join("reference.json"),
            serde_json::to_vec(&unsupported).unwrap(),
        )
        .unwrap();
        assert!(load_reference(dir.path())
            .unwrap_err()
            .to_string()
            .contains("unsupported regression reference schema"));
    }

    #[test]
    fn missing_corrupt_and_hash_mismatched_references_fail_closed() {
        let dir = tempdir().unwrap();
        assert!(load_reference(dir.path()).is_err());
        fs::write(dir.path().join(".sono-regression-reference"), b"schema=1\n").unwrap();
        fs::write(dir.path().join("reference.json"), b"not json").unwrap();
        assert!(load_reference(dir.path()).is_err());

        let reference = sample_reference(dir.path(), REGRESSION_SCHEMA_VERSION);
        save_sample_reference(dir.path(), &reference);
        fs::write(
            dir.path().join(&reference.modes[0].frames[0].png_file),
            b"corrupt png bytes",
        )
        .unwrap();
        assert!(load_reference(dir.path()).is_err());
    }

    #[test]
    fn json_layers_detect_draw_and_audio_event_changes() {
        let old_draw = json!({"sprites": [{"id": 7, "z": 0.0}]});
        let new_draw = json!({"sprites": [{"id": 8, "z": 0.0}]});
        let draw_diff = compare_json_layer("display_list", &old_draw, &new_draw);
        assert!(draw_diff.changed);
        assert!(draw_diff.detail.unwrap().contains("sprites[0].id"));

        let old_audio = json!({"audio_events": [{"type": "play", "clip_id": 1}]});
        let new_audio = json!({"audio_events": [{"type": "play", "clip_id": 2}]});
        assert!(compare_json_layer("audio_events", &old_audio, &new_audio).changed);
    }

    #[test]
    fn semantic_json_ignores_only_sub_nanosecond_float_noise() {
        let expected = json!({"corners": [[0.12345678901, -0.0000000001]]});
        let equivalent = json!({"corners": [[0.12345678902, 0.0000000002]]});
        let material = json!({"corners": [[0.123456790, 0.0000000002]]});

        assert!(!compare_json_layer("display_list", &expected, &equivalent).changed);
        assert!(compare_json_layer("display_list", &expected, &material).changed);
    }

    #[test]
    fn media_comparison_detects_pre_encoding_pcm_changes() {
        let old = RegressionMediaCapture {
            submitted_frames: 30,
            fps: 30,
            timeline_start: 0.0,
            timeline_end: 1.0,
            bgm_offset: 0.0,
            audio_window: json!({"source_start": 0.0, "leading_silence": 0.0}),
            bgm_pcm_sample_frames: Some(44_100),
            sfx_event_sha1: "events-a".into(),
            sfx_pcm_sha1: Some("pcm-a".into()),
            sfx_event_counts: json!({"audio_events": 1}),
            media_metadata: json!({"video_codec": "h264"}),
        };
        let mut current = old.clone();
        current.sfx_pcm_sha1 = Some("pcm-b".into());
        let diffs = compare_media(&old, &current).unwrap();
        assert!(diffs
            .iter()
            .any(|diff| diff.name == "sfx_mixed_pcm" && diff.changed));
        assert!(diffs.iter().all(|diff| diff.name != "encoded_mp4_bytes"));
    }

    #[test]
    fn pixel_diff_detects_changed_pixel_and_emits_region_statistics() {
        let expected = RgbImage::from_raw(2, 2, vec![0; 12]).unwrap();
        let current = RgbImage::from_raw(2, 2, vec![0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 0]).unwrap();
        let (stats, diff, heat) = pixel_difference(&expected, &current, 0);
        assert_eq!(stats.changed_pixels, 1);
        assert_eq!(stats.changed_bounds, Some([0, 1, 0, 1]));
        assert_eq!(stats.max_channel_delta, 10);
        assert_eq!(diff.len(), 12);
        assert_eq!(heat.len(), 12);
    }

    #[test]
    fn pixel_regression_writes_one_diff_and_one_heatmap_artifact() {
        let root = tempdir().unwrap();
        let expected_dir = root.path().join("reference");
        let current_dir = root.path().join("current");
        let output_dir = root.path().join("run");
        for directory in [&expected_dir, &current_dir] {
            fs::create_dir_all(directory.join("interpreter-1x1")).unwrap();
        }
        let expected_frame = sample_reference(&expected_dir, REGRESSION_SCHEMA_VERSION)
            .modes
            .remove(0)
            .frames
            .remove(0);
        let expected_mode = RegressionModeCapture {
            execution_mode: "interpreter".into(),
            resolution: [1, 1],
            gpu_adapter: None,
            native_regions_executed: 0,
            frames: vec![expected_frame.clone()],
            media: None,
        };
        let mut current_frame = expected_frame.clone();
        current_frame.png_file = "interpreter-1x1/current.png".into();
        current_frame.rgb_sha1 = sha1_hex(&[11, 20, 30]);
        current_frame.timing_samples = vec![timing(10.0)];
        save_rgb_png(
            &expected_dir.join(&expected_frame.png_file),
            1,
            1,
            &[10, 20, 30],
        )
        .unwrap();
        save_rgb_png(
            &current_dir.join(&current_frame.png_file),
            1,
            1,
            &[11, 20, 30],
        )
        .unwrap();
        let result = compare_frame(
            &expected_mode,
            &expected_frame,
            &current_frame,
            &expected_dir,
            &current_dir,
            &output_dir,
            0,
        )
        .unwrap();
        assert!(result.pixels.changed);
        assert_eq!(result.artifacts.len(), 3);
        assert!(result.artifacts[1].ends_with("-diff.png"));
        assert!(result.artifacts[2].ends_with("-heatmap.png"));
        assert!(Path::new(&result.artifacts[1]).is_file());
        assert!(Path::new(&result.artifacts[2]).is_file());
    }

    #[test]
    fn pixel_tolerance_is_bounded_and_only_ignores_small_channel_rounding() {
        let expected = RgbImage::from_raw(1, 1, vec![50, 50, 50]).unwrap();
        let current = RgbImage::from_raw(1, 1, vec![51, 50, 50]).unwrap();
        assert_eq!(pixel_difference(&expected, &current, 1).0.changed_pixels, 0);
        assert_eq!(pixel_difference(&expected, &current, 0).0.changed_pixels, 1);
        assert!(MAX_PIXEL_CHANNEL_TOLERANCE <= 2);
    }

    #[test]
    fn frame_timing_detects_material_change_and_suppresses_noise() {
        let changed = compare_timing(&[timing(10.0)], &[timing(20.0)]);
        assert_eq!(changed.classification, "changed");
        assert_eq!(changed.absolute_difference_ms, Some(10.0));

        let noise = compare_timing(&[timing(10.0)], &[timing(11.0)]);
        assert_eq!(noise.classification, "within_noise");
    }

    #[test]
    fn workload_identity_rejects_unmatched_input_or_resolution() {
        let old = RegressionWorkload {
            engine_sha1: "engine-a".into(),
            resources_sha1: "resources".into(),
            level_sha1: "level".into(),
            music_sha1: None,
            effective_config_sha1: "config".into(),
            fps: 60,
            start_time_micros: 0,
            duration_micros: 1,
            resolutions: vec![[1280, 720]],
            pixel_backend: "cpu".into(),
            gameplay_ui_mode: "custom".into(),
        };
        let new = RegressionWorkload {
            engine_sha1: "engine-b".into(),
            ..old.clone()
        };
        assert_eq!(workload_differences(&old, &new), vec!["engine content"]);
    }

    #[test]
    fn reference_updates_require_replace_flag_and_marked_reference() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("unrelated");
        fs::create_dir(&existing).unwrap();
        let result = update_reference(
            &crate::render::RenderConfig::default(),
            &RegressionOptions {
                fixture_id: "test".into(),
                checkpoint_times: vec![0.0],
                resolutions: vec![[1, 1]],
                compare_vm_gcc: false,
                timing_samples: 1,
                include_media: false,
            },
            &existing,
            true,
        );
        assert!(result.is_err());
        assert!(existing.is_dir());
    }

    #[test]
    fn replacement_summary_reports_removed_coverage_and_build_changes() {
        let directory = tempdir().unwrap();
        let old = sample_reference(directory.path(), REGRESSION_SCHEMA_VERSION);
        let mut new = old.clone();
        new.modes.clear();
        new.checkpoint_times.push(1.0);
        new.timing_samples_requested = 2;
        new.environment.renderer_commit = Some("new-commit".into());

        let changes = summarize_reference_changes(&old, &new);
        assert!(changes
            .iter()
            .any(|change| change.contains("removed interpreter")));
        assert!(changes
            .iter()
            .any(|change| change == "checkpoint schedule changed"));
        assert!(changes
            .iter()
            .any(|change| change.contains("timing sample count changed")));
        assert!(changes
            .iter()
            .any(|change| change == "capture environment/build metadata changed"));
    }
}
