//! Shared configuration and entry points for CLI, preview, and video rendering.

use crate::{
    formats, offline::FrameSession, offline::RenderedFrame, render_ui::RendererUiConfig,
    video_export,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const CONFIG_FILE_FORMAT: &str = "sono-renderer-config";
const CONFIG_FILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LevelOptionValue {
    pub index: usize,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderLayers {
    pub particles: bool,
    pub sfx: bool,
    pub bgm: bool,
}

impl Default for RenderLayers {
    fn default() -> Self {
        Self {
            particles: true,
            sfx: true,
            bgm: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderConfig {
    pub engine: PathBuf,
    pub resources: PathBuf,
    pub level: PathBuf,
    #[serde(default)]
    pub skin: Option<String>,
    #[serde(default)]
    pub music: Option<PathBuf>,
    /// Explicit video background, sampled on the canonical media timeline.
    #[serde(default)]
    pub mv: Option<PathBuf>,
    #[serde(default = "default_mv_enabled")]
    pub mv_enabled: bool,
    /// Sono-Renderer presentation brightness; does not change media timing.
    #[serde(default = "default_mv_background")]
    pub mv_background: f64,
    #[serde(default)]
    pub resource_overrides: ResourceOverrides,
    #[serde(default)]
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub start_time: f64,
    #[serde(default = "default_duration")]
    pub duration: f64,
    /// Infer an end from evaluated Watch schedules and audio bounds.
    #[serde(default)]
    pub whole_chart: bool,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    /// x264 constant-rate-factor quality (0 is lossless, 51 lowest quality).
    #[serde(default = "default_video_crf")]
    pub video_crf: u8,
    #[serde(default)]
    pub video_preset: VideoPreset,
    #[serde(default)]
    pub level_options: Vec<LevelOptionValue>,
    #[serde(default)]
    pub trace_entity_id: Option<usize>,
    #[serde(default)]
    pub layers: RenderLayers,
    #[serde(default)]
    pub ui: RendererUiConfig,
    /// Renderer-owned watermark configuration/profile. The master switch
    /// controls both fixed attribution and the subordinate moving mark.
    #[serde(default)]
    pub watermark: crate::watermark::WatermarkConfig,
    /// Optional desktop and Discord lifecycle notifications.
    #[serde(
        default,
        deserialize_with = "crate::notifications::deserialize_tolerant"
    )]
    pub notifications: crate::notifications::NotificationConfig,
    /// Shared CLI/desktop render-job scheduler configuration.
    #[serde(default)]
    pub job_manager: crate::job_config::JobManagerConfig,
    #[serde(default)]
    pub backend: RenderBackend,
    #[serde(default)]
    pub execution_mode: crate::sono_gcc::WatchExecutionMode,
    /// How Watch handles operations with intentionally approximate client
    /// compatibility semantics. Older configs deserialize as permissive.
    #[serde(default)]
    pub watch_compatibility_policy: crate::runtime::WatchCompatibilityPolicy,
    /// Stable seed for the renderer's deterministic Random approximation.
    #[serde(default = "default_watch_random_seed")]
    pub watch_random_seed: u64,
    /// Run independent Watch UpdateParallel callbacks on persistent workers.
    #[serde(default = "default_parallel_watch_updates")]
    pub parallel_watch_updates: bool,
    /// Explicit worker limit; None uses available machine parallelism.
    #[serde(default)]
    pub watch_workers: Option<usize>,
    /// Allow render setup to compile on a cache miss. The GUI disables this
    /// because its engine-selection worker owns background precompilation.
    #[serde(default = "default_compile_sono_gcc_on_demand")]
    pub compile_sono_gcc_on_demand: bool,
    #[serde(default)]
    pub profile: bool,
    #[serde(default)]
    pub profile_frames: bool,
    /// Run a full rendered preflight and compare draw/RGB hashes against the
    /// streaming render. Disabled for ordinary exports.
    #[serde(default)]
    pub validate_determinism: bool,
    /// Legacy configuration field, accepted but ignored: SFX now comes from rendering.
    #[serde(default = "default_concurrent_sfx_prepass")]
    pub concurrent_sfx_prepass: bool,
    /// Overlap serial Watch preparation with ordered render/output work.
    #[serde(default = "default_frame_pipeline")]
    pub frame_pipeline: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RenderBackend {
    Cpu,
    #[default]
    Wgpu,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VideoPreset {
    Ultrafast,
    Superfast,
    Veryfast,
    Faster,
    Fast,
    #[default]
    Medium,
    Slow,
    Slower,
    Veryslow,
}

impl VideoPreset {
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Ultrafast => "ultrafast",
            Self::Superfast => "superfast",
            Self::Veryfast => "veryfast",
            Self::Faster => "faster",
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::Slower => "slower",
            Self::Veryslow => "veryslow",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ResourceOverrides {
    pub particles: Option<PathBuf>,
    pub effects: Option<PathBuf>,
    pub background: Option<PathBuf>,
    pub particle_name: Option<String>,
}

pub(crate) fn validate_mv_background(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        bail!("MV background percentage must be in 0..=100");
    }
    Ok(())
}

fn default_mv_enabled() -> bool {
    true
}
fn default_mv_background() -> f64 {
    100.0
}
fn default_duration() -> f64 {
    2.0
}
fn default_compile_sono_gcc_on_demand() -> bool {
    true
}

fn default_watch_random_seed() -> u64 {
    crate::runtime::DEFAULT_WATCH_RANDOM_SEED
}
fn default_fps() -> u32 {
    12
}
fn default_width() -> u32 {
    640
}
fn default_height() -> u32 {
    360
}
fn default_video_crf() -> u8 {
    18
}

fn default_concurrent_sfx_prepass() -> bool {
    false
}

fn default_frame_pipeline() -> bool {
    true
}

fn default_parallel_watch_updates() -> bool {
    true
}

impl RenderConfig {
    /// Load a renderer-owned gameplay UI profile and attach it to this render.
    /// Relative image/font atlas and event-log paths resolve beside the profile.
    pub fn set_ui_profile(&mut self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let path = path.as_ref();
        self.ui = RendererUiConfig::load_profile(path)?;
        self.ui.resolve_profile_resources(path);
        Ok(())
    }

    /// Load a renderer-owned watermark profile. Asset paths resolve beside it.
    pub fn set_watermark_profile(&mut self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let path = path.as_ref();
        self.watermark = crate::watermark::WatermarkConfig::load_profile(path)?;
        self.watermark.resolve_profile_resources(path);
        Ok(())
    }

    /// Load the shared CLI/GUI JSON configuration. Relative paths in the
    /// config, including UI and watermark assets, resolve beside this file.
    pub fn load_config(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading renderer configuration {}", path.display()))?;
        let mut config = decode_config_file(&bytes)
            .with_context(|| format!("parsing renderer configuration {}", path.display()))?;
        config.ui.migrate_legacy_empty_default();
        let base = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| {
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf()
            });
        config.resolve_config_paths(&base);
        config.validate()?;
        if config.ui.mode == crate::render_ui::GameplayUiMode::Custom {
            config.ui.load_external_data()?;
        }
        if config.watermark.enabled {
            config.watermark.load_external_data()?;
        }
        Ok(config)
    }

    /// Save a complete shared renderer configuration as readable JSON.
    pub fn save_config(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let mut document =
            serde_json::to_value(self).context("serializing renderer configuration")?;
        let object = document
            .as_object_mut()
            .context("renderer configuration must serialize as a JSON object")?;
        object.insert(
            "format".into(),
            serde_json::Value::String(CONFIG_FILE_FORMAT.into()),
        );
        object.insert(
            "schema_version".into(),
            serde_json::Value::from(CONFIG_FILE_SCHEMA_VERSION),
        );
        let bytes = serde_json::to_vec_pretty(&document)
            .context("serializing versioned renderer configuration")?;
        write_config_atomic(path, &bytes).with_context(|| {
            format!(
                "atomically saving renderer configuration {}",
                path.display()
            )
        })
    }

    pub(crate) fn resolve_config_paths(&mut self, base: &std::path::Path) {
        fn resolve(path: &mut PathBuf, base: &std::path::Path) {
            if path.as_os_str().is_empty() || path.is_absolute() {
                return;
            }
            *path = base.join(&*path);
        }
        fn resolve_optional(path: &mut Option<PathBuf>, base: &std::path::Path) {
            if let Some(path) = path {
                resolve(path, base);
            }
        }

        resolve(&mut self.engine, base);
        resolve(&mut self.resources, base);
        resolve(&mut self.level, base);
        resolve_optional(&mut self.music, base);
        resolve_optional(&mut self.mv, base);
        resolve_optional(&mut self.output, base);
        resolve_optional(&mut self.resource_overrides.particles, base);
        resolve_optional(&mut self.resource_overrides.effects, base);
        resolve_optional(&mut self.resource_overrides.background, base);
        self.ui.resolve_config_resources(base);
        self.watermark.resolve_config_resources(base);
    }

    pub(crate) fn relativize_config_paths(&mut self, base: &std::path::Path) {
        for path in [&mut self.engine, &mut self.resources, &mut self.level] {
            relativize_path(path, base);
        }
        for path in [
            &mut self.music,
            &mut self.mv,
            &mut self.output,
            &mut self.resource_overrides.particles,
            &mut self.resource_overrides.effects,
            &mut self.resource_overrides.background,
        ]
        .into_iter()
        .flatten()
        {
            relativize_path(path, base);
        }
        self.ui.relativize_config_resources(base);
        self.watermark.relativize_config_resources(base);
    }

    /// Load the engine through the same archive/directory loader used by the
    /// existing inspection and video-export paths.
    pub fn load_engine_package(&self) -> Result<formats::EnginePackage> {
        formats::load_engine(&self.engine)
    }

    pub fn validate(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            bail!("render dimensions must be in 1..=8192");
        }
        if self.fps == 0 || self.fps > 240 {
            bail!("FPS must be in 1..=240");
        }
        if self.video_crf > 51 {
            bail!("H.264 video CRF must be in 0..=51");
        }
        if self
            .watch_workers
            .is_some_and(|count| !(1..=256).contains(&count))
        {
            bail!("Watch worker count must be in 1..=256");
        }
        if !self.start_time.is_finite() || self.start_time < 0.0 {
            bail!("start time must be finite and nonnegative");
        }
        if !self.duration.is_finite() || self.duration <= 0.0 {
            bail!("duration must be finite and positive");
        }
        for option in &self.level_options {
            if !option.value.is_finite() {
                bail!("Level Option {} must be finite", option.index);
            }
        }
        validate_mv_background(self.mv_background)?;
        self.ui.validate()?;
        self.watermark.validate()?;
        self.job_manager.validate()?;
        Ok(())
    }

    /// Validate the filesystem and media requirements for an MP4 export.
    /// This is intended for config-file preflight before loading an engine or
    /// starting other expensive render work.
    pub fn validate_export_inputs(&self) -> Result<()> {
        self.validate()?;
        for (name, path) in [
            ("engine", &self.engine),
            ("resources", &self.resources),
            ("level", &self.level),
        ] {
            if !path.is_file() {
                bail!("{name} input is missing or not a file: {}", path.display());
            }
        }
        if self.layers.bgm && self.music.as_ref().is_none_or(|path| !path.is_file()) {
            bail!("BGM is enabled: select an accessible music file or disable BGM");
        }
        if self
            .output
            .as_ref()
            .is_none_or(|path| path.as_os_str().is_empty())
        {
            bail!("video output path is required");
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            bail!("MP4 requires even width and height");
        }
        for (name, path) in [
            ("MV", self.mv.as_ref().filter(|_| self.mv_enabled)),
            (
                "particle resources",
                self.resource_overrides.particles.as_ref(),
            ),
            ("effect resources", self.resource_overrides.effects.as_ref()),
            (
                "background resources",
                self.resource_overrides.background.as_ref(),
            ),
        ] {
            if let Some(path) = path.filter(|path| !path.is_file()) {
                bail!("{name} input is missing or not a file: {}", path.display());
            }
        }
        Ok(())
    }

    fn materialized_config(&self) -> Result<Self> {
        let mut config = self.clone();
        match config.ui.mode {
            crate::render_ui::GameplayUiMode::Custom => {
                config.ui.prepare_external_data()?;
            }
            crate::render_ui::GameplayUiMode::Legacy => {}
            crate::render_ui::GameplayUiMode::Off => {
                config.ui.enabled = false;
            }
        }
        if config.watermark.enabled
            && config.watermark.loaded_assets.is_empty()
            && !config.watermark.assets.is_empty()
        {
            config.watermark.load_external_data()?;
        }
        config.validate()?;
        Ok(config)
    }

    /// Run the actual Watch/skin/background/particle/UI pipeline for one time.
    /// Time is aligned upward to the next frame at the configured FPS, matching
    /// the video export's global frame mapping.
    pub fn render_frame(&self, time: f64) -> Result<RenderedFrame> {
        self.render_frames(&[time])?
            .into_iter()
            .next()
            .context("single-frame renderer returned no frame")
    }

    /// Render several ordered checkpoints through one real Watch session.
    /// This avoids replaying the chart from time zero for every visual
    /// checkpoint while preserving the same global-frame and compositor path.
    pub(crate) fn render_frames(&self, times: &[f64]) -> Result<Vec<RenderedFrame>> {
        if times.is_empty() {
            bail!("at least one preview checkpoint is required");
        }
        if times.iter().any(|time| !time.is_finite() || *time < 0.0) {
            bail!("preview timestamps must be finite and nonnegative");
        }
        let config = self.materialized_config()?;
        let mut indexed_times = times
            .iter()
            .copied()
            .enumerate()
            .map(|(input_index, time)| {
                let index = (time * f64::from(config.fps)).ceil();
                if !index.is_finite() || index > crate::offline::MAX_EXPORT_FRAMES as f64 {
                    bail!("preview checkpoint exceeds the 1,000,000-frame renderer limit");
                }
                Ok((index as u64, input_index))
            })
            .collect::<Result<Vec<_>>>()?;
        indexed_times.sort_by_key(|(index, _)| *index);
        if indexed_times.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            bail!("preview checkpoint times must resolve to unique global frames");
        }
        let package = config.load_engine_package()?;
        let level = formats::load_level(&config.level)?;
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = config
            .skin
            .as_deref()
            .or_else(|| defaults.get("skins").map(String::as_str))
            .or(package.metadata.skin_name.as_deref())
            .context("engine has no skin selection")?;
        let background = defaults
            .get("backgrounds")
            .map(String::as_str)
            .or(package.metadata.background_name.as_deref());
        let effects = defaults
            .get("effects")
            .map(String::as_str)
            .or(package.metadata.effect_name.as_deref());
        let particles = defaults
            .get("particles")
            .map(String::as_str)
            .or(package.metadata.particle_name.as_deref());
        let options: Vec<_> = config
            .level_options
            .iter()
            .map(|item| (item.index, item.value))
            .collect();
        let mut session = FrameSession::new_configured_with_sources_and_watermark(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            &config.resources,
            skin,
            background,
            effects,
            config
                .resource_overrides
                .particle_name
                .as_deref()
                .or(particles),
            config.width,
            config.height,
            config.fps,
            &options,
            config.layers.particles,
            config.ui.clone(),
            config.start_time,
            config.duration,
            config.backend,
            config.profile,
            config.trace_entity_id.is_some(),
            &config.resource_overrides,
            config.watermark.clone(),
        )?;
        session.set_execution_mode_with_compile_policy(
            config.execution_mode,
            config.compile_sono_gcc_on_demand,
        );
        session
            .set_watch_compatibility(config.watch_compatibility_policy, config.watch_random_seed)?;
        session.set_parallel_updates(config.parallel_watch_updates, config.watch_workers)?;
        if let Some(path) = config.mv.as_deref().filter(|_| config.mv_enabled) {
            let first_index = indexed_times.first().unwrap().0;
            let last_index = indexed_times.last().unwrap().0;
            let start = times[indexed_times.first().unwrap().1];
            let duration = (last_index - first_index + 1) as f64 / f64::from(config.fps);
            let range = crate::offline::FrameRange::new(start, duration, config.fps)?;
            let timeline = crate::export_timeline::ExportTimeline::new(
                start,
                range,
                level.bgm_offset.unwrap_or(0.0),
            )?;
            session.configure_mv(&crate::ffmpeg::resolve()?, path, timeline)?;
            session.set_mv_background(config.mv_background);
        }
        let mut rendered = std::iter::repeat_with(|| None)
            .take(times.len())
            .collect::<Vec<Option<RenderedFrame>>>();
        for (frame_index, input_index) in indexed_times {
            rendered[input_index] = Some(session.render_global_frame(frame_index)?);
        }
        rendered
            .into_iter()
            .map(|frame| frame.context("renderer omitted a requested checkpoint"))
            .collect()
    }

    pub fn render_video(&self) -> Result<video_export::ExportReport> {
        self.render_video_lifecycle(Box::new(|_| {}), true, |config, sink| {
            video_export::export_config_with_progress(config, sink)
        })
    }

    pub fn render_video_controlled(
        &self,
        sink: crate::export_progress::ProgressSink,
        control: crate::export_control::ExportControl,
    ) -> Result<video_export::ExportReport> {
        self.render_video_lifecycle(sink, true, move |config, sink| {
            video_export::export_config_controlled(config, sink, control)
        })
    }

    pub(crate) fn render_video_controlled_unrecorded(
        &self,
        sink: crate::export_progress::ProgressSink,
        control: crate::export_control::ExportControl,
    ) -> Result<video_export::ExportReport> {
        self.render_video_lifecycle(sink, false, move |config, sink| {
            video_export::export_config_controlled(config, sink, control)
        })
    }

    pub fn render_video_with_progress(
        &self,
        sink: crate::export_progress::ProgressSink,
    ) -> Result<video_export::ExportReport> {
        self.render_video_lifecycle(sink, true, |config, sink| {
            video_export::export_config_with_progress(config, sink)
        })
    }

    fn render_video_lifecycle(
        &self,
        mut caller_sink: crate::export_progress::ProgressSink,
        persist_performance: bool,
        export: impl FnOnce(
            &Self,
            crate::export_progress::ProgressSink,
        ) -> Result<video_export::ExportReport>,
    ) -> Result<video_export::ExportReport> {
        use crate::notifications::RendererEventKind;

        let start = Instant::now();
        let notifications_enabled = self.notifications.has_active_route();
        let notification_config = notifications_enabled.then(|| self.notifications.clone());
        if notifications_enabled {
            crate::notifications::emit(
                &self.notifications,
                self.notification_event(RendererEventKind::RenderStarted),
            );
        }
        let progress_event_values = (notifications_enabled
            && self
                .notifications
                .routes_event(RendererEventKind::RenderProgress)
            && self.notifications.progress_mode != crate::notifications::ProgressMode::Disabled)
            .then(|| {
                self.notification_event(RendererEventKind::RenderProgress)
                    .values
            });
        let progress_start = start;
        let latest_phase = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let phase_state = latest_phase.clone();
        let mut last_progress = 0_u8;
        let mut last_progress_time = Duration::ZERO;
        let sink = Box::new(move |progress: crate::export_progress::ExportProgress| {
            if let Ok(mut phase) = phase_state.lock() {
                *phase = Some(format!("{:?}", progress.phase));
            }
            caller_sink(progress.clone());
            let Some(notification_config) = notification_config.as_ref() else {
                return;
            };
            if progress_event_values.is_none() {
                return;
            }
            if progress.phase != crate::export_progress::ExportPhase::Rendering {
                return;
            }
            let Some(percentage) = progress.percentage else {
                return;
            };
            let percent = percentage.round().clamp(0.0, 100.0) as u8;
            let elapsed = progress_start.elapsed();
            if !notification_config.progress_due(
                percent,
                elapsed,
                last_progress,
                last_progress_time,
            ) {
                return;
            }
            last_progress = percent;
            last_progress_time = elapsed;
            let mut event = crate::notifications::RendererEvent {
                kind: RendererEventKind::RenderProgress,
                values: progress_event_values.clone().unwrap_or_default(),
            };
            event.values.insert("progress".into(), percent.to_string());
            event.values.insert(
                "completed_frames".into(),
                progress.completed_frames.to_string(),
            );
            if let Some(total) = progress.total_frames {
                event.values.insert("frame_count".into(), total.to_string());
            }
            event.values.insert(
                "render_time".into(),
                format!("{:.2}", progress.elapsed_seconds),
            );
            crate::notifications::emit(notification_config, event);
        });

        let mut result = self
            .materialized_config()
            .and_then(|config| export(&config, sink));
        if persist_performance {
            if let Ok(report) = result.as_mut() {
                if let Err(error) = crate::performance::PerformanceStore::open_default()
                    .and_then(|store| store.save(&report.performance))
                {
                    eprintln!("warning: could not persist performance history: {error:#}");
                }
                if let Err(error) = crate::render_diagnostics::RenderDiagnosticStore::open_default()
                    .and_then(|store| store.save(&report.diagnostics))
                {
                    eprintln!("warning: could not persist render diagnostic: {error:#}");
                }
            } else if let Err(error) = &result {
                let error_chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
                let cancelled = error.is::<crate::export_control::Cancelled>();
                let diagnostic = crate::render_diagnostics::RenderDiagnosticRecord::from_failure(
                    format!(
                        "diagnostic-failure-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ),
                    None,
                    self,
                    latest_phase.lock().ok().and_then(|phase| phase.clone()),
                    Some(if cancelled { "cancelled" } else { "renderer" }.into()),
                    &error_chain,
                    if cancelled {
                        crate::render_diagnostics::DiagnosticOutcome::Cancelled
                    } else {
                        crate::render_diagnostics::DiagnosticOutcome::Failed
                    },
                );
                if let Err(persist_error) =
                    crate::render_diagnostics::RenderDiagnosticStore::open_default()
                        .and_then(|store| store.save(&diagnostic))
                {
                    eprintln!(
                        "warning: could not persist failed render diagnostic: {persist_error:#}"
                    );
                }
            }
        }
        let elapsed = format!("{:.2}", start.elapsed().as_secs_f64());
        if notifications_enabled {
            match &result {
                Ok(report) => {
                    let mut event = self.notification_event(RendererEventKind::RenderCompleted);
                    event.values.insert("render_time".into(), elapsed.clone());
                    event.values.insert("total_time".into(), elapsed);
                    event.values.insert(
                        "duration".into(),
                        format!("{:.3}", report.timeline.duration),
                    );
                    event
                        .values
                        .insert("frame_count".into(), report.submitted_frames.to_string());
                    event.values.insert(
                        "completed_frames".into(),
                        report.submitted_frames.to_string(),
                    );
                    crate::notifications::emit(&self.notifications, event);
                }
                Err(error) => {
                    let event = self
                        .notification_event(RendererEventKind::RenderFailed)
                        .with("error", format!("{error:#}"))
                        .with("render_time", elapsed.clone())
                        .with("total_time", elapsed);
                    crate::notifications::emit(&self.notifications, event);
                }
            }
        }
        result
    }

    pub(crate) fn render_video_unrecorded(&self) -> Result<video_export::ExportReport> {
        self.render_video_lifecycle(Box::new(|_| {}), false, |config, sink| {
            video_export::export_config_with_progress(config, sink)
        })
    }

    fn notification_event(
        &self,
        kind: crate::notifications::RendererEventKind,
    ) -> crate::notifications::RendererEvent {
        use crate::notifications::RendererEvent;
        let file_label = |path: &std::path::Path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.to_str().unwrap_or_default())
                .to_owned()
        };
        let execution_mode = match self.execution_mode {
            crate::sono_gcc::WatchExecutionMode::Interpreter => "Sono VM / Interpreter",
            crate::sono_gcc::WatchExecutionMode::SonoGcc => "Sono-GCC",
        };
        let mut event = RendererEvent::new(kind)
            .with("engine", file_label(&self.engine))
            .with("chart", file_label(&self.level))
            .with(
                "output",
                self.output
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            )
            .with(
                "output_path",
                self.output
                    .as_deref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            )
            .with("execution_mode", execution_mode)
            .with("width", self.width)
            .with("height", self.height)
            .with("FPS", self.fps)
            .with("resolution", format!("{}x{}", self.width, self.height))
            .with("duration", format!("{:.3}", self.duration));
        if self.whole_chart {
            event.values.insert("duration".into(), "whole chart".into());
        }
        event
    }
}

fn decode_config_file(bytes: &[u8]) -> Result<RenderConfig> {
    let mut document: serde_json::Value = serde_json::from_slice(bytes)?;
    strip_optional_flat_schema(
        &mut document,
        CONFIG_FILE_FORMAT,
        CONFIG_FILE_SCHEMA_VERSION,
        "renderer configuration",
    )?;
    Ok(serde_json::from_value(document)?)
}

/// Remove a flat JSON document's optional format/version markers and validate
/// them. Markerless files are the legacy schema zero representation.
pub(crate) fn strip_optional_flat_schema(
    document: &mut serde_json::Value,
    expected_format: &str,
    current_version: u32,
    label: &str,
) -> Result<u32> {
    let object = document
        .as_object_mut()
        .with_context(|| format!("{label} must be a JSON object"))?;
    let format = object.remove("format");
    let version = object.remove("schema_version");
    match (format, version) {
        (None, None) => Ok(0),
        (Some(format), Some(version)) => {
            let format = format
                .as_str()
                .with_context(|| format!("{label} format marker must be a string"))?;
            if format != expected_format {
                bail!("unsupported {label} format {format:?}");
            }
            let version = version
                .as_u64()
                .and_then(|version| u32::try_from(version).ok())
                .with_context(|| format!("{label} schema_version must be a nonnegative integer"))?;
            if version > current_version {
                bail!("{label} schema {version} is newer than supported schema {current_version}");
            }
            Ok(version)
        }
        (Some(_), None) | (None, Some(_)) => {
            bail!("{label} has an incomplete format/schema marker")
        }
    }
}

pub(crate) fn write_config_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".renderer-config-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("atomically replacing {}", path.display()))?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(crate) fn relativize_path(path: &mut std::path::PathBuf, base: &std::path::Path) {
    if !path.is_absolute() {
        return;
    }
    let resolved_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let resolved_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Ok(relative) = resolved_path.strip_prefix(&resolved_base) {
        *path = relative.to_path_buf();
    }
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            engine: PathBuf::new(),
            resources: PathBuf::new(),
            level: PathBuf::new(),
            skin: None,
            music: None,
            mv: None,
            mv_enabled: default_mv_enabled(),
            mv_background: default_mv_background(),
            resource_overrides: ResourceOverrides::default(),
            output: None,
            start_time: 0.0,
            duration: default_duration(),
            whole_chart: false,
            fps: default_fps(),
            width: default_width(),
            height: default_height(),
            video_crf: default_video_crf(),
            video_preset: VideoPreset::default(),
            level_options: Vec::new(),
            trace_entity_id: None,
            layers: RenderLayers::default(),
            ui: RendererUiConfig::reference_default(),
            watermark: crate::watermark::WatermarkConfig::default(),
            notifications: crate::notifications::NotificationConfig::default(),
            job_manager: crate::job_config::JobManagerConfig::default(),
            backend: RenderBackend::default(),
            execution_mode: crate::sono_gcc::WatchExecutionMode::default(),
            watch_compatibility_policy: crate::runtime::WatchCompatibilityPolicy::default(),
            watch_random_seed: default_watch_random_seed(),
            parallel_watch_updates: default_parallel_watch_updates(),
            watch_workers: None,
            compile_sono_gcc_on_demand: default_compile_sono_gcc_on_demand(),
            profile: false,
            profile_frames: false,
            validate_determinism: false,
            concurrent_sfx_prepass: default_concurrent_sfx_prepass(),
            frame_pipeline: default_frame_pipeline(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mv_percentage_validation_and_shared_roundtrip() {
        let mut config: RenderConfig =
            serde_json::from_value(serde_json::json!({"engine":"e", "resources":"r", "level":"l"}))
                .unwrap();
        assert_eq!(config.mv_background, 100.0);
        assert!(config.mv_enabled);
        assert!(config.watermark.enabled);
        assert!(!config.watermark.moving.enabled);
        config.mv_enabled = false;
        for percent in [0.0, 40.0, 100.0] {
            config.mv_background = percent;
            config.validate().unwrap();
            let copy: RenderConfig =
                serde_json::from_value(serde_json::to_value(&config).unwrap()).unwrap();
            assert_eq!(copy.mv_background, percent);
            assert!(!copy.mv_enabled);
        }
        for percent in [-0.01, 100.01, f64::NAN, f64::INFINITY] {
            config.mv_background = percent;
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn old_render_configs_default_to_permissive_watch_compatibility_and_seed_roundtrips() {
        let legacy: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine":"e", "resources":"r", "level":"l"
        }))
        .unwrap();
        assert_eq!(
            legacy.watch_compatibility_policy,
            crate::runtime::WatchCompatibilityPolicy::Permissive
        );
        assert_eq!(
            legacy.watch_random_seed,
            crate::runtime::DEFAULT_WATCH_RANDOM_SEED
        );

        let mut config = legacy;
        config.watch_compatibility_policy = crate::runtime::WatchCompatibilityPolicy::Strict;
        config.watch_random_seed = 17;
        let encoded = serde_json::to_value(&config).unwrap();
        let decoded: RenderConfig = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            decoded.watch_compatibility_policy,
            config.watch_compatibility_policy
        );
        assert_eq!(decoded.watch_random_seed, 17);
    }

    #[test]
    fn config_json_preserves_arbitrary_level_option_indices_and_layer_toggles() {
        let json = serde_json::json!({
            "engine":"e.zip", "resources":"r.scp", "level":"l.zip",
            "level_options":[{"index":1,"value":10.5},{"index":73,"value":-2.0}],
            "layers":{"particles":false,"sfx":true,"bgm":false},
            "ui":{"enabled":true,"primary_metric":{"enabled":true,"label":"POINTS",
                "provider":{"kind":"fixed","value":95},"maximum":100}
            }
        });
        let config: RenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(
            config.level_options[1],
            LevelOptionValue {
                index: 73,
                value: -2.0
            }
        );
        assert!(!config.layers.particles && config.layers.sfx && !config.layers.bgm);
        let encoded = serde_json::to_value(config).unwrap();
        assert_eq!(encoded["level_options"][1]["index"], 73);
    }

    #[test]
    fn shared_config_roundtrip_resolves_relative_ui_and_watermark_assets() {
        let directory = tempfile::tempdir().unwrap();
        let png_path = directory.path().join("pixel.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([30, 80, 120, 255]))
            .save(&png_path)
            .unwrap();
        std::fs::write(directory.path().join("events.json"), "[]").unwrap();

        let mut config = RenderConfig::default();
        config.engine = "engine.zip".into();
        config.resources = "resources.scp".into();
        config.level = "level.json.gz".into();
        config.ui.assets.insert("badge".into(), "pixel.png".into());
        config.ui.events_file = Some("events.json".into());
        config
            .watermark
            .assets
            .insert("mark".into(), "pixel.png".into());
        let path = directory.path().join("renderer.json");
        config.save_config(&path).unwrap();

        let loaded = RenderConfig::load_config(&path).unwrap();
        assert_eq!(loaded.ui.assets["badge"], png_path.canonicalize().unwrap());
        assert_eq!(
            loaded.ui.events_file.as_ref().unwrap(),
            &directory.path().join("events.json").canonicalize().unwrap()
        );
        assert_eq!(loaded.ui.loaded_asset_dimensions("badge"), Some((2, 2)));
        assert!(loaded.watermark.loaded_assets.contains_key("mark"));
        assert_eq!(loaded.width, 640);
        assert_eq!(loaded.fps, 12);
    }

    #[test]
    fn default_renderer_config_can_be_serialized_as_an_editable_template() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("default.json");
        RenderConfig::default().save_config(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["format"], CONFIG_FILE_FORMAT);
        assert_eq!(saved["schema_version"], CONFIG_FILE_SCHEMA_VERSION);
        let loaded = RenderConfig::load_config(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(RenderConfig::default()).unwrap()
        );
        assert!(loaded.engine.as_os_str().is_empty());
        assert!(loaded.watermark.enabled);
        assert!(!loaded.watermark.moving.enabled);
    }

    #[test]
    fn config_file_schema_accepts_legacy_flat_documents_and_rejects_future_versions() {
        let legacy = br#"{"engine":"e","resources":"r","level":"l","width":720}"#;
        let decoded = decode_config_file(legacy).unwrap();
        assert_eq!(decoded.width, 720);

        let mut current = serde_json::to_value(RenderConfig::default()).unwrap();
        current["format"] = serde_json::Value::String(CONFIG_FILE_FORMAT.into());
        current["schema_version"] = serde_json::Value::from(CONFIG_FILE_SCHEMA_VERSION);
        assert!(decode_config_file(&serde_json::to_vec(&current).unwrap()).is_ok());

        current["schema_version"] = serde_json::Value::from(CONFIG_FILE_SCHEMA_VERSION + 1);
        assert!(decode_config_file(&serde_json::to_vec(&current).unwrap())
            .unwrap_err()
            .to_string()
            .contains("newer than supported"));
    }

    #[test]
    fn config_save_replaces_atomically_and_a_rejected_save_preserves_the_previous_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("renderer.json");
        let first = RenderConfig::default();
        first.save_config(&path).unwrap();
        let old_bytes = std::fs::read(&path).unwrap();

        let mut invalid = first.clone();
        invalid.width = 0;
        assert!(invalid.save_config(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), old_bytes);

        let mut replacement = first;
        replacement.width = 1280;
        replacement.save_config(&path).unwrap();
        assert_eq!(RenderConfig::load_config(&path).unwrap().width, 1280);
    }

    #[test]
    fn notification_settings_roundtrip_and_legacy_config_defaults_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notification-config.json");
        let mut config = RenderConfig::default();
        config.notifications.discord.enabled = true;
        config.notifications.discord.bot_id = Some("123456789012345600".into());
        config.notifications.discord.bot_token_env = "RENDERER_BOT_TOKEN_ENV".into();
        config
            .notifications
            .discord
            .destinations
            .push(crate::notifications::DiscordDestination {
                guild_id: Some("123456789012345601".into()),
                channel_id: "123456789012345602".into(),
                events: vec![crate::notifications::RendererEventKind::RenderFailed],
                ..Default::default()
            });
        config.notifications.progress_mode = crate::notifications::ProgressMode::Milestones;
        config.notifications.progress_milestones = vec![20, 60, 90];
        config.save_config(&path).unwrap();
        let loaded = RenderConfig::load_config(&path).unwrap();
        assert_eq!(loaded.notifications, config.notifications);
        let json = std::fs::read_to_string(path).unwrap();
        assert!(json.contains("RENDERER_BOT_TOKEN_ENV"));
        assert!(json.contains("bot_token_env"));
        assert!(!json.contains("bot_token\": \""));

        let legacy: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": "e", "resources": "r", "level": "l"
        }))
        .unwrap();
        assert_eq!(
            legacy.notifications,
            crate::notifications::NotificationConfig::default()
        );
    }

    #[test]
    fn config_preflight_rejects_missing_assets_and_required_project_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = RenderConfig::default();
        config.engine = directory.path().join("engine.zip");
        config.resources = directory.path().join("resources.scp");
        config.level = directory.path().join("level.json.gz");
        config.music = Some(directory.path().join("music.mp3"));
        config.output = Some(directory.path().join("output.mp4"));
        for path in [
            &config.engine,
            &config.resources,
            &config.level,
            config.music.as_ref().unwrap(),
        ] {
            std::fs::write(path, b"").unwrap();
        }
        config.validate_export_inputs().unwrap();

        config.width = 641;
        assert!(config
            .validate_export_inputs()
            .unwrap_err()
            .to_string()
            .contains("even"));
        config.width = 640;
        std::fs::remove_file(&config.level).unwrap();
        assert!(config
            .validate_export_inputs()
            .unwrap_err()
            .to_string()
            .contains("level input"));

        let file = directory.path().join("invalid-assets.json");
        config.level = "level.json.gz".into();
        config
            .ui
            .assets
            .insert("missing".into(), "missing.png".into());
        config.save_config(&file).unwrap();
        assert!(RenderConfig::load_config(&file)
            .unwrap_err()
            .to_string()
            .contains("gameplay UI asset"));
    }

    #[test]
    fn preview_and_export_share_config_defaults_and_frame_alignment() {
        let json = serde_json::json!({
            "engine":"engine.zip", "resources":"resources.scp", "level":"level.zip"
        });
        let config: RenderConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.start_time, 0.0);
        assert_eq!(config.duration, 2.0);
        assert_eq!((config.fps, config.width, config.height), (12, 640, 360));
        assert_eq!(config.video_crf, 18);
        assert_eq!(config.video_preset, VideoPreset::Medium);
        assert_eq!(config.layers, RenderLayers::default());
        assert_eq!(config.backend, RenderBackend::Wgpu);
        assert_eq!(
            config.execution_mode,
            crate::sono_gcc::WatchExecutionMode::Interpreter
        );
        assert!(config.parallel_watch_updates);
        assert_eq!(config.watch_workers, None);
        assert!(config.compile_sono_gcc_on_demand);
        assert!(!config.concurrent_sfx_prepass);
        assert!(config.frame_pipeline);
        assert!(config.ui.enabled);
        assert_eq!((0.01_f64 * 12.0).ceil() as u64, 1);
    }

    #[test]
    fn encoder_quality_range_and_preset_names_are_stable() {
        let mut config = RenderConfig::default();
        for crf in [0, 18, 51] {
            config.video_crf = crf;
            config.validate().unwrap();
        }
        config.video_crf = 52;
        assert!(config.validate().unwrap_err().to_string().contains("CRF"));
        assert_eq!(VideoPreset::Veryslow.ffmpeg_name(), "veryslow");
        assert_eq!(VideoPreset::Medium.ffmpeg_name(), "medium");
    }

    #[test]
    fn shared_config_loads_next_rush_engine_zip_through_standard_loader() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": "resources.scp",
            "level": "level.zip"
        }))
        .unwrap();

        let package = config.load_engine_package().unwrap();
        assert!(!package.watch.nodes.is_empty());
        assert!(package.watch.update_spawn.is_some());
    }

    #[test]
    fn next_rush_shared_preview_initializes_baumkuchen_at_cli_options() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let options = [(1, 10.8), (22, 1.0)];
        let mut stepper = crate::offline::WatchFrameStepper::new_with_engine_options(
            &package.watch,
            &package.rom,
            &package.configuration,
            &level,
            12,
            &options,
        )
        .unwrap();

        // Preview shares the same frame-zero initialization and monotonically advances
        // through the requested frame, just as video export does.
        let report = stepper.advance_to(0).unwrap();
        assert!(report.callbacks.iter().any(|callback| {
            callback.archetype == "Initialization"
                && callback.stage == crate::watch_runtime::LifecycleStage::Preprocess
        }));
        assert_eq!(stepper.runtime_mut().global_memory.get(2002, 1), 10.8);
        assert_eq!(stepper.runtime_mut().global_memory.get(2002, 22), 1.0);
    }

    #[test]
    fn next_rush_event_only_prepass_matches_render_session_events_and_runtime() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let package =
            formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level = formats::load_level(&repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz")).unwrap();
        let resources = repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = defaults.get("skins").unwrap();
        let background = defaults.get("backgrounds").map(String::as_str);
        let effects = defaults.get("effects").map(String::as_str);
        let particles = defaults.get("particles").map(String::as_str);
        let options = [(1, 10.8), (22, 1.0)];
        let mut render = FrameSession::new_configured(
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
            false,
            RendererUiConfig::default(),
            0.0,
            2.0,
            RenderBackend::Cpu,
            false,
            false,
        )
        .unwrap();
        let mut events = crate::offline::EventSession::new_configured(
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
            false,
            false,
            false,
        )
        .unwrap();
        let mut saw_audio_events = false;
        for frame_index in 0..120 {
            let event_report = events.advance_global_frame(frame_index).unwrap();
            let rendered_report = render.render_global_frame(frame_index).unwrap().report;
            assert_eq!(event_report.callbacks, rendered_report.callbacks);
            assert_eq!(event_report.runtime_update, rendered_report.runtime_update);
            assert_eq!(
                event_report.runtime_update_after_callbacks,
                rendered_report.runtime_update_after_callbacks
            );
            assert_eq!(event_report.audio_events, rendered_report.audio_events);
            assert_eq!(
                event_report.scheduled_effects,
                rendered_report.scheduled_effects
            );
            assert_eq!(
                event_report.scheduled_looped_effects,
                rendered_report.scheduled_looped_effects
            );
            assert_eq!(
                event_report.scheduled_looped_effect_stops,
                rendered_report.scheduled_looped_effect_stops
            );
            assert_eq!(
                event_report.particle_events,
                rendered_report.particle_events
            );
            assert_eq!(
                event_report.destroyed_particle_effects,
                rendered_report.destroyed_particle_effects
            );
            saw_audio_events |= !event_report.audio_events.is_empty()
                || !event_report.scheduled_effects.is_empty()
                || !event_report.scheduled_looped_effects.is_empty()
                || !event_report.scheduled_looped_effect_stops.is_empty();
        }
        assert!(
            saw_audio_events,
            "Baumkuchen fixture should exercise SFX events"
        );
    }

    #[test]
    fn next_rush_baumkuchen_shared_preview_matches_cpu_reference_on_wgpu() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
            "start_time": 0.0,
            "duration": 0.25,
            "fps": 12,
            "width": 320,
            "height": 180,
            "level_options": [{"index": 1, "value": 10.8}, {"index": 22, "value": 1.0}],
            "layers": {"particles": false, "sfx": false, "bgm": false},
            "watermark": {
                "enabled": true,
                "attribution": {"font_size": 16.0, "opacity": 0.68, "margin": 8.0},
                "moving": {
                    "enabled": true,
                    "content": {"kind": "text", "text": "VM GCC PARITY"},
                    "font_size": 23.0,
                    "opacity": 0.28,
                    "start": [0.5, 0.5],
                    "bounds": [0.05, 0.05, 0.9, 0.9],
                    "movement": {"kind": "random-waypoints", "segment_seconds": 0.8},
                    "seed": 731,
                    "layer": "topmost"
                }
            },
            "backend": "wgpu"
        })).unwrap();

        let cpu = {
            let mut cpu_config = config.clone();
            cpu_config.backend = RenderBackend::Cpu;
            cpu_config.render_frame(0.0).unwrap()
        };
        let frame = config.render_frame(0.0).unwrap();
        let uploaded =
            crate::gpu_render::with_uploaded_base_reference(|| config.render_frame(0.0).unwrap());
        if let Some(index) = frame
            .rgb
            .iter()
            .zip(&uploaded.rgb)
            .position(|(a, b)| a != b)
        {
            panic!(
                "clear/upload first divergence at pixel ({}, {}), channel {}: {} != {}",
                index / 3 % 320,
                index / 3 / 320,
                index % 3,
                frame.rgb[index],
                uploaded.rgb[index],
            );
        }
        assert_eq!(frame.rgb.len(), uploaded.rgb.len());
        assert_eq!(
            crate::offline::hash_rgb(&frame.rgb),
            crate::offline::hash_rgb(&uploaded.rgb)
        );
        assert_close_rgb(&cpu.rgb, &frame.rgb, 3, 0.002);
    }

    #[test]
    fn real_chart_scoring_is_applied_by_the_shared_preview_frame_pipeline() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let level_path = repo.join("TestingSuite/Next Sekai Engine/levels/bike/bike.json.gz");
        let level = formats::load_level(&level_path).unwrap();
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
        assert!(first_note.time > 0.1, "fixture needs a pre-note checkpoint");
        let before_time = first_note.time - 0.05;
        let after_time = first_note.time + 0.05;

        let mut ui = RendererUiConfig::reference_default();
        ui.enabled = true;
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next Sekai Engine/engine/Next Sekai Engine.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": level_path,
            "start_time": 0.0,
            "duration": after_time + 0.1,
            "fps": 60,
            "width": 320,
            "height": 180,
            "layers": {"particles": false, "sfx": false, "bgm": false},
            "ui": ui,
            "backend": "cpu"
        }))
        .unwrap();

        let frames = config.render_frames(&[before_time, after_time]).unwrap();
        let before = frames[0].ui_state.gameplay.as_ref().unwrap();
        let after = frames[1].ui_state.gameplay.as_ref().unwrap();
        assert_eq!(frames[1].ui_state.scoring_status.as_deref().is_some(), true);
        assert!(after.score > before.score);
        assert!(after.combo > before.combo);
        assert_eq!(after.variables["effective_level_rating"], 0.0);
        assert_eq!(
            after.life, 1_000.0,
            "the overlay exposes a selected life track, not a universal note-life formula"
        );
        assert_eq!(after.judgment.as_deref(), Some("PERFECT"));
        assert_eq!(after.judgment_time, Some(first_note.time));
        assert!(after.variables.contains_key("score_progress"));
        assert_eq!(before.variables["combo_pulse_visible"], 0.0);
        assert_eq!(before.variables["judgment_opacity"], 0.0);
        assert!(after.variables["combo_pop_scale"] > 0.5);
        assert!(after.variables["combo_pop_scale"] <= 1.0);
        assert!(after.variables["combo_pulse_opacity"] > 0.0);
        assert!(after.variables["judgment_opacity"] > 0.0);
        assert_ne!(
            frames[0].rgb, frames[1].rgb,
            "the preview compositor must draw the event-driven HUD state"
        );
    }

    #[test]
    fn next_rush_armageddon_shared_preview_initialization_succeeds() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/Touhou - ARMAGEDDON/Armageddon.json.gz"),
            "start_time": 14.0,
            "duration": 2.0,
            "fps": 12,
            "width": 128,
            "height": 72,
            "level_options": [{"index": 1, "value": 10.8}, {"index": 22, "value": 1.0}],
            "layers": {"particles": false, "sfx": false, "bgm": false}
        })).unwrap();

        // Build the expensive Watch snapshot once. The previous test rendered
        // the complete 14-second ARMAGEDDON traversal independently on CPU
        // and WGPU, although only the raster backend differs between passes.
        let stage_start = std::time::Instant::now();
        eprintln!("ARMAGEDDON diagnostic: materializing config");
        let mut config = config.materialized_config().unwrap();
        eprintln!(
            "ARMAGEDDON diagnostic: config materialized in {:?}",
            stage_start.elapsed()
        );
        config.profile = true;
        let stage_start = std::time::Instant::now();
        eprintln!("ARMAGEDDON diagnostic: loading engine package");
        let package = config.load_engine_package().unwrap();
        eprintln!(
            "ARMAGEDDON diagnostic: engine package loaded in {:?}",
            stage_start.elapsed()
        );
        let stage_start = std::time::Instant::now();
        let level = formats::load_level(&config.level).unwrap();
        eprintln!(
            "ARMAGEDDON diagnostic: full level loaded in {:?} ({} entities)",
            stage_start.elapsed(),
            level.entities.len()
        );
        let stage_start = std::time::Instant::now();
        eprintln!("ARMAGEDDON diagnostic: constructing frame session");
        let mut session =
            configured_test_frame_session(&config, &package, &level, RenderBackend::Wgpu);
        eprintln!(
            "ARMAGEDDON diagnostic: frame session constructed in {:?}",
            stage_start.elapsed()
        );
        let frame_index = (14.0_f64 * f64::from(config.fps)).ceil() as u64;
        // Report stage-level progress for this unusually expensive real
        // fixture. Profiling uses the renderer's existing Performance Lab
        // counters rather than a separate timing model.
        eprintln!("ARMAGEDDON diagnostic: preparing Watch frame 0/{frame_index}");
        let stage_start = std::time::Instant::now();
        let first = session.prepare_global_frame(0).unwrap();
        eprintln!(
            "ARMAGEDDON Watch frame 0/{frame_index}: total={:?} profile={:?}",
            stage_start.elapsed(),
            first.report.runtime_profile
        );
        for checkpoint in 1..frame_index {
            let stage_start = std::time::Instant::now();
            let prepared = session.prepare_global_frame(checkpoint).unwrap();
            let profile = &prepared.report.runtime_profile;
            let (vm, preparation) = prepared.producer_timings();
            if checkpoint % u64::from(config.fps) == 0 || checkpoint + 1 == frame_index {
                eprintln!(
                    "ARMAGEDDON Watch frame {checkpoint}/{frame_index}: total={:?} timeline={:.3}s frame={:?} Watch={:?} preparation={:?} callbacks={} evaluations={}",
                    stage_start.elapsed(),
                    prepared.report.timeline.unwrap_or_default(),
                    profile.frame,
                    vm,
                    preparation,
                    profile.callbacks,
                    profile.evaluations,
                );
            }
        }
        eprintln!("ARMAGEDDON Watch preparing final frame {frame_index}/{frame_index}");
        let prepared = session.prepare_global_frame(frame_index).unwrap();
        let frame = session
            .render_prepared_with_backend(prepared.clone(), RenderBackend::Cpu)
            .unwrap();
        assert_eq!(frame.report.runtime_update[0], 14.0);
        assert_eq!(frame.rgb.len(), 128 * 72 * 3);
        let gpu = session
            .render_prepared_with_backend(prepared, RenderBackend::Wgpu)
            .unwrap();
        assert_close_rgb(&frame.rgb, &gpu.rgb, 3, 0.002);
    }

    #[test]
    fn gameplay_ui_event_state_is_composited_by_real_preview_pipeline() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture = repo.join("TestingSuite/Next Sekai Engine/levels/larp 64x");
        let mut config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": fixture.join("Next RUSH.zip"),
            "resources": fixture.join("ProSeka Faithful 0.8.4.scp"),
            "level": fixture.join("larp 64x.json.gz"),
            "start_time": 0.0,
            "duration": 3.0,
            "fps": 12,
            "width": 640,
            "height": 360,
            "layers": {"particles": false, "sfx": false, "bgm": false},
            "watermark": {"enabled": false}
        }))
        .unwrap();
        let (mut imported_ui, import_report) =
            RendererUiConfig::import_asset_pack(repo.join("assets/presentation-pack.json"), None)
                .unwrap();
        assert!(import_report.loaded_assets.iter().any(|id| id == "judge.1"));
        imported_ui.scoring = Some(crate::render_ui::UiScoringConfig {
            source: crate::render_ui::UiScoringSource::EventTrack,
            initial_score: 0.0,
            initial_combo: 0,
            initial_life: 1_000.0,
            minimum_life: 0.0,
            maximum_life: 1_000.0,
            rules: vec![
                crate::render_ui::UiScoreRule {
                    event: "hit".into(),
                    note_type: Some("normal".into()),
                    score: 100.0,
                    combo: crate::render_ui::UiComboChange::Increment,
                    life: -120.0,
                    judgement: "PERFECT".into(),
                    ..Default::default()
                },
                crate::render_ui::UiScoreRule {
                    event: "hit".into(),
                    note_type: Some("special".into()),
                    score: 200.0,
                    combo: crate::render_ui::UiComboChange::Increment,
                    life: 40.0,
                    judgement: "GREAT".into(),
                    ..Default::default()
                },
                crate::render_ui::UiScoreRule {
                    event: "miss".into(),
                    note_type: Some("normal".into()),
                    combo: crate::render_ui::UiComboChange::Reset,
                    life: -300.0,
                    judgement: "MISS".into(),
                    ..Default::default()
                },
            ],
            forecast: Default::default(),
        });
        for (id, source, font, y) in [
            (
                "imported-score",
                crate::render_ui::UiStateSource::Score,
                "score",
                0.20,
            ),
            (
                "imported-life",
                crate::render_ui::UiStateSource::Life,
                "life",
                0.34,
            ),
        ] {
            imported_ui.elements.push(crate::render_ui::UiElement {
                id: id.into(),
                layout: crate::render_ui::UiLayout {
                    x: 0.04,
                    y,
                    width: 0.36,
                    height: 0.10,
                    ..Default::default()
                },
                kind: crate::render_ui::UiElementKind::Counter {
                    value: crate::render_ui::UiExpression::Source { source },
                    prefix: String::new(),
                    suffix: String::new(),
                    digits: 0,
                    color: [255; 4],
                    font_size: 44.0,
                    font: Some(font.into()),
                    align: crate::render_ui::UiTextAlign::Left,
                },
                ..Default::default()
            });
        }
        config.ui = imported_ui;
        config.ui.events_file = Some(repo.join("ui-profiles/example-events.json"));
        config.ui.load_external_data().unwrap();
        let config = config.materialized_config().unwrap();
        let package = config.load_engine_package().unwrap();
        let level = formats::load_level(&config.level).unwrap();
        let mut session =
            configured_test_frame_session(&config, &package, &level, RenderBackend::Wgpu);
        let initial = session.prepare_global_frame(0).unwrap();
        let initial_frame = session
            .render_prepared_with_backend(initial, RenderBackend::Cpu)
            .unwrap();
        assert_eq!(
            initial_frame.ui_state.gameplay.as_ref().map(|state| (
                state.score,
                state.combo,
                state.life
            )),
            Some((0.0, 0, 1_000.0))
        );
        let prepared = session
            .prepare_global_frame((1.25_f64 * f64::from(config.fps)).ceil() as u64)
            .unwrap();
        let cpu_frame = session
            .render_prepared_with_backend(prepared.clone(), RenderBackend::Cpu)
            .unwrap();
        let gpu_frame = session
            .render_prepared_with_backend(prepared.clone(), RenderBackend::Wgpu)
            .unwrap();
        let gameplay = cpu_frame.ui_state.gameplay.as_ref().unwrap();
        assert_eq!(
            (gameplay.score, gameplay.combo, gameplay.life),
            (100.0, 1, 880.0)
        );
        assert_eq!(gameplay.judgment.as_deref(), Some("PERFECT"));
        assert_eq!(gameplay.latest_event_time, Some(1.0));

        let second_hit = session
            .prepare_global_frame((1.75_f64 * f64::from(config.fps)).ceil() as u64)
            .unwrap();
        let second_hit_frame = session
            .render_prepared_with_backend(second_hit, RenderBackend::Cpu)
            .unwrap();
        let second_hit_state = second_hit_frame.ui_state.gameplay.as_ref().unwrap();
        assert_eq!(
            (
                second_hit_state.score,
                second_hit_state.combo,
                second_hit_state.life
            ),
            (300.0, 2, 920.0)
        );
        assert_eq!(second_hit_state.judgment.as_deref(), Some("GREAT"));

        let miss = session
            .prepare_global_frame((2.5_f64 * f64::from(config.fps)).ceil() as u64)
            .unwrap();
        let miss_frame = session
            .render_prepared_with_backend(miss, RenderBackend::Cpu)
            .unwrap();
        let miss_state = miss_frame.ui_state.gameplay.as_ref().unwrap();
        assert_eq!(
            (miss_state.score, miss_state.combo, miss_state.life),
            (300.0, 0, 620.0)
        );
        assert_eq!(miss_state.judgment.as_deref(), Some("MISS"));
        assert_eq!(cpu_frame.ui_state.gameplay, gpu_frame.ui_state.gameplay);
        assert_eq!(cpu_frame.ui_state.time, gpu_frame.ui_state.time);
        assert_close_rgb(&cpu_frame.rgb, &gpu_frame.rgb, 3, 0.002);

        let mut plain_config = config.clone();
        plain_config.ui.mode = crate::render_ui::GameplayUiMode::Off;
        plain_config.ui.enabled = false;
        let plain_cpu = session
            .render_prepared_with_backend_and_ui(
                prepared.clone(),
                RenderBackend::Cpu,
                plain_config.ui.clone(),
            )
            .unwrap();
        let plain_gpu = session
            .render_prepared_with_backend_and_ui(
                prepared.clone(),
                RenderBackend::Wgpu,
                plain_config.ui,
            )
            .unwrap();
        let changed_cpu_pixels = cpu_frame
            .rgb
            .chunks_exact(3)
            .zip(plain_cpu.rgb.chunks_exact(3))
            .filter(|(hud, scene)| hud != scene)
            .count();
        assert!(
            changed_cpu_pixels > 500,
            "expected visible CPU HUD pixels, got {changed_cpu_pixels}"
        );
        let changed_gpu_pixels = gpu_frame
            .rgb
            .chunks_exact(3)
            .zip(plain_gpu.rgb.chunks_exact(3))
            .filter(|(hud, scene)| hud != scene)
            .count();
        assert!(
            changed_gpu_pixels > 500,
            "expected visible WGPU HUD pixels, got {changed_gpu_pixels}"
        );

        let repeated_cpu = session
            .render_prepared_with_backend(prepared.clone(), RenderBackend::Cpu)
            .unwrap();
        let repeated_gpu = session
            .render_prepared_with_backend(prepared, RenderBackend::Wgpu)
            .unwrap();
        assert_eq!(cpu_frame.ui_state.gameplay, repeated_cpu.ui_state.gameplay);
        assert_eq!(cpu_frame.rgb, repeated_cpu.rgb);
        assert_eq!(gpu_frame.ui_state.gameplay, repeated_gpu.ui_state.gameplay);
        assert_eq!(gpu_frame.rgb, repeated_gpu.rgb);
    }

    #[test]
    fn next_rush_lapis_shared_preview_matches_cpu_reference_on_wgpu() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config: RenderConfig = serde_json::from_value(serde_json::json!({
            "engine": repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
            "resources": repo.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp"),
            "level": repo.join("TestingSuite/Next Sekai Engine/levels/DIAMOND Lapis/DIAMOND Lapis.json.gz"),
            "start_time": 0.0, "duration": 1.0, "fps": 12, "width": 160, "height": 90,
            "layers": {"particles": false, "sfx": false, "bgm": false}
        })).unwrap();
        let cpu = config.render_frame(0.0).unwrap();
        let mut gpu_config = config;
        gpu_config.backend = RenderBackend::Wgpu;
        gpu_config.mv = Some("deliberately absent disabled movie.mp4".into());
        gpu_config.mv_enabled = false;
        let gpu = gpu_config.render_frame(0.0).unwrap();
        assert_close_rgb(&cpu.rgb, &gpu.rgb, 3, 0.002);
    }

    fn configured_test_frame_session<'a>(
        config: &RenderConfig,
        package: &'a formats::EnginePackage,
        level: &'a formats::LevelData,
        backend: RenderBackend,
    ) -> FrameSession<'a> {
        let defaults: std::collections::BTreeMap<_, _> =
            package.metadata.resource_defaults().into_iter().collect();
        let skin = config
            .skin
            .as_deref()
            .or_else(|| defaults.get("skins").map(String::as_str))
            .or(package.metadata.skin_name.as_deref())
            .unwrap();
        let background = defaults
            .get("backgrounds")
            .map(String::as_str)
            .or(package.metadata.background_name.as_deref());
        let effects = defaults
            .get("effects")
            .map(String::as_str)
            .or(package.metadata.effect_name.as_deref());
        let particles = defaults
            .get("particles")
            .map(String::as_str)
            .or(package.metadata.particle_name.as_deref());
        let options: Vec<_> = config
            .level_options
            .iter()
            .map(|item| (item.index, item.value))
            .collect();
        let mut session = FrameSession::new_configured_with_sources_and_watermark(
            &package.watch,
            &package.rom,
            &package.configuration,
            level,
            &config.resources,
            skin,
            background,
            effects,
            config
                .resource_overrides
                .particle_name
                .as_deref()
                .or(particles),
            config.width,
            config.height,
            config.fps,
            &options,
            config.layers.particles,
            config.ui.clone(),
            config.start_time,
            config.duration,
            backend,
            config.profile,
            config.trace_entity_id.is_some(),
            &config.resource_overrides,
            config.watermark.clone(),
        )
        .unwrap();
        session.set_execution_mode_with_compile_policy(
            config.execution_mode,
            config.compile_sono_gcc_on_demand,
        );
        session
            .set_watch_compatibility(config.watch_compatibility_policy, config.watch_random_seed)
            .unwrap();
        session
            .set_parallel_updates(config.parallel_watch_updates, config.watch_workers)
            .unwrap();
        session
    }

    fn assert_close_rgb(cpu: &[u8], gpu: &[u8], max_delta: u8, max_outlier_fraction: f64) {
        assert_eq!(cpu.len(), gpu.len());
        let mut outliers = 0usize;
        for (&a, &b) in cpu.iter().zip(gpu) {
            if a.abs_diff(b) > max_delta {
                outliers += 1;
            }
        }
        let fraction = outliers as f64 / cpu.len().max(1) as f64;
        assert!(
            fraction <= max_outlier_fraction,
            "{outliers}/{} RGB samples exceed {max_delta}: {fraction:.6}",
            cpu.len()
        );
    }
}
