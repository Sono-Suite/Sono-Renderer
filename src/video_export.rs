//! FFmpeg-backed media export. Deterministic frames come from `offline`.

use crate::runtime::{
    AudioEffectEvent, ScheduledEffect, ScheduledLoopedEffect, ScheduledLoopedEffectStop,
};
use crate::{
    ffmpeg,
    offline::{
        hash_draw_commands, hash_rgb, AudioWindow, EventSession, FrameRange, FrameSession,
        PreparedFrame, RenderedFrame,
    },
    render::RenderConfig,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct ExportReport {
    pub performance: crate::performance::RenderPerformanceRecord,
    pub diagnostics: crate::render_diagnostics::RenderDiagnosticRecord,
    pub watch_traversals: u32,
    pub audio_prepass_checkpoints: Vec<crate::audio_prepass_profile::AudioPrepassCheckpoint>,
    pub mv: Option<std::path::PathBuf>,
    pub timeline: crate::export_timeline::ExportTimeline,
    pub chart_end: Option<crate::export_end::ChartEnd>,
    pub bgm_source_duration: Option<f64>,
    pub bgm_pcm_sample_frames: Option<u64>,
    pub ffmpeg_source: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub submitted_frames: u64,
    pub fps: u32,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub bgm_offset: f64,
    pub audio_window: AudioWindow,
    pub sfx_event_sha1: String,
    /// Hash of mixed, pre-AAC SFX samples; absent when no playable SFX PCM was
    /// produced. Compressed audio bytes are not a stable PCM oracle.
    pub sfx_pcm_sha1: Option<String>,
    pub sfx_event_counts: SfxEventCounts,
    pub sfx_playback_events: usize,
    pub sfx_resources_used: usize,
    pub sfx_decoded_unique_resources: usize,
    pub sfx_ffmpeg_decodes: usize,
    pub event_pass_runtime_frames: u64,
    pub event_pass_callbacks: u64,
    pub event_pass_evaluations: u64,
    pub stream_runtime_frames: u64,
    pub stream_callbacks: u64,
    pub stream_evaluations: u64,
    pub watch_execution_status: String,
    pub stream_sono_gcc_regions_executed: u64,
    pub stream_sono_gcc_vm_to_native_cut_calls: u64,
    pub concurrent_sfx_prepass_used: bool,
    pub rgb_spool_bytes: u64,
    pub frame_pipeline: Option<FramePipelineStats>,
    pub deterministic_frame_hashes_match: Option<bool>,
    pub first_pass_hash: Option<String>,
    pub second_pass_hash: String,
    pub frame_diagnostics: Vec<FrameDiagnostic>,
    pub probe: Value,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FramePipelineStats {
    pub queue_capacity: usize,
    pub queue_high_water: usize,
    pub render_workers: usize,
    pub producer_watch_ms: f64,
    pub producer_snapshot_ms: f64,
    pub producer_blocked_ms: f64,
    pub queue_residence_ms: f64,
    pub consumer_waiting_ms: f64,
    pub render_ms: f64,
    pub output_ms: f64,
    pub first_frame_latency_ms: f64,
    pub render_phase_ms: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SfxEventCounts {
    pub audio_events: u64,
    pub scheduled_effects: u64,
    pub loop_starts: u64,
    pub loop_stops: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameDiagnostic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mv_media_pts: Option<f64>,
    pub output_local_frame_index: u64,
    pub global_frame_index: u64,
    pub requested_timeline_time: f64,
    pub runtime_timeline: Option<f64>,
    pub runtime_update: [f64; 4],
    pub runtime_update_after_callbacks: [f64; 4],
    pub timescale: f64,
    pub callback_count: usize,
    pub vm_evaluations: u64,
    pub sono_gcc_regions_executed: u64,
    pub sono_gcc_vm_to_native_cut_calls: u64,
    pub runtime_frame_count: u64,
    pub runtime_entity_count: usize,
    pub active_entity_count: usize,
    pub draw_count: usize,
    pub raw_draw_sha1: String,
    pub rgb_sha1: String,
    pub target_entity_draws: Vec<EntityDrawDiagnostic>,
    #[serde(skip)]
    audio_events: Vec<AudioEffectEvent>,
    #[serde(skip)]
    scheduled_effects: Vec<ScheduledEffect>,
    #[serde(skip)]
    scheduled_looped_effects: Vec<ScheduledLoopedEffect>,
    #[serde(skip)]
    scheduled_looped_effect_stops: Vec<ScheduledLoopedEffectStop>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityDrawDiagnostic {
    pub draw_node: usize,
    pub sprite_id: u32,
    pub corners: [[f64; 2]; 4],
    pub unlerp_values: Vec<UnlerpDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnlerpDiagnostic {
    pub node: usize,
    pub function: String,
    pub arguments: Vec<usize>,
    pub argument_values: Vec<Option<f64>>,
    pub result: f64,
}

pub struct ExportRequest<'a> {
    pub engine: &'a Path,
    pub resources: &'a Path,
    pub level: &'a Path,
    pub music: &'a Path,
    pub mv: Option<&'a Path>,
    pub mv_background: f64,
    pub apply_engine_background_rotation: bool,
    pub resource_overrides: &'a crate::render::ResourceOverrides,
    pub output: &'a Path,
    pub start_time: f64,
    pub duration: f64,
    pub whole_chart: bool,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub video_crf: u8,
    pub video_preset: crate::render::VideoPreset,
    pub trace_entity_id: Option<usize>,
    pub level_option_overrides: &'a [(usize, f64)],
    pub skin_name: Option<&'a str>,
    pub particles_enabled: bool,
    pub sfx_enabled: bool,
    pub bgm_enabled: bool,
    pub ui: &'a crate::render_ui::RendererUiConfig,
    pub backend: crate::render::RenderBackend,
    pub execution_mode: crate::sono_gcc::WatchExecutionMode,
    pub watch_compatibility_policy: crate::runtime::WatchCompatibilityPolicy,
    pub watch_random_seed: u64,
    pub parallel_watch_updates: bool,
    pub watch_workers: Option<usize>,
    pub compile_sono_gcc_on_demand: bool,
    pub profile: bool,
    pub profile_frames: bool,
    pub validate_determinism: bool,
    pub concurrent_sfx_prepass: bool,
    pub frame_pipeline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepassPlan {
    None,
    FullValidation,
}

fn prepass_plan(_sfx_enabled: bool, validate_determinism: bool) -> PrepassPlan {
    if validate_determinism {
        PrepassPlan::FullValidation
    } else {
        PrepassPlan::None
    }
}

pub fn export_config(config: &RenderConfig) -> Result<ExportReport> {
    export_config_with_progress(config, Box::new(|_| {}))
}

pub fn export_config_with_progress(
    config: &RenderConfig,
    sink: crate::export_progress::ProgressSink,
) -> Result<ExportReport> {
    export_config_controlled(
        config,
        sink,
        crate::export_control::ExportControl::default(),
    )
}

pub fn export_config_controlled(
    config: &RenderConfig,
    sink: crate::export_progress::ProgressSink,
    control: crate::export_control::ExportControl,
) -> Result<ExportReport> {
    export_config_with_tools_controlled(config, sink, ffmpeg::resolve, control)
}

fn export_config_with_tools_controlled(
    config: &RenderConfig,
    sink: crate::export_progress::ProgressSink,
    resolve_tools: impl FnOnce() -> Result<ffmpeg::FfmpegInstallation>,
    control: crate::export_control::ExportControl,
) -> Result<ExportReport> {
    config.validate()?;
    control.check()?;
    let output = config
        .output
        .as_deref()
        .context("video output path is required")?;
    if config.layers.bgm && config.music.is_none() {
        bail!("BGM is enabled but no music file is selected");
    }
    let options: Vec<_> = config
        .level_options
        .iter()
        .map(|item| (item.index, item.value))
        .collect();
    export_using_tools_controlled(
        ExportRequest {
            engine: &config.engine,
            resources: &config.resources,
            level: &config.level,
            music: config.music.as_deref().unwrap_or(&config.resources),
            mv: config.mv.as_deref().filter(|_| config.mv_enabled),
            mv_background: config.mv_background,
            apply_engine_background_rotation: config.apply_engine_background_rotation,
            resource_overrides: &config.resource_overrides,
            output,
            start_time: config.start_time,
            duration: config.duration,
            whole_chart: config.whole_chart,
            fps: config.fps,
            width: config.width,
            height: config.height,
            video_crf: config.video_crf,
            video_preset: config.video_preset,
            trace_entity_id: config.trace_entity_id,
            level_option_overrides: &options,
            skin_name: config.skin.as_deref(),
            particles_enabled: config.layers.particles,
            sfx_enabled: config.layers.sfx,
            bgm_enabled: config.layers.bgm,
            ui: &config.ui,
            backend: config.backend,
            execution_mode: config.execution_mode,
            watch_compatibility_policy: config.watch_compatibility_policy,
            watch_random_seed: config.watch_random_seed,
            parallel_watch_updates: config.parallel_watch_updates,
            watch_workers: config.watch_workers,
            compile_sono_gcc_on_demand: config.compile_sono_gcc_on_demand,
            profile: config.profile,
            profile_frames: config.profile_frames,
            validate_determinism: config.validate_determinism,
            concurrent_sfx_prepass: config.concurrent_sfx_prepass,
            frame_pipeline: config.frame_pipeline,
        },
        sink,
        resolve_tools,
        control,
    )
}

struct ExportChild(std::process::Child);
impl std::ops::Deref for ExportChild {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ExportChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for ExportChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn export(request: ExportRequest<'_>) -> Result<ExportReport> {
    export_with_progress(request, Box::new(|_| {}))
}

pub fn export_with_progress(
    request: ExportRequest<'_>,
    sink: crate::export_progress::ProgressSink,
) -> Result<ExportReport> {
    export_using_tools(request, sink, ffmpeg::resolve)
}

fn export_using_tools(
    request: ExportRequest<'_>,
    sink: crate::export_progress::ProgressSink,
    resolve_tools: impl FnOnce() -> Result<ffmpeg::FfmpegInstallation>,
) -> Result<ExportReport> {
    export_using_tools_controlled(request, sink, resolve_tools, Default::default())
}

fn export_using_tools_controlled(
    mut request: ExportRequest<'_>,
    sink: crate::export_progress::ProgressSink,
    resolve_tools: impl FnOnce() -> Result<ffmpeg::FfmpegInstallation>,
    control: crate::export_control::ExportControl,
) -> Result<ExportReport> {
    let sfx_decode_snapshot = crate::audio::decode_cache_stats();
    crate::render::validate_mv_background(request.mv_background)?;
    control.check()?;
    use crate::export_progress::{ExportPhase, ProgressTracker, ProgressWriter};
    let progress = ProgressTracker::new(sink);
    progress.phase(ExportPhase::Preparing, None);
    let export_started_at = Instant::now();
    let export_started_unix_ms = crate::performance::unix_time_ms();
    let export_start = request.profile.then(Instant::now);
    let mut profile = request.profile.then(|| {
        crate::profiling::ProfileCollector::new(
            request.profile_frames,
            match request.backend {
                crate::render::RenderBackend::Cpu => "cpu",
                crate::render::RenderBackend::Wgpu => "wgpu",
            },
            (request.width, request.height),
            request.fps,
        )
    });
    let startup_start = request.profile.then(Instant::now);
    for (label, path) in [
        ("engine", request.engine),
        ("resources", request.resources),
        ("level data", request.level),
    ] {
        if !path.is_file() {
            bail!(
                "{label} input is not an accessible file: {}",
                path.display()
            );
        }
    }
    if request.bgm_enabled && !request.music.is_file() {
        bail!(
            "BGM input is not an accessible file: {}",
            request.music.display()
        );
    }
    if let Some(parent) = request
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating output directory {}", parent.display()))?;
    }
    if request.width % 2 != 0 || request.height % 2 != 0 {
        bail!("H.264 yuv420p output requires even frame dimensions");
    }
    if let Ok(output_path) = request.output.canonicalize() {
        for input in [
            Some(request.engine),
            Some(request.resources),
            Some(request.level),
            Some(request.music),
            request.mv,
            request.resource_overrides.particles.as_deref(),
            request.resource_overrides.effects.as_deref(),
            request.resource_overrides.background.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if input.canonicalize().ok().as_ref() == Some(&output_path) {
                bail!("export output must not overwrite an input file");
            }
        }
    }
    // Validate before initializing any runtime/discovery pass.
    FrameRange::new(request.start_time, request.duration, request.fps)?;
    let package = crate::formats::load_engine(request.engine)?;
    let level = crate::formats::load_level(request.level)?;
    let resource_defaults: std::collections::BTreeMap<_, _> =
        package.metadata.resource_defaults().into_iter().collect();
    let skin_name = request
        .skin_name
        .or_else(|| {
            resource_defaults
                .get("skins")
                .map(String::as_str)
                .or(package.metadata.skin_name.as_deref())
        })
        .context("engine has no default skin name")?;
    let effect_name = resource_defaults
        .get("effects")
        .map(String::as_str)
        .or(package.metadata.effect_name.as_deref());
    let particle_name = resource_defaults
        .get("particles")
        .map(String::as_str)
        .or(package.metadata.particle_name.as_deref());
    let particle_name = request
        .resource_overrides
        .particle_name
        .as_deref()
        .or(particle_name);
    let background_name = resource_defaults
        .get("backgrounds")
        .map(String::as_str)
        .or(package.metadata.background_name.as_deref());
    let bgm_offset = level.bgm_offset.unwrap_or(0.0);
    if !bgm_offset.is_finite() {
        bail!("level bgmOffset must be finite");
    }
    let installation = resolve_tools()?;
    let bgm_source_duration = request
        .bgm_enabled
        .then(|| crate::export_media::source_duration(&installation, request.music))
        .transpose()?;
    let effect_assets = request
        .sfx_enabled
        .then_some(effect_name)
        .flatten()
        .map(|name| {
            crate::formats::load_effect_assets_optional(
                request
                    .resource_overrides
                    .effects
                    .as_deref()
                    .unwrap_or(request.resources),
                name,
            )
        })
        .transpose()?
        .flatten()
        .unwrap_or_else(|| crate::formats::EffectAssets {
            clips: std::collections::BTreeMap::new(),
        });
    let bindings = if request.sfx_enabled {
        crate::offline::effect_clip_bindings(&package.watch)?
    } else {
        std::collections::BTreeMap::new()
    };
    let mut initial_sono_gcc_metrics = None;
    let chart_end = if request.whole_chart {
        let mut discovery = EventSession::new_configured_with_sources(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            request.resources,
            skin_name,
            background_name,
            effect_name,
            particle_name,
            request.width,
            request.height,
            request.fps,
            request.level_option_overrides,
            request.particles_enabled,
            false,
            false,
            request.resource_overrides,
        )?;
        discovery.set_execution_mode_with_compile_policy(
            request.execution_mode,
            request.compile_sono_gcc_on_demand,
        );
        discovery.set_watch_compatibility(
            request.watch_compatibility_policy,
            request.watch_random_seed,
        )?;
        discovery.set_parallel_updates(request.parallel_watch_updates, request.watch_workers)?;
        discovery.set_export_control(control.clone());
        let end = crate::export_end::discover(
            &mut discovery,
            request.fps,
            bgm_source_duration.map(|duration| {
                crate::export_timeline::media_to_watch_time(duration, bgm_offset).max(0.0)
            }),
            &effect_assets,
            &bindings,
            Some(&progress),
            &package.watch,
        )?;
        initial_sono_gcc_metrics = discovery.performance_compile_metrics();
        control.log(format!(
            "Whole-chart end: {:.6}s ({})",
            end.watch_end, end.confidence
        ));
        let aligned_start =
            (request.start_time * f64::from(request.fps)).ceil() / f64::from(request.fps);
        if end.watch_end <= aligned_start {
            bail!("whole-chart start is at or beyond the inferred end");
        }
        request.duration = end.watch_end - aligned_start;
        eprintln!(
            "Whole-chart end: {:.6}s ({}, {}; {} persistent non-input entities)",
            end.watch_end, end.confidence, end.policy, end.persistent_entity_count
        );
        Some(end)
    } else {
        None
    };
    let range = FrameRange::new(request.start_time, request.duration, request.fps)?;
    let timeline =
        crate::export_timeline::ExportTimeline::new(request.start_time, range, bgm_offset)?;
    let audio = crate::offline::audio_window(timeline.watch_start, bgm_offset)?;
    progress.phase(ExportPhase::Preparing, Some(range.frame_count));

    let watch_traversals = std::cell::Cell::new(0);
    let watch_execution_status = std::cell::RefCell::new(String::new());
    let sono_gcc_metrics = std::cell::RefCell::new(initial_sono_gcc_metrics);
    let make_session = || {
        let mut session = FrameSession::new_configured_with_sources(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            request.resources,
            skin_name,
            background_name,
            effect_name,
            particle_name,
            request.width,
            request.height,
            request.fps,
            request.level_option_overrides,
            request.particles_enabled,
            request.ui.clone(),
            request.start_time,
            request.duration,
            request.backend,
            request.profile,
            request.trace_entity_id.is_some(),
            request.resource_overrides,
        )?;
        session.set_execution_mode_with_compile_policy(
            request.execution_mode,
            request.compile_sono_gcc_on_demand,
        );
        session.set_watch_compatibility(
            request.watch_compatibility_policy,
            request.watch_random_seed,
        )?;
        session.set_parallel_updates(request.parallel_watch_updates, request.watch_workers)?;
        session.set_apply_engine_background_rotation(request.apply_engine_background_rotation);
        if sono_gcc_metrics.borrow().is_none() {
            *sono_gcc_metrics.borrow_mut() = session.performance_compile_metrics();
        }
        *watch_execution_status.borrow_mut() = format!(
            "{}; {}",
            session.execution_mode_status(),
            session.parallel_updates_status()
        );
        session.set_export_control(control.clone());
        if let Some(path) = request.mv {
            session.configure_mv(&installation, path, timeline)?;
            session.set_mv_background(request.mv_background);
        }
        watch_traversals.set(watch_traversals.get() + 1);
        Ok::<_, anyhow::Error>(session)
    };
    // Only explicit determinism validation has a second playback traversal.
    let plan = prepass_plan(request.sfx_enabled, request.validate_determinism);
    let concurrent_sfx_prepass_used = false;
    let rgb_spool_bytes = 0;
    let mut expected_frames: Option<Vec<FrameDiagnostic>> = None;
    if plan == PrepassPlan::FullValidation {
        progress.phase(ExportPhase::Validating, None);
        let initialization = make_session()?;
        let preflight_start = Instant::now();
        expected_frames = Some(render_diagnostic_pass(
            initialization,
            range,
            &package.watch,
            request.trace_entity_id,
            profile.as_mut(),
        )?);
        if let Some(p) = profile.as_mut() {
            p.record(
                "Full determinism validation preflight",
                preflight_start.elapsed(),
            );
            p.record_preflight_wrapper(preflight_start.elapsed());
        }
    } else if let Some(p) = profile.as_mut() {
        p.record(
            "Initialization/startup",
            startup_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    let audio_prepass_checkpoints = Vec::new();
    let event_pass_runtime_frames = 0;
    let event_pass_callbacks = 0;
    let event_pass_evaluations = 0;
    let first_pass_hash = expected_frames.as_ref().map(|frames| {
        hash_sequence(
            &frames
                .iter()
                .map(|frame| frame.rgb_sha1.clone())
                .collect::<Vec<_>>(),
        )
    });
    // FFmpeg process startup and the streaming FrameSession/resource setup are
    // preparation work. Keep them out of the Rendering phase wall span.
    progress.phase(ExportPhase::Preparing, Some(range.frame_count));
    let ffmpeg_start = request.profile.then(Instant::now);
    let mut command = Command::new(&installation.ffmpeg);
    crate::export_control::hide_child_window(&mut command);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "rawvideo",
        "-pixel_format",
        "rgb24",
        "-video_size",
        &format!("{}x{}", request.width, request.height),
        "-framerate",
        &request.fps.to_string(),
    ]);
    command.args(["-i", "pipe:0"]);
    // An MP4 track timescale equal to this integer FPS represents every frame
    // duration exactly. FFmpeg's default timescale can round rates such as 239
    // FPS (for short clips, 27 frames were probed as 240 FPS).
    let video_track_timescale = request.fps.to_string();
    let video_crf = request.video_crf.to_string();
    command.args([
        "-vf",
        "format=yuv420p",
        "-frames:v",
        &range.frame_count.to_string(),
        "-t",
        &format!("{:.12}", range.frame_count as f64 / f64::from(range.fps)),
        "-c:v",
        "libx264",
        "-preset",
        request.video_preset.ffmpeg_name(),
        "-crf",
        &video_crf,
        "-pix_fmt",
        "yuv420p",
        "-video_track_timescale",
        &video_track_timescale,
        "-movflags",
        "+faststart",
    ]);
    let output_directory = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let video_file = tempfile::Builder::new()
        .prefix("sono-video-")
        .suffix(".mp4")
        .tempfile_in(output_directory)?;
    let final_file = tempfile::Builder::new()
        .prefix("sono-final-")
        .suffix(".mp4")
        .tempfile_in(output_directory)?;
    command.args(["-an"]).arg(video_file.path());
    // Drain FFmpeg diagnostics to a file so a noisy failure cannot fill a
    // stderr pipe while the render worker is blocked writing video frames.
    let mut ffmpeg_stderr_capture =
        tempfile::tempfile().context("creating temporary FFmpeg diagnostic capture")?;
    let ffmpeg_stderr_writer = ffmpeg_stderr_capture
        .try_clone()
        .context("cloning FFmpeg diagnostic capture handle")?;
    let mut child = ExportChild(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(ffmpeg_stderr_writer))
            .spawn()
            .with_context(|| format!("launching FFmpeg at {}", installation.ffmpeg.display()))?,
    );
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFmpeg startup",
            ffmpeg_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }

    let mut frame_pipeline_stats = None;
    let stdin = child
        .stdin
        .take()
        .context("FFmpeg process did not expose its raw-video input")?;
    let stdin = ProgressWriter::new(
        stdin,
        progress.clone(),
        request.width as usize * request.height as usize * 3,
    )
    .with_control(control.clone());
    let (actual_frames, audio_requests) = {
        let streaming_session_start = request.profile.then(Instant::now);
        let streaming_session = make_session()?;
        if let Some(p) = profile.as_mut() {
            p.record(
                "Streaming session initialization",
                streaming_session_start.map_or(Duration::ZERO, |t| t.elapsed()),
            );
            p.begin_render();
        }
        progress.phase(ExportPhase::Rendering, None);
        let frames = if request.frame_pipeline {
            match stream_frames_pipelined(
                streaming_session,
                range,
                stdin,
                expected_frames.as_deref(),
                &package.watch,
                request.trace_entity_id,
                request.profile,
            ) {
                Ok(result) => {
                    frame_pipeline_stats = Some(result.stats.clone());
                    if let Some(p) = profile.as_mut() {
                        p.set_pipeline_steady_window(
                            range.frame_count.saturating_sub(1),
                            Duration::from_secs_f64(
                                (result.stats.render_phase_ms
                                    - result.stats.first_frame_latency_ms)
                                    .max(0.0)
                                    / 1000.0,
                            ),
                        );
                        eprintln!("Pipeline stage totals overlap across frames; render includes nested raster/GPU wait/readback/unpack/UI. Producer blocked time and queue residence are correlated and must not be summed.");
                        for record in &result.records {
                            if let Some(frame_profile) = record.profile.as_ref() {
                                p.record_frame(
                                    "stream",
                                    record.global_index,
                                    record.timeline,
                                    frame_profile,
                                    record.diagnostics,
                                    record.output,
                                    record.wall,
                                    "FFmpeg handoff",
                                );
                                p.finish_stream_frame_profile(
                                    record.wall,
                                    Duration::ZERO,
                                    frame_profile.total,
                                    record.diagnostics,
                                    record.output,
                                );
                            }
                        }
                        eprintln!(
                            "Frame pipeline: queue capacity {}/{}, render workers {}, Watch {:.1}ms, snapshot {:.1}ms, producer blocked {:.1}ms, queue residence {:.1}ms, consumer waiting {:.1}ms, render {:.1}ms, FFmpeg output {:.1}ms, first-frame latency {:.1}ms",
                            result.stats.queue_high_water,
                            result.stats.queue_capacity,
                            result.stats.render_workers,
                            result.stats.producer_watch_ms,
                            result.stats.producer_snapshot_ms,
                            result.stats.producer_blocked_ms,
                            result.stats.queue_residence_ms,
                            result.stats.consumer_waiting_ms,
                            result.stats.render_ms,
                            result.stats.output_ms,
                            result.stats.first_frame_latency_ms,
                        );
                    }
                    (result.diagnostics, result.audio_requests)
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error).context("streaming pipelined RGB frames to FFmpeg");
                }
            }
        } else {
            match stream_frames(
                streaming_session,
                range,
                stdin,
                expected_frames.as_deref(),
                &package.watch,
                request.trace_entity_id,
                profile.as_mut(),
                "FFmpeg handoff",
                "FFmpeg final input flush/close",
                None,
            ) {
                Ok(frames) => {
                    let requests = collect_audio_requests(&frames);
                    (frames, requests)
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error).context("streaming deterministic RGB frames to FFmpeg");
                }
            }
        };
        if let Some(p) = profile.as_mut() {
            p.finish_render();
        }
        frames
    };
    progress.validate_complete()?;
    progress.phase(ExportPhase::AudioMixing, None);
    let finalize_start = request.profile.then(Instant::now);
    let output = loop {
        control.check()?;
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    ffmpeg_stderr_capture
        .seek(SeekFrom::Start(0))
        .context("rewinding FFmpeg diagnostic capture")?;
    let mut ffmpeg_stderr = String::new();
    ffmpeg_stderr_capture
        .read_to_string(&mut ffmpeg_stderr)
        .context("reading FFmpeg diagnostic capture")?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFmpeg finalization",
            finalize_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    if !output.success() {
        bail!(
            "FFmpeg failed with status {:?}: {}",
            output.code(),
            ffmpeg_stderr.trim()
        );
    }
    // Audio comes exclusively from the authoritative rendering Watch producer.
    let audio_requests = if request.sfx_enabled {
        audio_requests
    } else {
        empty_audio_requests()
    };
    let sfx_event_sha1 = hash_audio_requests(&audio_requests)?;
    let sfx_event_counts = count_audio_requests(&audio_requests);
    let audio_start = Instant::now();
    let bgm_pcm = request
        .bgm_enabled
        .then(|| {
            crate::export_media::prepare_bgm_controlled(
                &installation,
                request.music,
                timeline,
                &control,
            )
        })
        .transpose()?;
    let (events, scheduled, loop_starts, loop_stops) = audio_requests;
    let effect_mix = if request.sfx_enabled {
        crate::audio::mix_effects_wav_with_pcm_hash(
            &effect_assets,
            &bindings,
            &events,
            &scheduled,
            &loop_starts,
            &loop_stops,
            timeline.watch_start,
            timeline.duration,
            request.profile,
        )?
    } else {
        crate::audio::SfxMixResult {
            wav: None,
            pcm_sha1: None,
            warnings: vec![],
            unique_resources: 0,
            ffmpeg_decodes: 0,
            playback_events: 0,
        }
    };
    control.check()?;
    for warning in &effect_mix.warnings {
        control.log(format!("Warning: {warning}"));
        eprintln!("warning: {warning}");
    }
    control.log(format!(
        "SFX audio: {} playback events, {} unique resources, {} FFmpeg decodes",
        effect_mix.playback_events, effect_mix.unique_resources, effect_mix.ffmpeg_decodes
    ));
    let sfx_decode_totals = crate::audio::decode_cache_stats();
    control.log(format!(
        "SFX decode cache for export: {} new unique resources, {} FFmpeg decodes; process totals {} resources / {} decodes",
        sfx_decode_totals.0.saturating_sub(sfx_decode_snapshot.0),
        sfx_decode_totals.1.saturating_sub(sfx_decode_snapshot.1),
        sfx_decode_totals.0,
        sfx_decode_totals.1
    ));
    let sfx_file = effect_mix
        .wav
        .as_ref()
        .map(|bytes| {
            let mut file = tempfile::Builder::new().suffix(".wav").tempfile()?;
            file.write_all(bytes)?;
            Ok::<_, anyhow::Error>(file)
        })
        .transpose()?;
    if let Some(p) = profile.as_mut() {
        p.record("Post-render audio preparation", audio_start.elapsed());
    }
    progress.phase(ExportPhase::AudioEncoding, None);
    let mut mux = Command::new(&installation.ffmpeg);
    crate::export_control::hide_child_window(&mut mux);
    mux.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video_file.path());
    if let Some(file) = &bgm_pcm {
        mux.args(["-f", "s16le", "-ar", "44100", "-ac", "2", "-i"])
            .arg(file.path());
    }
    if let Some(file) = &sfx_file {
        mux.arg("-i").arg(file.path());
    }
    mux.args(audio_map_args(request.bgm_enabled, sfx_file.is_some()));
    mux.args([
        "-c:v",
        "copy",
        "-t",
        &format!("{:.12}", timeline.duration),
        "-movflags",
        "+faststart",
    ]);
    if request.bgm_enabled || sfx_file.is_some() {
        mux.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    let mux_result = control
        .output(mux.arg(final_file.path()))
        .context("muxing collected Watch audio with encoded video")?;
    if !mux_result.status.success() {
        bail!(
            "post-render audio mux failed: {}",
            String::from_utf8_lossy(&mux_result.stderr)
        );
    }
    progress.phase(ExportPhase::Finalizing, None);
    let probe_start = request.profile.then(Instant::now);
    let probe_output = control.output(crate::export_control::hide_child_window(&mut Command::new(&installation.ffprobe))
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,start_time:stream=index,codec_type,codec_name,width,height,pix_fmt,r_frame_rate,avg_frame_rate,time_base,duration,nb_frames,start_time",
            "-of",
            "json",
            &final_file.path().to_string_lossy(),
        ])
        )
        .context("launching FFprobe on rendered MP4")?;
    if let Some(p) = profile.as_ref() {
        let _ = p;
    }
    if !probe_output.status.success() {
        bail!(
            "FFprobe failed with status {:?}: {}",
            probe_output.status.code(),
            String::from_utf8_lossy(&probe_output.stderr).trim()
        );
    }
    let probe: Value =
        serde_json::from_slice(&probe_output.stdout).context("decoding FFprobe JSON result")?;
    validate_probe(
        &probe,
        range.frame_count,
        request.width,
        request.height,
        request.fps,
        range.frame_count as f64 / f64::from(range.fps),
        request.bgm_enabled || sfx_file.is_some(),
    )?;
    if let Some(p) = profile.as_mut() {
        p.record(
            "FFprobe validation",
            probe_start.map_or(Duration::ZERO, |t| t.elapsed()),
        );
    }
    let second_pass_hash = hash_sequence(
        &actual_frames
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect::<Vec<_>>(),
    );
    let stream_runtime_frames = actual_frames
        .iter()
        .map(|frame| frame.runtime_frame_count)
        .sum();
    let stream_callbacks = actual_frames
        .iter()
        .map(|frame| frame.callback_count as u64)
        .sum();
    let stream_evaluations = actual_frames.iter().map(|frame| frame.vm_evaluations).sum();
    let stream_sono_gcc_regions_executed = actual_frames
        .iter()
        .map(|frame| frame.sono_gcc_regions_executed)
        .sum();
    let stream_sono_gcc_vm_to_native_cut_calls = actual_frames
        .iter()
        .map(|frame| frame.sono_gcc_vm_to_native_cut_calls)
        .sum();
    let deterministic_frame_hashes_match = expected_frames
        .as_ref()
        .map(|expected| compare_frame_diagnostics(expected, &actual_frames));
    if let (Some(p), Some(start)) = (profile.as_mut(), export_start) {
        p.record("Total export", start.elapsed());
        p.print(start.elapsed());
    }
    let watch_execution_status = watch_execution_status.borrow().clone();
    control.check()?;
    final_file
        .persist(request.output)
        .map_err(|error| error.error)
        .context("publishing validated export")?;
    progress.phase(ExportPhase::Complete, None);
    let mut stage_timings = progress
        .phase_timings()
        .into_iter()
        .map(|(phase, elapsed)| crate::performance::stage(format!("{phase:?}"), elapsed, 1))
        .collect::<Vec<_>>();
    if let Some(profile) = &profile {
        stage_timings.extend(profile.attribution_timings());
    }
    let mut performance_config = RenderConfig::default();
    performance_config.engine = request.engine.to_path_buf();
    performance_config.resources = request.resources.to_path_buf();
    performance_config.level = request.level.to_path_buf();
    performance_config.music = request.bgm_enabled.then(|| request.music.to_path_buf());
    performance_config.mv = request.mv.map(Path::to_path_buf);
    performance_config.mv_enabled = request.mv.is_some();
    performance_config.mv_background = request.mv_background;
    performance_config.resource_overrides = request.resource_overrides.clone();
    performance_config.output = Some(request.output.to_path_buf());
    performance_config.start_time = request.start_time;
    performance_config.duration = request.duration;
    performance_config.whole_chart = request.whole_chart;
    performance_config.fps = request.fps;
    performance_config.width = request.width;
    performance_config.height = request.height;
    performance_config.video_crf = request.video_crf;
    performance_config.video_preset = request.video_preset;
    performance_config.level_options = request
        .level_option_overrides
        .iter()
        .map(|(index, value)| crate::render::LevelOptionValue {
            index: *index,
            value: *value,
        })
        .collect();
    performance_config.skin = request.skin_name.map(str::to_owned);
    performance_config.layers = crate::render::RenderLayers {
        particles: request.particles_enabled,
        sfx: request.sfx_enabled,
        bgm: request.bgm_enabled,
    };
    performance_config.ui = request.ui.clone();
    performance_config.backend = request.backend;
    performance_config.execution_mode = request.execution_mode;
    performance_config.parallel_watch_updates = request.parallel_watch_updates;
    performance_config.watch_workers = request.watch_workers;
    performance_config.compile_sono_gcc_on_demand = request.compile_sono_gcc_on_demand;
    performance_config.profile = request.profile;
    performance_config.profile_frames = request.profile_frames;
    performance_config.validate_determinism = request.validate_determinism;
    performance_config.concurrent_sfx_prepass = request.concurrent_sfx_prepass;
    performance_config.frame_pipeline = request.frame_pipeline;
    let mut performance = crate::performance::RenderPerformanceRecord::new(
        &performance_config,
        export_started_unix_ms,
        crate::performance::unix_time_ms(),
        export_started_at.elapsed(),
        stage_timings,
        frame_pipeline_stats
            .as_ref()
            .map(crate::performance::pipeline),
        range.frame_count,
        stream_runtime_frames,
        stream_callbacks,
        stream_evaluations,
        stream_sono_gcc_regions_executed,
        watch_execution_status.clone(),
    );
    if let Some(profile) = &profile {
        performance.environment.gpu_identity = profile.adapter_identity();
    }
    performance.sono_gcc_compilation = sono_gcc_metrics.into_inner();
    if let Some(metrics) = &performance.sono_gcc_compilation {
        performance.gcc_cache_state = Some(metrics.cache_state.clone());
    }
    let exclusive_phase_ms = performance
        .stage_timings
        .iter()
        .filter(|stage| stage.semantics == crate::performance::StageSemantics::ExclusiveWallSpan)
        .map(|stage| stage.duration_ms)
        .sum::<f64>();
    let bookkeeping_residual_ms = performance.execution_time_ms - exclusive_phase_ms;
    if bookkeeping_residual_ms > 0.0 {
        performance.stage_timings.push(crate::performance::stage(
            "Export setup and record finalization",
            Duration::from_secs_f64(bookkeeping_residual_ms / 1000.0),
            1,
        ));
    }
    let ffmpeg_source = match &installation.source {
        ffmpeg::FfmpegSource::SharedAddons => "shared addons".to_owned(),
        ffmpeg::FfmpegSource::Managed => "managed".to_owned(),
    };
    let mut diagnostics = crate::render_diagnostics::RenderDiagnosticRecord::from_export(
        &performance,
        &ffmpeg_source,
        &probe,
        deterministic_frame_hashes_match,
    );
    let (particle_warnings, watch_warnings): (Vec<_>, Vec<_>) = control
        .logged_once_messages()
        .into_iter()
        .partition(|warning| warning.starts_with("particle resource "));
    diagnostics.append_watch_compatibility_warnings(watch_warnings);
    diagnostics.append_particle_resource_warnings(particle_warnings);
    Ok(ExportReport {
        performance,
        diagnostics,
        watch_traversals: watch_traversals.get(),
        audio_prepass_checkpoints,
        mv: request.mv.map(Path::to_path_buf),
        timeline,
        chart_end,
        bgm_source_duration,
        bgm_pcm_sample_frames: bgm_pcm.as_ref().map(|_| timeline.audio_sample_frames),
        ffmpeg_source,
        ffmpeg_path: installation.ffmpeg.display().to_string(),
        ffprobe_path: installation.ffprobe.display().to_string(),
        submitted_frames: range.frame_count,
        fps: range.fps,
        timeline_start: range.start_time(),
        timeline_end: range.time(range.first_index + range.frame_count),
        bgm_offset,
        audio_window: audio,
        sfx_event_sha1,
        sfx_pcm_sha1: effect_mix.pcm_sha1.clone(),
        sfx_event_counts,
        sfx_playback_events: effect_mix.playback_events,
        sfx_resources_used: effect_mix.unique_resources,
        sfx_decoded_unique_resources: sfx_decode_totals.0.saturating_sub(sfx_decode_snapshot.0),
        sfx_ffmpeg_decodes: sfx_decode_totals.1.saturating_sub(sfx_decode_snapshot.1),
        event_pass_runtime_frames,
        event_pass_callbacks,
        event_pass_evaluations,
        stream_runtime_frames,
        stream_callbacks,
        stream_evaluations,
        watch_execution_status,
        stream_sono_gcc_regions_executed,
        stream_sono_gcc_vm_to_native_cut_calls,
        concurrent_sfx_prepass_used,
        rgb_spool_bytes: if concurrent_sfx_prepass_used {
            rgb_spool_bytes
        } else {
            0
        },
        frame_pipeline: frame_pipeline_stats,
        deterministic_frame_hashes_match,
        first_pass_hash,
        second_pass_hash,
        frame_diagnostics: actual_frames,
        probe,
    })
}

fn render_diagnostic_pass(
    mut session: FrameSession<'_>,
    range: FrameRange,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    mut profile: Option<&mut crate::profiling::ProfileCollector>,
) -> Result<Vec<FrameDiagnostic>> {
    (0..range.frame_count)
        .map(|segment_index| {
            let loop_start = profile.as_ref().map(|_| Instant::now());
            let frame_index = range.index(segment_index)?;
            let frame = session.render_global_frame(frame_index)?;
            let diagnostic_start = profile.as_ref().map(|_| Instant::now());
            let diagnostic = frame_diagnostic(
                segment_index,
                frame_index,
                range.time(frame_index),
                &frame,
                watch,
                target_entity_id,
            );
            let diagnostic_time = diagnostic_start.map_or(Duration::ZERO, |start| start.elapsed());
            if let (Some(p), Some(frame_profile), Some(start)) =
                (profile.as_deref_mut(), frame.profile.as_ref(), loop_start)
            {
                p.record_preflight(frame_profile, diagnostic_time, start.elapsed());
            }
            Ok(diagnostic)
        })
        .collect()
}

fn stream_frames(
    mut session: FrameSession<'_>,
    range: FrameRange,
    mut stdin: impl Write,
    expected_frames: Option<&[FrameDiagnostic]>,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    profile: Option<&mut crate::profiling::ProfileCollector>,
    handoff_label: &'static str,
    flush_label: &'static str,
    cancellation: Option<&AtomicU8>,
) -> Result<Vec<FrameDiagnostic>> {
    if expected_frames.is_some_and(|frames| frames.len() != range.frame_count as usize) {
        bail!("determinism validation returned an unexpected number of frames");
    }
    let mut actual_frames = Vec::with_capacity(range.frame_count as usize);
    let mut profile = profile;
    for segment_index in 0..range.frame_count as usize {
        if cancellation.is_some_and(|token| token.load(Ordering::Acquire) != 0) {
            bail!("stream rendering cancelled after concurrent SFX prepass failure");
        }
        let frame_wall = profile.as_ref().map(|_| Instant::now());
        let global_index = range.index(segment_index as u64)?;
        let frame = session.render_global_frame(global_index)?;
        let frame_profile = frame.profile.clone();
        let diagnostic_start = profile.as_ref().map(|_| Instant::now());
        let diagnostic = frame_diagnostic(
            segment_index as u64,
            global_index,
            range.time(global_index),
            &frame,
            watch,
            target_entity_id,
        );
        if let Some(expected) = expected_frames.map(|frames| &frames[segment_index]) {
            if !frame_hashes_match(expected, &diagnostic) {
                bail!("frame {global_index} changed between deterministic passes");
            }
        }
        let diagnostic_time = diagnostic_start.map_or(Duration::ZERO, |start| start.elapsed());
        let handoff_start = profile.as_ref().map(|_| Instant::now());
        stdin
            .write_all(&frame.rgb)
            .with_context(|| format!("writing raw RGB frame {global_index} to export output"))?;
        let handoff_time = handoff_start.map_or(Duration::ZERO, |start| start.elapsed());
        actual_frames.push(diagnostic);
        if let (Some(start), Some(p), Some(frame_profile)) =
            (frame_wall, profile.as_deref_mut(), frame_profile.as_ref())
        {
            let bookkeeping_start = Instant::now();
            p.record_frame(
                "stream",
                global_index,
                range.time(global_index),
                frame_profile,
                diagnostic_time,
                handoff_time,
                start.elapsed(),
                handoff_label,
            );
            let bookkeeping = bookkeeping_start.elapsed();
            p.finish_stream_frame_profile(
                start.elapsed(),
                bookkeeping,
                frame_profile.total,
                diagnostic_time,
                handoff_time,
            );
        }
    }
    let flush_start = profile.as_ref().map(|_| Instant::now());
    stdin.flush().context("flushing raw RGB frames to FFmpeg")?;
    drop(stdin);
    if let (Some(start), Some(p)) = (flush_start, profile.as_deref_mut()) {
        p.record(flush_label, start.elapsed());
    }
    Ok(actual_frames)
}

fn frame_hashes_match(first: &FrameDiagnostic, second: &FrameDiagnostic) -> bool {
    first.raw_draw_sha1 == second.raw_draw_sha1 && first.rgb_sha1 == second.rgb_sha1
}

const FRAME_QUEUE_CAPACITY: usize = 2;
const MAX_GPU_RENDER_WORKERS: usize = 4;

struct PreparedJob {
    local_index: u64,
    global_index: u64,
    timeline: f64,
    ready_at: Instant,
    frame: PreparedFrame,
}

struct PendingRenderJob {
    local_index: u64,
    global_index: u64,
    timeline: f64,
    ready_at: Instant,
}

struct CompletedRenderJob {
    job: PendingRenderJob,
    render_started: Instant,
    render_time: Duration,
    result: Result<RenderedFrame>,
}

fn render_worker_loop(
    receiver: std::sync::mpsc::Receiver<PreparedJob>,
    sender: std::sync::mpsc::SyncSender<CompletedRenderJob>,
    resources: std::sync::Arc<crate::offline::FrameRenderResources>,
    profile_enabled: bool,
) {
    while let Ok(job) = receiver.recv() {
        let PreparedJob {
            local_index,
            global_index,
            timeline,
            ready_at,
            frame,
        } = job;
        let metadata = PendingRenderJob {
            local_index,
            global_index,
            timeline,
            ready_at,
        };
        let render_started = Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::offline::render_prepared_frame(resources.clone(), profile_enabled, frame)
        }))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("GPU render worker panicked")));
        let render_time = render_started.elapsed();
        let failed = result.is_err();
        if sender
            .send(CompletedRenderJob {
                job: metadata,
                render_started,
                render_time,
                result,
            })
            .is_err()
            || failed
        {
            break;
        }
    }
}

struct PipelineFrameRecord {
    global_index: u64,
    timeline: f64,
    profile: Option<crate::offline::FrameStageProfile>,
    diagnostics: Duration,
    output: Duration,
    wall: Duration,
}

struct PipelineWorkerResult {
    diagnostics: Vec<FrameDiagnostic>,
    records: Vec<PipelineFrameRecord>,
    consumer_waiting: Duration,
    queue_residence: Duration,
    render: Duration,
    output: Duration,
    first_frame_latency: Duration,
}

struct PipelineResult {
    audio_requests: AudioRequests,
    diagnostics: Vec<FrameDiagnostic>,
    stats: FramePipelineStats,
    records: Vec<PipelineFrameRecord>,
}

fn stream_frames_pipelined(
    mut session: FrameSession<'_>,
    range: FrameRange,
    mut stdin: impl Write + Send,
    expected_frames: Option<&[FrameDiagnostic]>,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
    profile_enabled: bool,
) -> Result<PipelineResult> {
    if expected_frames.is_some_and(|frames| frames.len() != range.frame_count as usize) {
        bail!("determinism validation returned an unexpected number of frames");
    }
    let resources = session.render_resources();
    let render_workers = if resources.backend() == crate::render::RenderBackend::Wgpu {
        std::env::var("SONO_WGPU_IN_FLIGHT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, MAX_GPU_RENDER_WORKERS)
    } else {
        1
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel::<PreparedJob>(FRAME_QUEUE_CAPACITY);
    let outstanding = AtomicUsize::new(0);
    let high_water = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let pipeline_start = Instant::now();
    let worker_watch = watch;
    let worker_cancelled = &cancelled;
    let worker_outstanding = &outstanding;

    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                std::thread::scope(|render_scope| {
                    let mut records = Vec::with_capacity(range.frame_count as usize);
                    let mut diagnostics = Vec::with_capacity(range.frame_count as usize);
                    let mut consumer_waiting = Duration::ZERO;
                    let mut queue_residence = Duration::ZERO;
                    let mut render_total = Duration::ZERO;
                    let mut output_total = Duration::ZERO;
                    let mut first_frame_latency = Duration::ZERO;
                    let mut emit = |job: PendingRenderJob,
                                    render_started: Instant,
                                    render_time: Duration,
                                    result: Result<RenderedFrame>|
                     -> Result<()> {
                        let mut rendered = result?;
                        render_total += render_time;
                        queue_residence += render_started.duration_since(job.ready_at);
                        if let Some(frame_profile) = rendered.profile.as_mut() {
                            frame_profile.total = frame_profile.vm
                                + frame_profile.preparation
                                + frame_profile.backend_wall
                                + frame_profile.runtime_ui;
                        }
                        let diagnostic_start = Instant::now();
                        let diagnostic = frame_diagnostic(
                            job.local_index,
                            job.global_index,
                            job.timeline,
                            &rendered,
                            worker_watch,
                            target_entity_id,
                        );
                        let diagnostics_time = diagnostic_start.elapsed();
                        if let Some(expected) =
                            expected_frames.map(|frames| &frames[job.local_index as usize])
                        {
                            if !frame_hashes_match(expected, &diagnostic) {
                                bail!(
                                    "frame {} changed between deterministic passes",
                                    job.global_index
                                );
                            }
                        }
                        let output_start = Instant::now();
                        stdin.write_all(&rendered.rgb).with_context(|| {
                            format!("writing raw RGB frame {} to FFmpeg", job.global_index)
                        })?;
                        let output_time = output_start.elapsed();
                        output_total += output_time;
                        if records.is_empty() {
                            first_frame_latency = pipeline_start.elapsed();
                        }
                        records.push(PipelineFrameRecord {
                            global_index: job.global_index,
                            timeline: job.timeline,
                            profile: rendered.profile,
                            diagnostics: diagnostics_time,
                            output: output_time,
                            wall: job.ready_at.elapsed(),
                        });
                        diagnostics.push(diagnostic);
                        Ok(())
                    };
                    if render_workers == 1 {
                        loop {
                            let wait_start = Instant::now();
                            let job = match receiver.recv() {
                                Ok(job) => job,
                                Err(_) => {
                                    consumer_waiting += wait_start.elapsed();
                                    break;
                                }
                            };
                            consumer_waiting += wait_start.elapsed();
                            worker_outstanding.fetch_sub(1, Ordering::AcqRel);
                            if worker_cancelled.load(Ordering::Acquire) {
                                break;
                            }
                            let render_started = Instant::now();
                            let result = crate::offline::render_prepared_frame(
                                resources.clone(),
                                profile_enabled,
                                job.frame,
                            );
                            let render_time = render_started.elapsed();
                            emit(
                                PendingRenderJob {
                                    local_index: job.local_index,
                                    global_index: job.global_index,
                                    timeline: job.timeline,
                                    ready_at: job.ready_at,
                                },
                                render_started,
                                render_time,
                                result,
                            )?;
                        }
                    } else {
                        let (result_sender, result_receiver) =
                            std::sync::mpsc::sync_channel::<CompletedRenderJob>(render_workers);
                        let mut worker_senders = Vec::with_capacity(render_workers);
                        for _ in 0..render_workers {
                            let (job_sender, job_receiver) = std::sync::mpsc::sync_channel(0);
                            worker_senders.push(job_sender);
                            let result_sender = result_sender.clone();
                            let render_resources = resources.clone();
                            render_scope.spawn(move || {
                                render_worker_loop(
                                    job_receiver,
                                    result_sender,
                                    render_resources,
                                    profile_enabled,
                                )
                            });
                        }
                        drop(result_sender);
                        let mut completed = BTreeMap::new();
                        let mut next_to_write = 0u64;
                        let mut submitted = 0u64;
                        let mut worker_index = 0usize;
                        let mut source_closed = false;
                        while next_to_write < range.frame_count {
                            if worker_cancelled.load(Ordering::Acquire) {
                                source_closed = true;
                            }
                            while !source_closed
                                && submitted.saturating_sub(next_to_write) < render_workers as u64
                            {
                                let wait_start = Instant::now();
                                let job = match receiver.recv() {
                                    Ok(job) => job,
                                    Err(_) => {
                                        consumer_waiting += wait_start.elapsed();
                                        source_closed = true;
                                        break;
                                    }
                                };
                                consumer_waiting += wait_start.elapsed();
                                worker_outstanding.fetch_sub(1, Ordering::AcqRel);
                                if worker_cancelled.load(Ordering::Acquire) {
                                    source_closed = true;
                                    break;
                                }
                                worker_senders[worker_index].send(job).map_err(|_| {
                                    anyhow::anyhow!("GPU render worker closed its input queue")
                                })?;
                                worker_index = (worker_index + 1) % render_workers;
                                submitted += 1;
                            }

                            if submitted == next_to_write {
                                if source_closed {
                                    break;
                                }
                                continue;
                            }
                            let finished = result_receiver
                                .recv()
                                .context("receiving completed GPU frame")?;
                            completed.insert(finished.job.local_index, finished);
                            while let Some(finished) = completed.remove(&next_to_write) {
                                emit(
                                    finished.job,
                                    finished.render_started,
                                    finished.render_time,
                                    finished.result,
                                )?;
                                next_to_write += 1;
                            }
                        }
                        drop(worker_senders);
                    }
                    drop(emit);
                    stdin
                        .flush()
                        .context("flushing pipelined RGB frames to FFmpeg")?;
                    drop(stdin);
                    Ok(PipelineWorkerResult {
                        diagnostics,
                        records,
                        consumer_waiting,
                        queue_residence,
                        render: render_total,
                        output: output_total,
                        first_frame_latency,
                    })
                })
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("frame render/output worker panicked")));
            if result.is_err() {
                worker_cancelled.store(true, Ordering::Release);
            }
            result
        });

        let mut audio_requests = empty_audio_requests();
        let mut producer_result: Result<(Duration, Duration, Duration)> =
            Ok((Duration::ZERO, Duration::ZERO, Duration::ZERO));
        let producer_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for local_index in 0..range.frame_count {
                if cancelled.load(Ordering::Acquire) {
                    producer_result = Err(anyhow::anyhow!(
                        "frame pipeline cancelled after worker failure"
                    ));
                    break;
                }
                let global_index = match range.index(local_index) {
                    Ok(index) => index,
                    Err(error) => {
                        producer_result = Err(error);
                        break;
                    }
                };
                let prepared = match session.prepare_global_frame(global_index) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        producer_result =
                            Err(error).context("preparing Watch frame for render pipeline");
                        break;
                    }
                };
                append_report_events(&mut audio_requests, &prepared.report);
                let (watch_time, snapshot_time) = prepared.producer_timings();
                let current = producer_result.as_mut().unwrap();
                current.0 += watch_time;
                current.1 += snapshot_time;
                let ready_at = Instant::now();
                let was_full = outstanding.load(Ordering::Acquire) >= FRAME_QUEUE_CAPACITY;
                let send_start = Instant::now();
                outstanding.fetch_add(1, Ordering::AcqRel);
                let send_result = sender.send(PreparedJob {
                    local_index,
                    global_index,
                    timeline: range.time(global_index),
                    ready_at,
                    frame: prepared,
                });
                let send_time = send_start.elapsed();
                if send_result.is_err() {
                    outstanding.fetch_sub(1, Ordering::AcqRel);
                    producer_result = Err(anyhow::anyhow!(
                        "frame render/output worker closed the queue"
                    ));
                    break;
                }
                if was_full {
                    producer_result.as_mut().unwrap().2 += send_time;
                }
                high_water.fetch_max(
                    outstanding
                        .load(Ordering::Acquire)
                        .min(FRAME_QUEUE_CAPACITY),
                    Ordering::AcqRel,
                );
            }
        }));
        if producer_panic.is_err() {
            producer_result = Err(anyhow::anyhow!("Watch frame producer panicked"));
        }
        drop(sender);
        if producer_result.is_err() {
            cancelled.store(true, Ordering::Release);
        }
        let worker_result = worker
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("frame render/output worker panicked")));
        match worker_result {
            Err(error) => Err(error),
            Ok(worker_result) => {
                let (watch_time, snapshot_time, blocked_time) = producer_result?;
                if worker_result.records.len() != range.frame_count as usize {
                    bail!("frame pipeline stopped before all frames were rendered");
                }
                let render_phase = pipeline_start.elapsed();
                Ok(PipelineResult {
                    audio_requests,
                    diagnostics: worker_result.diagnostics,
                    stats: FramePipelineStats {
                        queue_capacity: FRAME_QUEUE_CAPACITY,
                        queue_high_water: high_water.load(Ordering::Acquire),
                        render_workers,
                        producer_watch_ms: watch_time.as_secs_f64() * 1000.0,
                        producer_snapshot_ms: snapshot_time.as_secs_f64() * 1000.0,
                        producer_blocked_ms: blocked_time.as_secs_f64() * 1000.0,
                        queue_residence_ms: worker_result.queue_residence.as_secs_f64() * 1000.0,
                        consumer_waiting_ms: worker_result.consumer_waiting.as_secs_f64() * 1000.0,
                        render_ms: worker_result.render.as_secs_f64() * 1000.0,
                        output_ms: worker_result.output.as_secs_f64() * 1000.0,
                        first_frame_latency_ms: worker_result.first_frame_latency.as_secs_f64()
                            * 1000.0,
                        render_phase_ms: render_phase.as_secs_f64() * 1000.0,
                    },
                    records: worker_result.records,
                })
            }
        }
    })
}

fn compare_frame_diagnostics(first: &[FrameDiagnostic], second: &[FrameDiagnostic]) -> bool {
    first.len() == second.len()
        && first
            .iter()
            .zip(second)
            .all(|(a, b)| frame_hashes_match(a, b))
}

fn frame_diagnostic(
    output_index: u64,
    global_index: u64,
    time: f64,
    frame: &RenderedFrame,
    watch: &crate::watch::WatchData,
    target_entity_id: Option<usize>,
) -> FrameDiagnostic {
    let target_entity_draws = target_entity_id.map_or_else(Vec::new, |entity_id| {
        frame
            .report
            .display_list
            .sprites
            .iter()
            .filter(|draw| {
                draw.provenance.as_ref().and_then(|source| source.entity_id) == Some(entity_id)
            })
            .map(|draw| {
                let unlerp_values = draw
                    .trace
                    .as_ref()
                    .into_iter()
                    .flat_map(|trace| trace.node_values.iter())
                    .filter_map(|(&node_id, &value)| {
                        let node = watch.nodes.get(node_id)?;
                        let function = node.func.as_deref()?;
                        if !matches!(function, "Unlerp" | "UnlerpClamped") {
                            return None;
                        }
                        let arguments: Vec<usize> = node
                            .args
                            .iter()
                            .filter_map(|arg| arg.as_u64().and_then(|id| usize::try_from(id).ok()))
                            .collect();
                        let argument_values = arguments
                            .iter()
                            .map(|argument| {
                                draw.trace
                                    .as_ref()
                                    .and_then(|trace| trace.node_values.get(argument).copied())
                            })
                            .collect();
                        Some(UnlerpDiagnostic {
                            node: node_id,
                            function: function.to_owned(),
                            arguments,
                            argument_values,
                            result: value,
                        })
                    })
                    .collect();
                EntityDrawDiagnostic {
                    draw_node: draw
                        .provenance
                        .as_ref()
                        .map_or(usize::MAX, |source| source.draw_node),
                    sprite_id: draw.sprite_id,
                    corners: draw.corners,
                    unlerp_values,
                }
            })
            .collect()
    });
    FrameDiagnostic {
        mv_media_pts: frame.mv_media_pts,
        output_local_frame_index: output_index,
        global_frame_index: global_index,
        requested_timeline_time: time,
        runtime_timeline: frame.report.timeline,
        runtime_update: frame.report.runtime_update,
        runtime_update_after_callbacks: frame.report.runtime_update_after_callbacks,
        timescale: frame.report.timescale,
        callback_count: frame.report.callbacks.len(),
        vm_evaluations: frame.report.vm_evaluations,
        sono_gcc_regions_executed: frame.report.runtime_profile.sono_gcc_regions_executed,
        sono_gcc_vm_to_native_cut_calls: frame
            .report
            .runtime_profile
            .sono_gcc_vm_to_native_cut_calls,
        runtime_frame_count: if frame.report.runtime_profile.frame_count > 0 {
            frame.report.runtime_profile.frame_count
        } else if output_index == 0 {
            global_index + 1
        } else {
            1
        },
        runtime_entity_count: frame.report.runtime_entity_count,
        active_entity_count: frame.report.active_entity_count,
        draw_count: frame.report.display_list.sprites.len(),
        raw_draw_sha1: hash_draw_commands(&frame.report.display_list),
        rgb_sha1: hash_rgb(&frame.rgb),
        target_entity_draws,
        audio_events: frame.report.audio_events.clone(),
        scheduled_effects: frame.report.scheduled_effects.clone(),
        scheduled_looped_effects: frame.report.scheduled_looped_effects.clone(),
        scheduled_looped_effect_stops: frame.report.scheduled_looped_effect_stops.clone(),
    }
}

type AudioRequests = (
    Vec<AudioEffectEvent>,
    Vec<ScheduledEffect>,
    Vec<ScheduledLoopedEffect>,
    Vec<ScheduledLoopedEffectStop>,
);

fn empty_audio_requests() -> AudioRequests {
    (Vec::new(), Vec::new(), Vec::new(), Vec::new())
}

fn hash_audio_requests(requests: &AudioRequests) -> Result<String> {
    let bytes = serde_json::to_vec(requests).context("serializing collected SFX event sequence")?;
    Ok(hex::encode(Sha1::digest(bytes)))
}

fn count_audio_requests(requests: &AudioRequests) -> SfxEventCounts {
    SfxEventCounts {
        audio_events: requests.0.len() as u64,
        scheduled_effects: requests.1.len() as u64,
        loop_starts: requests.2.len() as u64,
        loop_stops: requests.3.len() as u64,
    }
}

fn append_report_events(requests: &mut AudioRequests, report: &crate::watch_runtime::FrameReport) {
    requests.0.extend(report.audio_events.iter().cloned());
    requests.1.extend(report.scheduled_effects.iter().cloned());
    requests
        .2
        .extend(report.scheduled_looped_effects.iter().cloned());
    requests
        .3
        .extend(report.scheduled_looped_effect_stops.iter().cloned());
}

fn collect_audio_requests(frames: &[FrameDiagnostic]) -> AudioRequests {
    let mut requests = empty_audio_requests();
    for frame in frames {
        requests.0.extend(frame.audio_events.iter().cloned());
        requests.1.extend(frame.scheduled_effects.iter().cloned());
        requests
            .2
            .extend(frame.scheduled_looped_effects.iter().cloned());
        requests
            .3
            .extend(frame.scheduled_looped_effect_stops.iter().cloned());
    }
    requests
}

#[cfg(test)]
fn collect_event_only_requests(
    session: EventSession<'_>,
    range: FrameRange,
    profile_enabled: bool,
    cancellation: &AtomicU8,
) -> Result<EventPassOutput> {
    collect_event_only_requests_with_progress(session, range, profile_enabled, cancellation, None)
}

#[cfg(test)]
fn collect_event_only_requests_with_progress(
    mut session: EventSession<'_>,
    range: FrameRange,
    profile_enabled: bool,
    cancellation: &AtomicU8,
    progress: Option<&crate::export_progress::ProgressTracker>,
) -> Result<EventPassOutput> {
    let mut requests = empty_audio_requests();
    let mut checkpoint_accumulator =
        profile_enabled.then(crate::audio_prepass_profile::Checkpoints::new);
    let mut checkpoints = Vec::new();
    let mut previous_index = None;
    let (mut runtime_frames, mut callbacks, mut evaluations) = (0, 0, 0);
    for segment_index in 0..range.frame_count {
        if cancellation.load(Ordering::Acquire) != 0 {
            bail!("SFX event pass cancelled after concurrent render failure");
        }
        let frame_index = range.index(segment_index)?;
        let report = session.advance_global_frame(frame_index)?;
        if let Some(progress) = progress {
            progress.pulse();
        }
        append_report_events(&mut requests, &report);
        runtime_frames += previous_index.map_or(frame_index + 1, |previous| frame_index - previous);
        previous_index = Some(frame_index);
        callbacks += report.callbacks.len() as u64;
        evaluations += report.vm_evaluations;
        if let Some(accumulator) = checkpoint_accumulator.as_mut() {
            if let Some(checkpoint) = accumulator.observe(
                &report,
                session.runtime(),
                (frame_index + 1) as f64 / f64::from(range.fps),
                segment_index + 1 == range.frame_count,
            ) {
                eprintln!(
                    "AudioPrepass checkpoint {}",
                    serde_json::to_string(&checkpoint)?
                );
                checkpoints.push(checkpoint);
            }
        }
    }
    Ok(EventPassOutput {
        checkpoints,
        requests,
        runtime_frames,
        callbacks,
        evaluations,
    })
}

#[cfg(test)]
struct EventPassOutput {
    checkpoints: Vec<crate::audio_prepass_profile::AudioPrepassCheckpoint>,
    requests: AudioRequests,
    runtime_frames: u64,
    callbacks: u64,
    evaluations: u64,
}

fn hash_sequence(hashes: &[String]) -> String {
    let mut digest = Sha1::new();
    for hash in hashes {
        digest.update(hash.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn audio_map_args(has_bgm: bool, has_sfx: bool) -> Vec<String> {
    match (has_bgm, has_sfx) {
        (true, true) => vec![
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "[aout]".into(),
            "-filter_complex".into(),
            "[1:a][2:a]amix=inputs=2:duration=first:normalize=0[aout]".into(),
        ],
        (true, false) => vec!["-map".into(), "0:v:0".into(), "-map".into(), "1:a:0".into()],
        (false, true) => vec!["-map".into(), "0:v:0".into(), "-map".into(), "1:a:0".into()],
        (false, false) => vec!["-map".into(), "0:v:0".into()],
    }
}

fn validate_probe(
    probe: &Value,
    frames: u64,
    width: u32,
    height: u32,
    fps: u32,
    expected_duration: f64,
    has_audio: bool,
) -> Result<()> {
    let streams = probe
        .get("streams")
        .and_then(Value::as_array)
        .context("FFprobe JSON has no streams array")?;
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let video = video.context("export has no video stream")?;
    if has_audio && audio.is_none() {
        bail!("export has no audio stream");
    }
    if video["width"].as_u64() != Some(u64::from(width))
        || video["height"].as_u64() != Some(u64::from(height))
    {
        bail!("FFprobe video dimensions do not match requested frame size");
    }
    let actual_frames = video["nb_frames"]
        .as_str()
        .and_then(|value| value.parse::<u64>().ok())
        .context("FFprobe video frame count is missing or invalid")?;
    if actual_frames != frames {
        bail!("FFprobe video frame count does not match submitted frame count");
    }
    if streams.len() != if has_audio { 2 } else { 1 } {
        bail!(
            "expected exactly one video and one audio stream, got {}",
            streams.len()
        );
    }
    let frame_rate = video["avg_frame_rate"]
        .as_str()
        .context("FFprobe average video frame rate is missing")?;
    if parse_rational(frame_rate).context("invalid FFprobe average video frame rate")?
        != f64::from(fps)
    {
        bail!("FFprobe video frame rate does not match requested FPS");
    }
    if video["codec_name"].as_str().is_none_or(str::is_empty) {
        bail!("FFprobe did not report video codec");
    }
    if video["codec_name"] != "h264" || video["pix_fmt"] != "yuv420p" {
        bail!("FFprobe codecs or pixel format differ from the configured MP4 output");
    }
    if has_audio && audio.is_some_and(|stream| stream["codec_name"] != "aac") {
        bail!("FFprobe audio codec differs from the configured MP4 output");
    }
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .context("FFprobe format duration is missing or invalid")?;
    if (duration - expected_duration).abs() > 1.0 / f64::from(fps) {
        bail!("FFprobe output duration differs from the requested frame sequence");
    }
    for stream in std::iter::once(video).chain(audio.into_iter()) {
        let stream_duration = stream["duration"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .context("FFprobe stream duration is missing or invalid")?;
        if (stream_duration - expected_duration).abs() > 1.0 / f64::from(fps) {
            bail!("FFprobe stream duration differs from the requested segment");
        }
        let start = stream["start_time"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .context("FFprobe stream start time is missing or invalid")?;
        if start.abs() > 1e-6 {
            bail!("FFprobe stream does not start at output time zero");
        }
    }
    Ok(())
}

fn parse_rational(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "real FFmpeg export integration: run explicitly for the current Windows toolchain"]
    fn real_automatic_ap_chart_export_contains_gameplay_ui() {
        real_ap_chart_export(false);
    }

    #[test]
    #[ignore = "real FFmpeg export integration: run explicitly for the current Windows toolchain"]
    fn real_optional_score_count_chart_export() {
        real_ap_chart_export(true);
    }

    fn real_ap_chart_export(animate_score_count: bool) {
        let tools = crate::export_mv::tests::tools()
            .expect("checked-in FFmpeg 8.1 pair for real export validation");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let output_directory = repository.join("target/acceptance-output");
        std::fs::create_dir_all(&output_directory).unwrap();
        let mut config = RenderConfig::default();
        config.engine = repository.join("TestingSuite/Next RUSH/engine/Next RUSH.zip");
        config.resources =
            repository.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        config.level = repository.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz");
        config.output = Some(output_directory.join(format!(
            "sono-overlay-scoring-{}.mp4",
            crate::performance::unix_time_ms()
        )));
        config.start_time = 0.0;
        config.duration = 4.0;
        config.fps = 60;
        config.width = 1920;
        config.height = 1080;
        config.level_options = vec![
            crate::render::LevelOptionValue {
                index: 1,
                value: 10.8,
            },
            crate::render::LevelOptionValue {
                index: 22,
                value: 1.0,
            },
        ];
        config.layers.particles = false;
        config.layers.sfx = false;
        config.layers.bgm = false;
        config.backend = crate::render::RenderBackend::Cpu;
        let (imported_ui, import_report) = crate::render_ui::RendererUiConfig::import_asset_pack(
            repository.join("assets/presentation-pack.json"),
            None,
        )
        .unwrap();
        assert_eq!(
            imported_ui.profile_name,
            "Sono-Renderer supplied presentation reference — Sono-Overlay reference HUD"
        );
        assert_eq!(
            imported_ui.assets[&imported_ui.fonts["combo"].glyph_assets["8"]],
            PathBuf::from("pack://combo.p8")
        );
        assert_eq!(
            imported_ui.assets[&imported_ui.fonts["combo-burst"].glyph_assets["8"]],
            PathBuf::from("pack://combo.b8")
        );
        assert_eq!(
            imported_ui.fonts["combo.glyph-family.n"].glyph_assets["8"],
            "combo.n8"
        );
        assert!(import_report
            .unconfigured_element_families
            .iter()
            .all(|family| family != "combo.glyph-family.n"));
        config.ui = if animate_score_count {
            crate::render_ui::RendererUiConfig::reference_default()
        } else {
            imported_ui
        };
        config.ui.animate_score_count = animate_score_count;
        config.ui.enabled = true;
        // Exercise the user-facing default: no scoring block and no manual
        // event file. The shared render session must prepare All Perfect data.
        config.ui.scoring = None;
        assert_eq!(
            config.ui.resolved_scoring_source(),
            crate::render_ui::UiScoringSource::SonoOverlay
        );
        let level = crate::formats::load_level(&config.level).unwrap();
        let forecast = crate::sono_overlay_scoring::simulate(
            &level,
            &crate::sono_overlay_scoring::ForecastConfig::default(),
        )
        .unwrap();
        let first_note = forecast
            .events
            .iter()
            .find(|event| !event.skill_activation)
            .unwrap();
        // The forecast intentionally uses a strict event-time boundary; the
        // event is visible on the first output sample after its timestamp.
        let first_note_state = forecast.state_at(first_note.time + 1.0 / 60.0).unwrap();
        assert!(first_note_state.score > 0.0);
        assert!(first_note_state.combo > 0);
        assert_eq!(first_note_state.judgment.as_deref(), Some("PERFECT"));
        let combo_46_event = forecast
            .frames
            .iter()
            .find(|frame| frame.combo == 46 && frame.source_entity.is_some())
            .expect("real Baumkuchen chart has an All Perfect combo-46 event");
        let combo_46_time = combo_46_event.time;
        // Export three consecutive late-pop frames where the Lua source
        // enables both Glow and Light. The full age sequence is captured by
        // the compositor test; this integration checks its real MP4 path.
        config.start_time = combo_46_time + 9.0 / 60.0;
        config.duration = if animate_score_count { 15.0 } else { 3.0 } / 60.0;
        if animate_score_count {
            let mut prepared = config.ui.clone();
            prepared.prepare_scoring_for_level(&level);
            let state = prepared
                .gameplay_state_at_with_frame_rate(config.start_time, 60.0)
                .unwrap()
                .unwrap();
            assert!(state.variables["display_score_count"] < state.score);
            println!(
                "Opt-in main score: displayed={}, authoritative={}",
                state.variables["display_score_count"], state.score
            );
            prepared.prepare_external_data().unwrap();
            let mut samples = Vec::new();
            for frame in [0, 7, 14] {
                let time = config.start_time + f64::from(frame) / 60.0;
                let mut gameplay = prepared
                    .gameplay_state_at_with_frame_rate(time, 60.0)
                    .unwrap()
                    .unwrap();
                samples.push(serde_json::json!({
                    "frame": frame, "time": time, "score": gameplay.score,
                    "displayed": gameplay.variables["display_score_count"]
                }));
                let frame_state = crate::render_ui::UiFrameState {
                    time,
                    frame_rate: 60.0,
                    gameplay: Some(gameplay.clone()),
                    ..Default::default()
                };
                let mut animated = vec![0; 1920 * 1080 * 3];
                crate::render_ui::render_gameplay_ui(
                    &mut animated,
                    1920,
                    1080,
                    &prepared,
                    &frame_state,
                )
                .unwrap();
                image::RgbImage::from_raw(1920, 1080, animated.clone())
                    .unwrap()
                    .save(output_directory.join(format!("score-count-preview-{frame}.png")))
                    .unwrap();
                let mut immediate = vec![0; animated.len()];
                let mut instant_config = prepared.clone();
                instant_config.animate_score_count = false;
                gameplay.variables.remove("display_score_count");
                let instant_state = crate::render_ui::UiFrameState {
                    gameplay: Some(gameplay),
                    ..frame_state
                };
                crate::render_ui::render_gameplay_ui(
                    &mut immediate,
                    1920,
                    1080,
                    &instant_config,
                    &instant_state,
                )
                .unwrap();
                assert_ne!(animated, immediate);
                image::RgbImage::from_raw(1920, 1080, immediate)
                    .unwrap()
                    .save(output_directory.join(format!("score-count-immediate-{frame}.png")))
                    .unwrap();
            }
            std::fs::write(
                output_directory.join("score-count-samples.json"),
                serde_json::to_vec_pretty(&samples).unwrap(),
            )
            .unwrap();
        }

        let report = export_config_with_tools_controlled(
            &config,
            Box::new(|_| {}),
            || Ok(tools.clone()),
            crate::export_control::ExportControl::default(),
        )
        .unwrap();
        assert_eq!(
            report.submitted_frames,
            if animate_score_count { 15 } else { 3 }
        );
        assert!(config.output.as_ref().unwrap().metadata().unwrap().len() > 0);
        assert_eq!(report.probe["streams"][0]["codec_name"], "h264");
        assert_eq!(report.probe["streams"][0]["width"], 1920);
        assert_eq!(report.probe["streams"][0]["height"], 1080);
        assert!(report
            .watch_execution_status
            .contains("Sono VM / interpreter selected"));
        println!(
            "Generated real gameplay-HUD MP4 around combo 46 at chart time {combo_46_time:.6}: {}",
            config.output.as_ref().unwrap().display(),
        );
    }

    struct SlowSink(Duration);

    impl Write for SlowSink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.0);
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct BrokenSink;

    impl Write for BrokenSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated FFmpeg stdin closure",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct PanickingSink;

    impl Write for PanickingSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            panic!("simulated output worker panic")
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct DiscardSink;

    impl Write for DiscardSink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // End-to-end seam uses the same exporter and installed tools without
    // asking unit-test executables to provision FFmpeg in target/debug/deps.
    #[test]
    fn single_watch_export_audio_pipeline_offsets_and_failure_cleanup() {
        let Some(tools) = crate::export_mv::tests::tools() else {
            return;
        };
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let effect = defaults.get("effects").unwrap();
        let assets = crate::formats::load_effect_assets_optional(&resources, effect)
            .unwrap()
            .unwrap();
        let binding = crate::offline::effect_clip_bindings(&package.watch)
            .unwrap()
            .into_iter()
            .find(|(_, name)| {
                assets
                    .clips
                    .get(name)
                    .and_then(Option::as_ref)
                    .is_some_and(|v| !v.is_empty())
            })
            .unwrap();
        let fixture = crate::export_mv::tests::fixture(&tools, false);
        let engine = fixture.path().join("engine.zip");
        let level_path = fixture.path().join("level.json");
        let music = fixture.path().join("bgm.wav");
        let mv = fixture.path().join("timestamped.mkv");
        fs::write(
            &music,
            crate::audio::wav_header(44100, 1, &[10000_i16.to_le_bytes(); 44100].concat()),
        )
        .unwrap();
        let ui = crate::render_ui::RendererUiConfig::default();
        // All calls are valid signatures. Immediate Play runs in initialize,
        // per-frame Play exercises post-preprocess events, and scheduled events
        // straddle nonzero export starts and the output end.
        for (backend, pipeline, start, offset, sfx, whole, validate) in [
            (
                crate::render::RenderBackend::Cpu,
                false,
                0.0,
                0.1,
                true,
                false,
                false,
            ),
            (
                crate::render::RenderBackend::Cpu,
                true,
                0.3,
                -0.15,
                true,
                false,
                false,
            ),
            (
                crate::render::RenderBackend::Wgpu,
                true,
                0.0,
                -0.1,
                true,
                false,
                false,
            ),
            (
                crate::render::RenderBackend::Wgpu,
                false,
                0.3,
                0.1,
                false,
                false,
                false,
            ),
            (
                crate::render::RenderBackend::Cpu,
                true,
                0.0,
                0.0,
                true,
                true,
                false,
            ),
            (
                crate::render::RenderBackend::Wgpu,
                true,
                0.0,
                0.0,
                true,
                true,
                false,
            ),
            (
                crate::render::RenderBackend::Cpu,
                false,
                0.0,
                0.0,
                true,
                false,
                true,
            ),
        ] {
            let mut presentation_reference = None;
            for percent in [100.0, 40.0, 0.0] {
                let watch: crate::watch::WatchData = serde_json::from_value(serde_json::json!({
                "archetypes":[{"name":"Note","hasInput":true,"spawnTime":{"index":0},"despawnTime":{"index":1},"preprocess":{"index":7},
                    "initialize": if whole { serde_json::Value::Null } else { serde_json::json!({"index":8}) },
                    "updateSequential": if whole { serde_json::Value::Null } else { serde_json::json!({"index":8}) }}],
                "skin":{"sprites":[]}, "effect":{"clips":[{"id":binding.0,"name":binding.1}]},
                "nodes":[{"value":0},{"value":0.8},{"value":binding.0},{"value":0.15},{"value":1.45},
                    {"func":"PlayScheduled","args":[2,3,0]}, {"func":"PlayScheduled","args":[2,4,0]},
                    {"func":"Execute","args":[5,6]}, {"func":"Play","args":[2,0]}]
            })).unwrap();
                let mut archive = zip::ZipWriter::new(fs::File::create(&engine).unwrap());
                for (name, value) in [
                    (
                        "engine.json",
                        serde_json::json!({"skin_name":skin,"effect_name":effect}),
                    ),
                    ("EngineConfiguration", serde_json::json!({"options":[]})),
                    ("EngineWatchData", serde_json::to_value(&watch).unwrap()),
                ] {
                    archive
                        .start_file(name, zip::write::SimpleFileOptions::default())
                        .unwrap();
                    archive
                        .write_all(&serde_json::to_vec(&value).unwrap())
                        .unwrap();
                }
                archive.finish().unwrap();
                let level: crate::formats::LevelData = serde_json::from_value(
                serde_json::json!({"bgmOffset":offset,"entities":[{"archetype":"Note","data":[]}]}),
            )
            .unwrap();
                fs::write(&level_path, serde_json::to_vec(&level).unwrap()).unwrap();
                let output = fixture.path().join("export.mp4");
                let phases = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let captured = phases.clone();
                let report = export_using_tools(
                    ExportRequest {
                        engine: &engine,
                        resources: &resources,
                        level: &level_path,
                        music: &music,
                        mv: Some(&mv),
                        mv_background: percent,
                        apply_engine_background_rotation: false,
                        resource_overrides: &crate::render::ResourceOverrides::default(),
                        output: &output,
                        start_time: start,
                        duration: 0.75,
                        whole_chart: whole,
                        fps: 12,
                        width: 64,
                        height: 36,
                        video_crf: 18,
                        video_preset: crate::render::VideoPreset::Medium,
                        trace_entity_id: None,
                        level_option_overrides: &[],
                        skin_name: None,
                        particles_enabled: false,
                        sfx_enabled: sfx,
                        bgm_enabled: true,
                        ui: &ui,
                        backend,
                        execution_mode: crate::sono_gcc::WatchExecutionMode::Interpreter,
                        watch_compatibility_policy:
                            crate::runtime::WatchCompatibilityPolicy::Permissive,
                        watch_random_seed: crate::runtime::DEFAULT_WATCH_RANDOM_SEED,
                        parallel_watch_updates: true,
                        watch_workers: None,
                        compile_sono_gcc_on_demand: true,
                        profile: true,
                        profile_frames: false,
                        validate_determinism: validate,
                        concurrent_sfx_prepass: true,
                        frame_pipeline: pipeline,
                    },
                    Box::new(move |event| captured.lock().unwrap().push(event.phase)),
                    || Ok(tools.clone()),
                )
                .unwrap();
                assert_eq!(report.performance.workload.execution_mode, "interpreter");
                assert_eq!(report.performance.frames, report.submitted_frames);
                assert_eq!(report.performance.pipeline_overlap.is_some(), pipeline);
                assert!(report.performance.profiling_detail_enabled);
                assert_eq!(
                    report.diagnostics.performance_record_id.as_deref(),
                    Some(report.performance.id.as_str())
                );
                let validation = &report.diagnostics.media.as_ref().unwrap().output_validation;
                assert!(validation.ffprobe_succeeded);
                assert_eq!(validation.submitted_frames, report.submitted_frames);
                assert_eq!(validation.width, Some(64));
                assert_eq!(validation.height, Some(36));
                for phase in [
                    "Preparing",
                    "Rendering",
                    "AudioMixing",
                    "AudioEncoding",
                    "Finalizing",
                ] {
                    assert!(report.performance.stage_timings.iter().any(|stage| {
                        stage.name == phase
                            && stage.semantics
                                == crate::performance::StageSemantics::ExclusiveWallSpan
                    }));
                }
                let invariants = (
                    report.submitted_frames,
                    report.timeline.watch_start.to_bits(),
                    report.timeline.duration.to_bits(),
                    report.sfx_event_sha1.clone(),
                    report
                        .frame_diagnostics
                        .iter()
                        .map(|f| {
                            (
                                f.global_frame_index,
                                f.runtime_update.map(f64::to_bits),
                                f.mv_media_pts.map(f64::to_bits),
                            )
                        })
                        .collect::<Vec<_>>(),
                );
                if let Some((old, hash)) = &presentation_reference {
                    assert_eq!(&invariants, old);
                    assert_ne!(&report.second_pass_hash, hash);
                } else {
                    presentation_reference = Some((invariants, report.second_pass_hash.clone()));
                }
                assert_eq!(report.watch_traversals, if validate { 2 } else { 1 });
                assert_eq!(report.event_pass_runtime_frames, 0);
                assert_eq!(report.rgb_spool_bytes, 0);
                assert!(!report.concurrent_sfx_prepass_used);
                assert_eq!(
                    report.stream_runtime_frames,
                    (report.timeline_end * 12.0).round() as u64
                );
                assert_eq!(
                    report.bgm_pcm_sample_frames,
                    Some(report.timeline.audio_sample_frames)
                );
                assert_eq!(report.probe["streams"][1]["start_time"], "0.000000");
                if validate {
                    assert_eq!(report.deterministic_frame_hashes_match, Some(true));
                }
                let range = FrameRange::new(start, report.timeline.duration, 12).unwrap();
                let reference = EventSession::new_configured(
                    &watch,
                    &[],
                    &serde_json::json!({"options":[]}),
                    &level,
                    &resources,
                    skin,
                    None,
                    Some(effect),
                    None,
                    64,
                    36,
                    12,
                    &[],
                    false,
                    false,
                    false,
                )
                .unwrap();
                let events =
                    collect_event_only_requests(reference, range, false, &AtomicU8::new(0))
                        .unwrap();
                assert_eq!(
                    report.sfx_event_sha1,
                    hash_audio_requests(&if sfx {
                        events.requests
                    } else {
                        empty_audio_requests()
                    })
                    .unwrap()
                );
                if sfx && !whole {
                    assert!(report.sfx_event_counts.audio_events > 0);
                }
                let phases = phases.lock().unwrap();
                assert!(phases.contains(&crate::export_progress::ExportPhase::AudioMixing));
                assert!(phases.contains(&crate::export_progress::ExportPhase::AudioEncoding));
                assert_eq!(
                    phases.last(),
                    Some(&crate::export_progress::ExportPhase::Complete)
                );
                assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("sono-")));
            }
        }

        // The same real renderer is also exercised through the persistent job
        // manager: a failed job must leave its sibling queued job runnable,
        // and the successful result is atomically published at the requested
        // output path without exposing the staging filename.
        let queue_store = fixture.path().join("render-jobs.json");
        let broken_engine = fixture.path().join("removed-engine.zip");
        fs::copy(&engine, &broken_engine).unwrap();
        let make_job_config = |engine_path: std::path::PathBuf, output: std::path::PathBuf| {
            let mut config = crate::render::RenderConfig::default();
            config.engine = engine_path;
            config.resources = resources.clone();
            config.level = level_path.clone();
            config.output = Some(output);
            config.duration = 2.0;
            config.fps = 12;
            config.width = 64;
            config.height = 36;
            config.backend = crate::render::RenderBackend::Cpu;
            config.job_manager.max_concurrent_jobs = 2;
            config.layers.particles = false;
            config.layers.sfx = false;
            config.layers.bgm = false;
            config.notifications.enabled = false;
            config
        };
        let manager = crate::job_manager::RenderJobManager::open_at(
            &queue_store,
            crate::job_manager::JobManagerConfig {
                max_concurrent_jobs: 2,
                ..crate::job_manager::JobManagerConfig::default()
            },
        )
        .unwrap();
        let profile_manager = crate::config_profiles::ConfigProfileManager::open_at(
            fixture.path().join("config-profiles.json"),
        )
        .unwrap();
        let failed_id = manager
            .enqueue(
                make_job_config(
                    broken_engine.clone(),
                    fixture.path().join("should-not-exist.mp4"),
                ),
                None,
            )
            .unwrap();
        fs::remove_file(&broken_engine).unwrap();
        let queued_output = fixture.path().join("job-manager-render.mp4");
        let vm_profile_config = make_job_config(engine.clone(), queued_output.clone());
        let vm_profile = profile_manager
            .create("Queue VM fixture", vm_profile_config.clone())
            .unwrap();
        let completed_id = manager
            .enqueue_from_profile(
                vm_profile_config.clone(),
                None,
                vm_profile.id.clone(),
                vm_profile.name.clone(),
            )
            .unwrap();
        let mut updated_vm_profile = vm_profile_config.clone();
        updated_vm_profile.duration = 4.0;
        profile_manager
            .update(&vm_profile.id, updated_vm_profile)
            .unwrap();
        let gcc_output = fixture.path().join("job-manager-gcc-render.mp4");
        let mut gcc_config = make_job_config(engine.clone(), gcc_output.clone());
        gcc_config.execution_mode = crate::sono_gcc::WatchExecutionMode::SonoGcc;
        let gcc_profile = profile_manager
            .create("Queue GCC fixture", gcc_config.clone())
            .unwrap();
        let gcc_id = manager
            .enqueue_from_profile(
                gcc_config,
                None,
                gcc_profile.id.clone(),
                gcc_profile.name.clone(),
            )
            .unwrap();
        profile_manager.delete(&gcc_profile.id).unwrap();
        manager.start().unwrap();
        manager.wait_until_idle().unwrap();
        let failed = manager.inspect(&failed_id).unwrap();
        let completed = manager.inspect(&completed_id).unwrap();
        let gcc = manager.inspect(&gcc_id).unwrap();
        assert_eq!(failed.state, crate::job_manager::JobState::Failed);
        assert!(failed.diagnostic.is_some());
        assert_eq!(completed.state, crate::job_manager::JobState::Completed);
        assert_eq!(gcc.state, crate::job_manager::JobState::Completed);
        assert_eq!(
            completed.profile_id.as_deref(),
            Some(vm_profile.id.as_str())
        );
        assert_eq!(completed.profile_name.as_deref(), Some("Queue VM fixture"));
        assert_eq!(completed.config.duration, vm_profile_config.duration);
        assert_eq!(gcc.profile_id.as_deref(), Some(gcc_profile.id.as_str()));
        assert_eq!(gcc.profile_name.as_deref(), Some("Queue GCC fixture"));
        assert!(profile_manager.get(&gcc_profile.id).is_err());
        assert!(queued_output.is_file());
        assert!(gcc_output.is_file());
        let (Some(vm_started), Some(vm_finished), Some(gcc_started), Some(gcc_finished)) = (
            completed.started_unix_time_ms,
            completed.finished_unix_time_ms,
            gcc.started_unix_time_ms,
            gcc.finished_unix_time_ms,
        ) else {
            panic!("completed integration jobs must retain their execution intervals");
        };
        assert!(
            vm_started < gcc_finished && gcc_started < vm_finished,
            "two-worker renderer jobs did not overlap: VM {vm_started}..{vm_finished}, GCC {gcc_started}..{gcc_finished}"
        );
        let vm_outcome = completed.outcome.as_ref().unwrap();
        let gcc_outcome = gcc.outcome.as_ref().unwrap();
        let performance_store = crate::performance::PerformanceStore::open_default().unwrap();
        let vm_performance = performance_store
            .get(vm_outcome.performance_record_id.as_deref().unwrap())
            .unwrap();
        let gcc_performance = performance_store
            .get(gcc_outcome.performance_record_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            gcc_performance.gcc_codegen_version.as_deref(),
            Some(crate::sono_gcc::compiler_identity().0)
        );
        assert_eq!(
            gcc_performance.gcc_runtime_abi.as_deref(),
            Some(crate::sono_gcc::compiler_identity().1)
        );
        let history_store = manager.history_store().unwrap();
        let failed_history = history_store
            .get(&crate::render_history::history_id(&failed_id, 1))
            .unwrap();
        let vm_history = history_store
            .get(&crate::render_history::history_id(&completed_id, 1))
            .unwrap();
        let gcc_history = history_store
            .get(&crate::render_history::history_id(&gcc_id, 1))
            .unwrap();
        assert_eq!(
            failed_history.status,
            crate::render_history::HistoryStatus::Failed
        );
        assert!(failed_history.error.is_some());
        assert!(failed_history.diagnostic_record_id.is_some());
        assert_eq!(
            vm_history.status,
            crate::render_history::HistoryStatus::Completed
        );
        assert_eq!(
            vm_history.profile_id.as_deref(),
            Some(vm_profile.id.as_str())
        );
        assert_eq!(
            vm_history.config_snapshot.duration,
            vm_profile_config.duration
        );
        assert_eq!(
            vm_history.performance_record_id.as_deref(),
            Some(vm_performance.id.as_str())
        );
        assert!(vm_history.diagnostic_record_id.is_some());
        assert_eq!(
            gcc_history.status,
            crate::render_history::HistoryStatus::Completed
        );
        assert_eq!(
            gcc_history.profile_id.as_deref(),
            Some(gcc_profile.id.as_str())
        );
        assert_eq!(
            gcc_history.performance_record_id.as_deref(),
            Some(gcc_performance.id.as_str())
        );
        assert_eq!(gcc_history.cache_state, gcc_performance.gcc_cache_state);
        assert!(vm_performance.queue_wait_time_ms.is_some());
        assert!(gcc_performance.queue_wait_time_ms.is_some());
        assert!(vm_performance.total_user_latency_ms.unwrap() >= vm_performance.execution_time_ms);
        assert!(
            gcc_performance.total_user_latency_ms.unwrap() >= gcc_performance.execution_time_ms
        );
        assert!(gcc_performance.gcc_cache_state.is_some());
        let backend_comparison =
            crate::performance::compare_records(&vm_performance, &gcc_performance);
        if gcc_performance.gcc_cache_state.as_deref() == Some("fallback") {
            assert!(!backend_comparison.comparable);
            assert!(backend_comparison
                .differences
                .iter()
                .any(|difference| difference == "effective Watch execution backend"));
        } else {
            assert!(backend_comparison.comparable);
        }
        assert_eq!(vm_outcome.rgb_sequence_sha1, gcc_outcome.rgb_sequence_sha1);
        assert_eq!(vm_outcome.sfx_event_sha1, gcc_outcome.sfx_event_sha1);
        assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("partial.mp4")));

        // A manifest exported from the real VM history record must preserve
        // resolved inputs after the source profile changes, then submit a new
        // ordinary queue job and link its own history result to the manifest.
        let manifest_path = fixture.path().join("vm-render.sonomanifest.json");
        let manifest_generation_started = Instant::now();
        let manifest = crate::render_manifest::RenderManifest::from_history_with_performance(
            &vm_history,
            Some(&vm_performance),
        )
        .unwrap();
        let manifest_generation_elapsed = manifest_generation_started.elapsed();
        let manifest_input_bytes = manifest
            .inputs
            .iter()
            .filter(|input| input.active)
            .filter_map(|input| input.size_bytes)
            .sum::<u64>();
        manifest.save_atomic(&manifest_path).unwrap();
        let manifest = crate::render_manifest::RenderManifest::load(&manifest_path).unwrap();
        let manifest_verification_started = Instant::now();
        let verification = manifest.verify(&Default::default()).unwrap();
        let manifest_verification_elapsed = manifest_verification_started.elapsed();
        eprintln!(
            "RENDER_MANIFEST_IO active_inputs={} input_mib={:.2} generate_ms={:.3} verify_ms={:.3}",
            manifest.inputs.iter().filter(|input| input.active).count(),
            manifest_input_bytes as f64 / (1024.0 * 1024.0),
            manifest_generation_elapsed.as_secs_f64() * 1000.0,
            manifest_verification_elapsed.as_secs_f64() * 1000.0,
        );
        assert!(verification.can_reproduce_exact_backend);
        assert!(verification
            .items
            .iter()
            .filter(|item| item.category
                == crate::render_manifest::DifferenceCategory::InputContent
                && item.affects_semantics)
            .all(|item| item.status == crate::render_manifest::VerificationStatus::Match));
        let reproduced_output = fixture.path().join("manifest-reproduction.mp4");
        let reproduced_config = manifest
            .build_reproduction_config(&Default::default(), &reproduced_output)
            .unwrap();
        assert_eq!(reproduced_config.duration, vm_profile_config.duration);
        let reproduction_job_id = manager
            .enqueue_with_metadata(
                reproduced_config,
                None,
                manifest
                    .provenance
                    .profile_id
                    .clone()
                    .zip(manifest.provenance.profile_name.clone()),
                crate::job_manager::RenderJobMetadata {
                    original_job_id: manifest.provenance.job_id.clone(),
                    rerendered_from_history_id: manifest.provenance.history_id.clone(),
                    reproduced_from_manifest_sha256: Some(manifest.manifest_sha256.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        manager.wait_until_idle().unwrap();
        let reproduced = manager.inspect(&reproduction_job_id).unwrap();
        assert_eq!(reproduced.state, crate::job_manager::JobState::Completed);
        assert!(reproduced_output.is_file());
        assert_eq!(
            reproduced.reproduced_from_manifest_sha256.as_deref(),
            Some(manifest.manifest_sha256.as_str())
        );
        let reproduced_history = history_store
            .get(&crate::render_history::history_id(&reproduction_job_id, 1))
            .unwrap();
        assert_eq!(
            reproduced_history
                .reproduced_from_manifest_sha256
                .as_deref(),
            Some(manifest.manifest_sha256.as_str())
        );
        assert_eq!(
            reproduced_history.rgb_sequence_sha1,
            vm_history.rgb_sequence_sha1
        );
        assert_eq!(reproduced_history.sfx_event_sha1, vm_history.sfx_event_sha1);

        // Restore the missing input and retry the exact saved configuration.
        // This exercises a real failed attempt followed by a successful retry.
        fs::copy(&engine, &broken_engine).unwrap();
        manager.retry(&failed_id).unwrap();
        manager.wait_until_idle().unwrap();
        let retried = manager.inspect(&failed_id).unwrap();
        assert_eq!(retried.state, crate::job_manager::JobState::Completed);
        assert_eq!(retried.attempt, 2);
        assert_eq!(retried.previous_attempt_diagnostics.len(), 1);
        assert!(fixture.path().join("should-not-exist.mp4").is_file());
        let retry_history = history_store
            .get(&crate::render_history::history_id(&failed_id, 2))
            .unwrap();
        assert_eq!(
            retry_history.status,
            crate::render_history::HistoryStatus::Completed
        );
        assert_eq!(
            retry_history.retried_from_history_id.as_deref(),
            Some(crate::render_history::history_id(&failed_id, 1).as_str())
        );

        // Exercise the actual benchmark runner and its opt-in detailed-profile
        // overhead estimate on this short real-render fixture. The assertion
        // checks record shape only; wall-time thresholds would be machine-noise
        // sensitive and do not belong in a correctness integration test.
        let mut telemetry_config = make_job_config(
            engine.clone(),
            fixture.path().join("telemetry-benchmark.mp4"),
        );
        telemetry_config.duration = 0.25;
        telemetry_config.frame_pipeline = false;
        let telemetry_benchmark =
            crate::performance::run_benchmark(telemetry_config, 0, 1, false, true).unwrap();
        let overhead = telemetry_benchmark.telemetry_overhead.as_ref().unwrap();
        assert_eq!(overhead.detailed_samples.len(), 1);
        assert_eq!(telemetry_benchmark.backends[0].samples.len(), 1);
        eprintln!(
            "PERF_LAB_OVERHEAD lightweight={:.3}ms detailed={:.3}ms delta={:+.3}ms ({})",
            overhead.lightweight_median_ms,
            overhead.detailed_median_ms,
            overhead.overhead_ms,
            overhead.classification
        );

        // Cancel a live long render through the job manager. The final path
        // must remain absent and the partial staging file must be cleaned up.
        let cancelled_output = fixture.path().join("job-manager-cancelled.mp4");
        let mut cancellable_config = make_job_config(engine.clone(), cancelled_output.clone());
        cancellable_config.duration = 30.0;
        let cancelled_id = manager.enqueue(cancellable_config, None).unwrap();
        let activation_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let state = manager.inspect(&cancelled_id).unwrap().state;
            if state.is_active() {
                break;
            }
            assert_eq!(state, crate::job_manager::JobState::Queued);
            assert!(
                std::time::Instant::now() < activation_deadline,
                "cancel fixture never became active"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        manager.cancel(&cancelled_id).unwrap();
        manager.wait_until_idle().unwrap();
        let cancelled = manager.inspect(&cancelled_id).unwrap();
        assert_eq!(cancelled.state, crate::job_manager::JobState::Cancelled);
        assert!(!cancelled_output.exists());
        assert!(!fixture
            .path()
            .join(format!(
                ".job-manager-cancelled.{}.partial.mp4",
                cancelled.id
            ))
            .exists());
        drop(manager);

        // Keep the short, non-divisor frame-rate case covered end to end. The
        // default MP4 track timescale rounded this 27-frame 239 FPS clip to
        // 240 FPS and correctly failed the strict FFprobe validation.
        let short_239_output = fixture.path().join("short-239-fps.mp4");
        let short_239_report = export_using_tools(
            ExportRequest {
                engine: &engine,
                resources: &resources,
                level: &level_path,
                music: &music,
                mv: None,
                mv_background: 100.0,
                apply_engine_background_rotation: false,
                resource_overrides: &crate::render::ResourceOverrides::default(),
                output: &short_239_output,
                start_time: 0.0,
                duration: 0.11,
                whole_chart: false,
                fps: 239,
                width: 16,
                height: 24,
                // Exercise non-default shared encoder settings through the
                // actual FFmpeg invocation on this small output.
                video_crf: 22,
                video_preset: crate::render::VideoPreset::Fast,
                trace_entity_id: None,
                level_option_overrides: &[],
                skin_name: None,
                particles_enabled: false,
                sfx_enabled: false,
                bgm_enabled: false,
                ui: &ui,
                backend: crate::render::RenderBackend::Cpu,
                execution_mode: crate::sono_gcc::WatchExecutionMode::Interpreter,
                watch_compatibility_policy: crate::runtime::WatchCompatibilityPolicy::Permissive,
                watch_random_seed: crate::runtime::DEFAULT_WATCH_RANDOM_SEED,
                parallel_watch_updates: true,
                watch_workers: None,
                compile_sono_gcc_on_demand: true,
                profile: false,
                profile_frames: false,
                validate_determinism: false,
                concurrent_sfx_prepass: false,
                frame_pipeline: true,
            },
            Box::new(|_| {}),
            || Ok(tools.clone()),
        )
        .unwrap();
        assert_eq!(short_239_report.submitted_frames, 27);
        assert!(short_239_output.is_file());

        for phase in [
            crate::export_progress::ExportPhase::Rendering,
            crate::export_progress::ExportPhase::AudioEncoding,
            crate::export_progress::ExportPhase::Finalizing,
        ] {
            let output = fixture.path().join("cancelled.mp4");
            fs::write(&output, b"prior output").unwrap();
            let control = crate::export_control::ExportControl::default();
            let cancel = control.clone();
            let result = export_using_tools_controlled(
                ExportRequest {
                    engine: &engine,
                    resources: &resources,
                    level: &level_path,
                    music: &music,
                    mv: Some(&mv),
                    mv_background: 40.0,
                    apply_engine_background_rotation: false,
                    resource_overrides: &Default::default(),
                    output: &output,
                    start_time: 0.0,
                    duration: 0.5,
                    whole_chart: false,
                    fps: 12,
                    width: 64,
                    height: 36,
                    video_crf: 18,
                    video_preset: crate::render::VideoPreset::Medium,
                    trace_entity_id: None,
                    level_option_overrides: &[],
                    skin_name: None,
                    particles_enabled: false,
                    sfx_enabled: true,
                    bgm_enabled: true,
                    ui: &ui,
                    backend: crate::render::RenderBackend::Cpu,
                    execution_mode: crate::sono_gcc::WatchExecutionMode::Interpreter,
                    watch_compatibility_policy:
                        crate::runtime::WatchCompatibilityPolicy::Permissive,
                    watch_random_seed: crate::runtime::DEFAULT_WATCH_RANDOM_SEED,
                    parallel_watch_updates: true,
                    watch_workers: None,
                    compile_sono_gcc_on_demand: true,
                    profile: false,
                    profile_frames: false,
                    validate_determinism: false,
                    concurrent_sfx_prepass: false,
                    frame_pipeline: true,
                },
                Box::new(move |event| {
                    if event.phase == phase {
                        cancel.cancel();
                    }
                }),
                || Ok(tools.clone()),
                control,
            );
            assert!(result.unwrap_err().is::<crate::export_control::Cancelled>());
            assert_eq!(fs::read(&output).unwrap(), b"prior output");
            assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("sono-")));
        }
        // Failure after video+audio construction (probe) must preserve a prior
        // output and clean every staged media file, never publish partial data.
        let output = fixture.path().join("preserved.mp4");
        fs::write(&output, b"prior output").unwrap();
        let mut broken_tools = tools.clone();
        broken_tools.ffprobe = fixture.path().join("missing-ffprobe");
        let result = export_using_tools(
            ExportRequest {
                engine: &engine,
                resources: &resources,
                level: &level_path,
                music: &music,
                mv: None,
                mv_background: 100.0,
                apply_engine_background_rotation: false,
                resource_overrides: &crate::render::ResourceOverrides::default(),
                output: &output,
                start_time: 0.0,
                duration: 0.25,
                whole_chart: false,
                fps: 12,
                width: 64,
                height: 36,
                video_crf: 18,
                video_preset: crate::render::VideoPreset::Medium,
                trace_entity_id: None,
                level_option_overrides: &[],
                skin_name: None,
                particles_enabled: false,
                sfx_enabled: true,
                bgm_enabled: false,
                ui: &ui,
                backend: crate::render::RenderBackend::Cpu,
                execution_mode: crate::sono_gcc::WatchExecutionMode::Interpreter,
                watch_compatibility_policy: crate::runtime::WatchCompatibilityPolicy::Permissive,
                watch_random_seed: crate::runtime::DEFAULT_WATCH_RANDOM_SEED,
                parallel_watch_updates: true,
                watch_workers: None,
                compile_sono_gcc_on_demand: true,
                profile: false,
                profile_frames: false,
                validate_determinism: false,
                concurrent_sfx_prepass: false,
                frame_pipeline: true,
            },
            Box::new(|_| {}),
            || Ok(broken_tools),
        );
        assert!(result.unwrap_err().to_string().contains("FFprobe"));
        assert_eq!(fs::read(&output).unwrap(), b"prior output");
        assert!(fs::read_dir(fixture.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("sono-")));
    }

    #[test]
    fn no_sfx_production_export_has_no_prepass() {
        assert_eq!(prepass_plan(false, false), PrepassPlan::None);
        assert_eq!(prepass_plan(true, false), PrepassPlan::None);
        assert_eq!(prepass_plan(false, true), PrepassPlan::FullValidation);
        assert_eq!(prepass_plan(true, true), PrepassPlan::FullValidation);
    }

    #[test]
    fn explicit_determinism_validation_compares_draw_and_rgb_hashes() {
        let diagnostic = |draw: &str, rgb: &str| FrameDiagnostic {
            mv_media_pts: None,
            output_local_frame_index: 0,
            global_frame_index: 0,
            requested_timeline_time: 0.0,
            runtime_timeline: None,
            runtime_update: [0.0; 4],
            runtime_update_after_callbacks: [0.0; 4],
            timescale: 1.0,
            callback_count: 0,
            vm_evaluations: 0,
            sono_gcc_regions_executed: 0,
            sono_gcc_vm_to_native_cut_calls: 0,
            runtime_frame_count: 0,
            runtime_entity_count: 0,
            active_entity_count: 0,
            draw_count: 0,
            raw_draw_sha1: draw.to_owned(),
            rgb_sha1: rgb.to_owned(),
            target_entity_draws: Vec::new(),
            audio_events: Vec::new(),
            scheduled_effects: Vec::new(),
            scheduled_looped_effects: Vec::new(),
            scheduled_looped_effect_stops: Vec::new(),
        };
        let first = [diagnostic("draw-a", "rgb-a")];
        assert!(compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-a", "rgb-a")]
        ));
        assert!(!compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-b", "rgb-a")]
        ));
        assert!(!compare_frame_diagnostics(
            &first,
            &[diagnostic("draw-a", "rgb-b")]
        ));
        assert!(!compare_frame_diagnostics(&first, &[]));
    }

    #[test]
    fn streaming_without_preflight_matches_sequential_baumkuchen_frame() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let make_session = || {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                defaults.get("skins").unwrap(),
                defaults.get("backgrounds").map(String::as_str),
                defaults.get("effects").map(String::as_str),
                defaults.get("particles").map(String::as_str),
                320,
                180,
                12,
                &[(1, 10.8), (22, 1.0)],
                false,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                0.25,
                crate::render::RenderBackend::Wgpu,
                true,
                false,
            )
            .unwrap()
        };
        let expected_frame = make_session().render_global_frame(0).unwrap();
        let session = make_session();
        let range = FrameRange {
            first_index: 0,
            frame_count: 1,
            fps: 12,
        };
        let mut output = Vec::new();
        let frames = stream_frames(
            session,
            range,
            &mut output,
            None,
            &package.watch,
            None,
            None,
            "FFmpeg handoff",
            "FFmpeg final input flush/close",
            None,
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].rgb_sha1,
            crate::offline::hash_rgb(&expected_frame.rgb)
        );
        assert_eq!(output.len(), 320 * 180 * 3);
    }

    #[test]
    fn mv_pipeline_matches_sequential_and_cpu_wgpu_share_composition() {
        let Some(tools) = crate::export_mv::tests::tools() else {
            return;
        };
        let fixture = crate::export_mv::tests::fixture(&tools, false);
        let mv = fixture.path().join("timestamped.mkv");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let range = FrameRange::new(0.0, 1.5, 8).unwrap();
        let timeline = crate::export_timeline::ExportTimeline::new(0.0, range, -0.125).unwrap();
        let make = |backend, mv_enabled| {
            let mut session = FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                defaults.get("skins").unwrap(),
                defaults.get("backgrounds").map(String::as_str),
                defaults.get("effects").map(String::as_str),
                defaults.get("particles").map(String::as_str),
                64,
                36,
                8,
                &[(1, 10.8), (22, 1.0)],
                true,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                1.5,
                backend,
                false,
                false,
            )
            .unwrap();
            if mv_enabled {
                session.configure_mv(&tools, &mv, timeline).unwrap();
            }
            session
        };
        let mut backend_rgbs = Vec::new();
        for backend in [
            crate::render::RenderBackend::Cpu,
            crate::render::RenderBackend::Wgpu,
        ] {
            let mut sequential_rgb = Vec::new();
            let expected = stream_frames(
                make(backend, true),
                range,
                &mut sequential_rgb,
                None,
                &package.watch,
                None,
                None,
                "test handoff",
                "test close",
                None,
            )
            .unwrap();
            assert_eq!(
                expected.iter().map(|f| f.mv_media_pts).collect::<Vec<_>>(),
                vec![
                    None,
                    Some(0.0),
                    Some(0.0),
                    Some(0.25),
                    Some(0.25),
                    Some(0.5),
                    Some(0.5),
                    Some(0.75),
                    Some(0.75),
                    None,
                    None,
                    None
                ]
            );
            let mut pipeline_rgb = Vec::new();
            let output = stream_frames_pipelined(
                make(backend, true),
                range,
                &mut pipeline_rgb,
                Some(&expected),
                &package.watch,
                None,
                false,
            )
            .unwrap();
            assert_eq!(
                hash_audio_requests(&output.audio_requests).unwrap(),
                hash_audio_requests(&collect_audio_requests(&expected)).unwrap()
            );
            assert_eq!(pipeline_rgb, sequential_rgb);
            assert!(compare_frame_diagnostics(&expected, &output.diagnostics));
            let mut without_mv = Vec::new();
            stream_frames(
                make(backend, false),
                range,
                &mut without_mv,
                None,
                &package.watch,
                None,
                None,
                "test handoff",
                "test close",
                None,
            )
            .unwrap();
            let bytes = 64 * 36 * 3;
            for n in [0, 9, 10, 11] {
                assert_eq!(
                    sequential_rgb[n * bytes..(n + 1) * bytes],
                    without_mv[n * bytes..(n + 1) * bytes]
                );
            }
            assert_ne!(
                sequential_rgb[bytes..9 * bytes],
                without_mv[bytes..9 * bytes]
            );
            backend_rgbs.push(sequential_rgb);
        }
        let max_error = backend_rgbs[0]
            .iter()
            .zip(&backend_rgbs[1])
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            max_error <= 2,
            "CPU/WGPU MV composition max channel difference: {max_error}"
        );
        // A closed output must cancel the bounded pipeline and release its
        // decoder, even with prepared MV snapshots still in the queue.
        let error = stream_frames_pipelined(
            make(crate::render::RenderBackend::Cpu, true),
            range,
            BrokenSink,
            None,
            &package.watch,
            None,
            false,
        )
        .err()
        .unwrap();
        assert!(error
            .chain()
            .any(|e| e.to_string().contains("simulated FFmpeg stdin closure")));
    }

    #[test]
    fn wgpu_pipeline_matches_sequential_rgb_with_determinism_validation() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let make_session = || {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                defaults.get("skins").unwrap(),
                defaults.get("backgrounds").map(String::as_str),
                defaults.get("effects").map(String::as_str),
                defaults.get("particles").map(String::as_str),
                64,
                36,
                12,
                &[(1, 10.8), (22, 1.0)],
                true,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                2.0,
                crate::render::RenderBackend::Wgpu,
                false,
                false,
            )
            .unwrap()
        };
        let range = FrameRange {
            first_index: 0,
            frame_count: 24,
            fps: 12,
        };
        let mut sequential_rgb = Vec::new();
        let expected = stream_frames(
            make_session(),
            range,
            &mut sequential_rgb,
            None,
            &package.watch,
            None,
            None,
            "FFmpeg handoff",
            "FFmpeg final input flush/close",
            None,
        )
        .unwrap();
        let mut pipeline_rgb = Vec::new();
        let output = stream_frames_pipelined(
            make_session(),
            range,
            &mut pipeline_rgb,
            Some(&expected),
            &package.watch,
            None,
            false,
        )
        .unwrap();
        assert_eq!(pipeline_rgb, sequential_rgb);
        assert!(compare_frame_diagnostics(&expected, &output.diagnostics));
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.global_frame_index)
                .collect::<Vec<_>>(),
            (0..range.frame_count).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn bounded_pipeline_matches_sequential_hashes_and_preserves_fifo() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let make_session = |ui| {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                ui,
                0.0,
                10.0,
                crate::render::RenderBackend::Cpu,
                false,
                false,
            )
            .unwrap()
        };
        let range = FrameRange {
            first_index: 0,
            frame_count: 24,
            fps: 12,
        };
        let mut sequential = make_session(crate::render_ui::RendererUiConfig::default());
        let sequential_frames: Vec<_> = (0..range.frame_count)
            .map(|local| {
                let global = range.index(local).unwrap();
                let frame = sequential.render_global_frame(global).unwrap();
                (global, hash_rgb(&frame.rgb), frame.report.vm_evaluations)
            })
            .collect();
        let mut output = Vec::new();
        let pipelined = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            range,
            &mut output,
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        let pipelined_frames: Vec<_> = pipelined
            .diagnostics
            .iter()
            .map(|frame| {
                (
                    frame.global_frame_index,
                    frame.rgb_sha1.clone(),
                    frame.vm_evaluations,
                )
            })
            .collect();
        assert_eq!(sequential_frames, pipelined_frames);
        assert_eq!(
            pipelined
                .diagnostics
                .iter()
                .map(|frame| frame.output_local_frame_index)
                .collect::<Vec<_>>(),
            (0..range.frame_count).collect::<Vec<_>>()
        );
        assert!(pipelined.stats.queue_high_water > 0);
        assert!(pipelined.stats.queue_high_water <= pipelined.stats.queue_capacity);
        assert_eq!(output.len(), 64 * 36 * 3 * range.frame_count as usize);

        let short_range = FrameRange {
            frame_count: 8,
            ..range
        };
        let slowed = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            // The consumer must be slower than debug Watch evaluation for
            // this control to exercise backpressure rather than just FIFO.
            SlowSink(Duration::from_millis(200)),
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        assert_eq!(slowed.stats.queue_high_water, FRAME_QUEUE_CAPACITY);
        assert!(slowed.stats.producer_blocked_ms > 0.0);

        let broken = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            BrokenSink,
            None,
            &package.watch,
            None,
            false,
        );
        assert!(format!("{:#}", broken.err().unwrap()).contains("simulated FFmpeg stdin closure"));

        let panicked = stream_frames_pipelined(
            make_session(crate::render_ui::RendererUiConfig::default()),
            short_range,
            PanickingSink,
            None,
            &package.watch,
            None,
            false,
        );
        assert!(panicked
            .err()
            .unwrap()
            .to_string()
            .contains("worker panicked"));

        let mut failing_ui = crate::render_ui::RendererUiConfig::default();
        failing_ui.mode = crate::render_ui::GameplayUiMode::Legacy;
        failing_ui.enabled = true;
        failing_ui.primary_metric.provider = crate::render_ui::UiValueProvider::EngineMetric {
            key: "missing".to_owned(),
        };
        let producer_failure = stream_frames_pipelined(
            make_session(failing_ui),
            FrameRange {
                frame_count: 1,
                ..range
            },
            Vec::new(),
            None,
            &package.watch,
            None,
            false,
        );
        assert!(format!("{:#}", producer_failure.err().unwrap())
            .contains("no authoritative Watch source"));
    }

    #[test]
    fn baumkuchen_1080p_pipeline_matches_sequential_rgb_and_known_sfx_workload() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let range = FrameRange {
            first_index: 0,
            frame_count: 120,
            fps: 60,
        };
        let make_session = || {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                1920,
                1080,
                60,
                &options,
                true,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                2.0,
                crate::render::RenderBackend::Wgpu,
                true,
                false,
            )
            .unwrap()
        };
        let mut sequential = make_session();
        let sequential_hashes: Vec<_> = (0..range.frame_count)
            .map(|local| {
                let global = range.index(local).unwrap();
                let frame = sequential.render_global_frame(global).unwrap();
                (global, crate::offline::hash_rgb(&frame.rgb))
            })
            .collect();
        let session = make_session();
        let output = stream_frames_pipelined(
            session,
            range,
            DiscardSink,
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        let pipelined_hashes: Vec<_> = output
            .diagnostics
            .iter()
            .map(|frame| (frame.global_frame_index, frame.rgb_sha1.clone()))
            .collect();
        assert_eq!(pipelined_hashes, sequential_hashes);
        // Re-pinned after correcting Sonolus Pre/Post return semantics: the
        // compiled scheduler now preserves one additional loop interval.
        // Independent sequential and pipelined traversals must still agree.
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.runtime_frame_count)
                .sum::<u64>(),
            120
        );
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.callback_count as u64)
                .sum::<u64>(),
            10_890
        );
        // Evaluating all children of variadic arithmetic adds seven reached
        // nodes in this fixture; both sequential and pipelined traversals agree.
        assert_eq!(
            output
                .diagnostics
                .iter()
                .map(|frame| frame.vm_evaluations)
                .sum::<u64>(),
            12_871_006
        );

        let event_session = EventSession::new_configured(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &resources,
            skin,
            background,
            effects,
            particles,
            1920,
            1080,
            60,
            &options,
            true,
            true,
            false,
        )
        .unwrap();
        let events =
            collect_event_only_requests(event_session, range, true, &AtomicU8::new(0)).unwrap();
        assert_eq!(
            hash_audio_requests(&events.requests).unwrap(),
            "2d8c261fbba9a28a77842a05665720e655d73f0f"
        );
        assert_eq!(
            count_audio_requests(&events.requests),
            SfxEventCounts {
                audio_events: 0,
                scheduled_effects: 615,
                loop_starts: 90,
                loop_stops: 90,
            }
        );
        // The authoritative serial Watch producer supplies exactly this timeline.
        assert_eq!(
            hash_audio_requests(&output.audio_requests).unwrap(),
            hash_audio_requests(&events.requests).unwrap()
        );
        assert_eq!(
            hash_audio_requests(&collect_audio_requests(&output.diagnostics)).unwrap(),
            hash_audio_requests(&output.audio_requests).unwrap()
        );
        assert_eq!(events.runtime_frames, 120);
        assert_eq!(events.callbacks, 10_890);
        // Variadic arithmetic now evaluates all operands, reaching seven
        // additional child nodes in this Watch interval.
        assert_eq!(events.evaluations, 12_871_006);
        assert_eq!(events.checkpoints.len(), 1);
        let checkpoint = &events.checkpoints[0];
        assert_eq!((checkpoint.watch_start, checkpoint.watch_end), (0.0, 2.0));
        assert_eq!(checkpoint.runtime_frames, 120);
        assert_eq!(checkpoint.callbacks, events.callbacks);
        assert_eq!(checkpoint.evaluations, events.evaluations);
        assert_eq!(checkpoint.sfx_events, [0, 615, 90, 90]);
        assert_eq!(
            checkpoint.callback_stages.values().sum::<u64>(),
            events.callbacks
        );
        assert!(checkpoint.draw_commands > 0);
        assert!(checkpoint.memory_entries_copied > 0);
    }

    #[test]
    fn authoritative_pipeline_baumkuchen_events_match_reference_and_sequential_frames() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            crate::formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = crate::formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let make_event_session = || {
            EventSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                false,
                false,
            )
            .unwrap()
        };
        let make_frame_session = || {
            FrameSession::new_configured(
                &package.watch,
                &package.rom,
                &package.configuration,
                &level,
                &resources,
                skin,
                background,
                effects,
                particles,
                64,
                36,
                12,
                &options,
                true,
                crate::render_ui::RendererUiConfig::default(),
                0.0,
                10.0,
                crate::render::RenderBackend::Cpu,
                false,
                false,
            )
            .unwrap()
        };
        let range = FrameRange {
            first_index: 0,
            frame_count: 120,
            fps: 12,
        };
        let sequential_events =
            collect_event_only_requests(make_event_session(), range, false, &AtomicU8::new(0))
                .unwrap();
        let mut sequential_session = make_frame_session();
        let sequential_hashes: Vec<_> = (0..range.frame_count)
            .map(|index| {
                let frame = sequential_session
                    .render_global_frame(range.index(index).unwrap())
                    .unwrap();
                hash_rgb(&frame.rgb)
            })
            .collect();

        let mut rgb = Vec::new();
        let concurrent = stream_frames_pipelined(
            make_frame_session(),
            range,
            &mut rgb,
            None,
            &package.watch,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            hash_audio_requests(&sequential_events.requests).unwrap(),
            hash_audio_requests(&concurrent.audio_requests).unwrap()
        );
        assert_eq!(
            count_audio_requests(&sequential_events.requests),
            count_audio_requests(&concurrent.audio_requests)
        );
        assert!(
            count_audio_requests(&concurrent.audio_requests).audio_events
                + count_audio_requests(&concurrent.audio_requests).scheduled_effects
                + count_audio_requests(&concurrent.audio_requests).loop_starts
                + count_audio_requests(&concurrent.audio_requests).loop_stops
                > 0
        );
        assert_eq!(
            sequential_events.runtime_frames,
            concurrent
                .diagnostics
                .iter()
                .map(|f| f.runtime_frame_count)
                .sum::<u64>()
        );
        assert_eq!(
            sequential_events.callbacks,
            concurrent
                .diagnostics
                .iter()
                .map(|f| f.callback_count as u64)
                .sum::<u64>()
        );
        assert_eq!(
            sequential_events.evaluations,
            concurrent
                .diagnostics
                .iter()
                .map(|f| f.vm_evaluations)
                .sum::<u64>()
        );
        let concurrent_hashes: Vec<_> = concurrent
            .diagnostics
            .iter()
            .map(|frame| frame.rgb_sha1.clone())
            .collect();
        assert_eq!(sequential_hashes, concurrent_hashes);
        assert_eq!(
            hash_sequence(&sequential_hashes),
            hash_sequence(&concurrent_hashes)
        );
        assert_eq!(rgb.len(), 64 * 36 * 3 * 120);
    }

    #[test]
    fn bgm_preparation_inserts_samples_instead_of_shifting_timestamps() {
        let timeline = crate::export_timeline::ExportTimeline::new(
            0.0,
            FrameRange::new(0.0, 1.0, 12).unwrap(),
            -0.05,
        )
        .unwrap();
        let filter = timeline.bgm_filter();
        assert!(filter.contains("adelay=2205S:all=1"));
        assert!(filter.contains("apad=whole_len=44100,atrim=end_sample=44100"));
        assert!(filter.ends_with("asetpts=N/SR/TB"));
    }

    #[test]
    fn bgm_is_mapped_alone_without_sfx_and_mixed_when_sfx_exists() {
        let without_sfx = audio_map_args(true, false).join(" ");
        assert!(without_sfx.contains("1:a:0"));
        assert!(!without_sfx.contains("atrim")); // BGM is already zero-based PCM.

        let with_sfx = audio_map_args(true, true).join(" ");
        assert!(with_sfx.contains("[1:a]"));
        assert!(with_sfx.contains("[2:a]"));
        assert!(with_sfx.contains("amix=inputs=2"));
        assert!(with_sfx.contains("normalize=0"));

        let sfx_only = audio_map_args(false, true).join(" ");
        assert!(sfx_only.contains("1:a:0"));
        assert!(!sfx_only.contains("atrim"));
        assert_eq!(audio_map_args(false, false), ["-map", "0:v:0"]);
    }
}
