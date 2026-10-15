use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use renderer::{compatibility, formats};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "Sono-Renderer",
    version,
    about = "Sonolus Watch-mode renderer and media exporter"
)]
struct Cli {
    /// Use a non-default named configuration profile catalog.
    #[arg(long, global = true)]
    profile_store: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum BackendArg {
    Cpu,
    Wgpu,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum VideoPresetArg {
    Ultrafast,
    Superfast,
    Veryfast,
    Faster,
    Fast,
    Medium,
    Slow,
    Slower,
    Veryslow,
}

impl From<VideoPresetArg> for renderer::render::VideoPreset {
    fn from(value: VideoPresetArg) -> Self {
        match value {
            VideoPresetArg::Ultrafast => Self::Ultrafast,
            VideoPresetArg::Superfast => Self::Superfast,
            VideoPresetArg::Veryfast => Self::Veryfast,
            VideoPresetArg::Faster => Self::Faster,
            VideoPresetArg::Fast => Self::Fast,
            VideoPresetArg::Medium => Self::Medium,
            VideoPresetArg::Slow => Self::Slow,
            VideoPresetArg::Slower => Self::Slower,
            VideoPresetArg::Veryslow => Self::Veryslow,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ExecutionModeArg {
    Interpreter,
    SonoGcc,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum WatchCompatibilityPolicyArg {
    Permissive,
    Strict,
}

impl From<WatchCompatibilityPolicyArg> for renderer::runtime::WatchCompatibilityPolicy {
    fn from(value: WatchCompatibilityPolicyArg) -> Self {
        match value {
            WatchCompatibilityPolicyArg::Permissive => Self::Permissive,
            WatchCompatibilityPolicyArg::Strict => Self::Strict,
        }
    }
}

const SONO_GCC_REGION_SIZE_BUCKET_LABELS: [&str; 8] =
    ["1", "2", "3-4", "5-8", "9-16", "17-32", "33-64", "65+"];

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum JobPriorityArg {
    High,
    Normal,
    Low,
}

#[derive(Subcommand)]
enum CacheAction {
    /// List managed disk entries and process-local caches.
    List {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show totals by managed cache category.
    Stats {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect one store-qualified cache entry (for example appdata:HASH).
    Inspect { id: String },
    /// Validate hashes and metadata for one entry or every managed entry.
    Validate {
        id: Option<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Remove one generated cache entry.
    Remove { id: String },
    /// Clear inactive persistent entries and, unless --disk-only, safe memory caches.
    Clear {
        /// Keep process-local decoded audio, atlas textures, and loaded-program caches.
        #[arg(long)]
        disk_only: bool,
    },
    /// Explicitly prune oldest-use entries until the persistent cache is under this byte limit.
    Prune {
        #[arg(long, value_name = "BYTES")]
        max_bytes: u64,
    },
    /// Recompile a graph from a valid cached graph payload after removing its old entry.
    Rebuild { id: String },
}

#[derive(Subcommand)]
enum AdminAction {
    /// Show the saved admin service settings (this does not query a running process).
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Validate the saved bind and remote-access prerequisites without listening.
    Validate,
    /// Save admin service settings. Binding changes require an application restart.
    Configure {
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    /// Run the hosted admin service until Ctrl+C.
    Run,
}

impl From<JobPriorityArg> for renderer::job_manager::JobPriority {
    fn from(value: JobPriorityArg) -> Self {
        match value {
            JobPriorityArg::High => Self::High,
            JobPriorityArg::Normal => Self::Normal,
            JobPriorityArg::Low => Self::Low,
        }
    }
}

impl From<ExecutionModeArg> for renderer::sono_gcc::WatchExecutionMode {
    fn from(value: ExecutionModeArg) -> Self {
        match value {
            ExecutionModeArg::Interpreter => Self::Interpreter,
            ExecutionModeArg::SonoGcc => Self::SonoGcc,
        }
    }
}

#[derive(Debug, Clone, Default, Args)]
struct RenderLayerArgs {
    #[arg(long, conflicts_with = "no_ui")]
    ui: bool,
    #[arg(long)]
    no_ui: bool,
    #[arg(long, conflicts_with = "no_ui_primary_metric")]
    ui_primary_metric: bool,
    #[arg(long)]
    no_ui_primary_metric: bool,
    #[arg(long, conflicts_with = "no_ui_secondary_metric")]
    ui_secondary_metric: bool,
    #[arg(long)]
    no_ui_secondary_metric: bool,
    #[arg(long, conflicts_with = "no_ui_combo")]
    ui_combo: bool,
    #[arg(long)]
    no_ui_combo: bool,
    #[arg(long, conflicts_with = "no_ui_judgment")]
    ui_judgment: bool,
    #[arg(long)]
    no_ui_judgment: bool,
    #[arg(long, conflicts_with = "no_ui_progress")]
    ui_progress: bool,
    #[arg(long)]
    no_ui_progress: bool,
    #[arg(long)]
    primary_metric_provider: Option<String>,
    #[arg(long)]
    primary_metric_max: Option<f64>,
    #[arg(long)]
    primary_metric_label: Option<String>,
    #[arg(long)]
    secondary_metric_provider: Option<String>,
    #[arg(long)]
    secondary_metric_max: Option<f64>,
    #[arg(long)]
    secondary_metric_label: Option<String>,
    #[arg(long)]
    combo_provider: Option<String>,
    #[arg(long)]
    combo_label: Option<String>,
    #[arg(long)]
    judgment_provider: Option<String>,
    #[arg(long)]
    judgment_label: Option<String>,
    #[arg(long, conflicts_with = "no_particles")]
    particles: bool,
    #[arg(long)]
    no_particles: bool,
    #[arg(long, conflicts_with = "no_sfx")]
    sfx: bool,
    #[arg(long)]
    no_sfx: bool,
    #[arg(long, conflicts_with = "no_bgm")]
    bgm: bool,
    #[arg(long)]
    no_bgm: bool,
}

#[cfg(test)]
mod render_argument_tests {
    use super::*;

    #[test]
    fn cli_layer_and_ui_controls_build_independent_render_configuration() {
        let args = RenderLayerArgs {
            ui: false,
            no_ui: false,
            ui_primary_metric: false,
            no_ui_primary_metric: true,
            ui_secondary_metric: true,
            no_ui_secondary_metric: false,
            ui_combo: false,
            no_ui_combo: false,
            ui_judgment: false,
            no_ui_judgment: false,
            ui_progress: false,
            no_ui_progress: true,
            primary_metric_provider: None,
            primary_metric_max: None,
            primary_metric_label: None,
            secondary_metric_provider: Some("memory:2000:7".into()),
            secondary_metric_max: Some(100.0),
            secondary_metric_label: Some("ALT".into()),
            combo_provider: None,
            combo_label: None,
            judgment_provider: None,
            judgment_label: None,
            particles: false,
            no_particles: true,
            sfx: false,
            no_sfx: true,
            bgm: false,
            no_bgm: false,
        };
        let ui = args.ui_config().unwrap();
        assert!(!ui.enabled || ui.secondary_metric.enabled);
        assert!(ui.secondary_metric.enabled);
        assert!(!ui.primary_metric.enabled);
        assert!(!ui.progress);
        assert_eq!(
            ui.secondary_metric.provider,
            renderer::render_ui::UiValueProvider::Memory {
                block_id: 2000,
                index: 7
            }
        );
        let layers = args.layers();
        assert!(!layers.particles && !layers.sfx && layers.bgm);
    }
}

impl RenderLayerArgs {
    fn ui_config(&self) -> Result<renderer::render_ui::RendererUiConfig> {
        use renderer::render_ui::RendererUiConfig;
        let mut ui = RendererUiConfig::default();
        self.apply_ui_overrides(&mut ui)?;
        ui.validate()?;
        Ok(ui)
    }

    fn apply_ui_overrides(&self, ui: &mut renderer::render_ui::RendererUiConfig) -> Result<()> {
        use renderer::render_ui::GameplayUiMode;
        if self.no_ui {
            ui.enabled = false;
            ui.mode = GameplayUiMode::Off;
            return Ok(());
        }
        if self.ui
            || self.ui_primary_metric
            || self.ui_secondary_metric
            || self.ui_combo
            || self.ui_judgment
            || self.ui_progress
            || self.primary_metric_provider.is_some()
            || self.secondary_metric_provider.is_some()
            || self.combo_provider.is_some()
            || self.judgment_provider.is_some()
        {
            ui.enabled = true;
        }
        if self.ui_primary_metric {
            ui.primary_metric.enabled = true;
        }
        if self.no_ui_primary_metric {
            ui.primary_metric.enabled = false;
        }
        if self.ui_secondary_metric {
            ui.secondary_metric.enabled = true;
        }
        if self.no_ui_secondary_metric {
            ui.secondary_metric.enabled = false;
        }
        if self.ui_combo {
            ui.combo.enabled = true;
        }
        if self.no_ui_combo {
            ui.combo.enabled = false;
        }
        if self.ui_judgment {
            ui.judgment.enabled = true;
        }
        if self.no_ui_judgment {
            ui.judgment.enabled = false;
        }
        if self.ui || self.ui_progress {
            ui.progress = true;
        }
        if self.no_ui_progress {
            ui.progress = false;
        }
        if let Some(provider) = &self.primary_metric_provider {
            ui.primary_metric.provider = parse_ui_provider(provider)?;
            ui.primary_metric.enabled = !self.no_ui_primary_metric;
        }
        if let Some(provider) = &self.secondary_metric_provider {
            ui.secondary_metric.provider = parse_ui_provider(provider)?;
            ui.secondary_metric.enabled = !self.no_ui_secondary_metric;
        }
        if let Some(provider) = &self.combo_provider {
            ui.combo.provider = parse_ui_provider(provider)?;
            ui.combo.enabled = !self.no_ui_combo;
        }
        if let Some(provider) = &self.judgment_provider {
            ui.judgment.provider = parse_ui_provider(provider)?;
            ui.judgment.enabled = !self.no_ui_judgment;
        }
        if let Some(maximum) = self.primary_metric_max {
            ui.primary_metric.maximum = Some(maximum);
        }
        if let Some(maximum) = self.secondary_metric_max {
            ui.secondary_metric.maximum = Some(maximum);
        }
        if let Some(label) = &self.primary_metric_label {
            ui.primary_metric.label = label.clone();
        }
        if let Some(label) = &self.secondary_metric_label {
            ui.secondary_metric.label = label.clone();
        }
        if let Some(label) = &self.combo_label {
            ui.combo.label = label.clone();
        }
        if let Some(label) = &self.judgment_label {
            ui.judgment.label = label.clone();
        }
        Ok(())
    }

    fn layers(&self) -> renderer::render::RenderLayers {
        renderer::render::RenderLayers {
            particles: !self.no_particles,
            sfx: !self.no_sfx,
            bgm: !self.no_bgm,
        }
    }
}

fn parse_ui_provider(value: &str) -> Result<renderer::render_ui::UiValueProvider> {
    use renderer::render_ui::UiValueProvider;
    if let Some(raw) = value.strip_prefix("fixed:") {
        return Ok(UiValueProvider::Fixed {
            value: raw.parse()?,
        });
    }
    if let Some(raw) = value.strip_prefix("external:") {
        return Ok(UiValueProvider::External {
            value: raw.parse()?,
        });
    }
    if value == "progress" {
        return Ok(UiValueProvider::Progress);
    }
    if value == "accuracy" {
        return Ok(UiValueProvider::Accuracy);
    }
    if let Some(key) = value.strip_prefix("engine:") {
        return Ok(UiValueProvider::EngineMetric { key: key.into() });
    }
    if let Some(mapping) = value.strip_prefix("judgment:") {
        return Ok(UiValueProvider::JudgmentDerived {
            mapping: mapping.into(),
        });
    }
    if let Some(pair) = value.strip_prefix("memory:") {
        let (block, index) = pair
            .split_once(':')
            .context("memory provider must be memory:BLOCK:INDEX")?;
        return Ok(UiValueProvider::Memory {
            block_id: block.parse()?,
            index: index.parse()?,
        });
    }
    anyhow::bail!("unknown metric provider {value:?}; use fixed:N, external:N, memory:BLOCK:INDEX, progress, engine:KEY, accuracy, or judgment:MAP")
}

#[derive(Subcommand)]
enum Command {
    /// Inspect or run the hosted platform administration console.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Inspect or acquire one authorized HTTP(S) media source.
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },
    /// Inspect or preflight a reusable project file.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Validate and inspect all entries in a versioned presentation asset pack.
    InspectPresentationAssetPack {
        path: PathBuf,
        /// Emit machine-readable JSON, including content identities and media metadata.
        #[arg(long)]
        json: bool,
    },
    /// Resolve a project and submit its immutable render snapshot to the job queue.
    RenderProject {
        project: PathBuf,
        /// Explicitly select a profile, overriding the project's required profile reference.
        #[arg(long)]
        profile: Option<String>,
        /// Override the project's default MP4 path.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Persist the job without starting workers.
        #[arg(long)]
        enqueue_only: bool,
        /// Override the shared render queue store location.
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Render an MP4 from the shared CLI/GUI JSON configuration.
    RenderConfig {
        config: Option<PathBuf>,
        /// Resolve a saved full-render profile instead of the supplied config values.
        #[arg(long, conflicts_with = "config")]
        profile: Option<String>,
    },
    /// Validate a shared renderer JSON configuration and all enabled assets.
    ValidateConfig {
        config: Option<PathBuf>,
        /// Validate a saved full-render profile instead of the supplied config values.
        #[arg(long, conflicts_with = "config")]
        profile: Option<String>,
    },
    /// Write a default renderer JSON configuration template.
    DefaultConfig {
        output: PathBuf,
    },
    /// Remove reproducible Sono-GCC compiled artifacts from the persistent cache.
    ClearGccCache,
    /// Inspect and manage generated renderer caches. User data is never included.
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    FfmpegInfo,
    ValidateProject {
        engine: PathBuf,
        resources: PathBuf,
        level_data: PathBuf,
        music: PathBuf,
        #[arg(long)]
        cover: Option<PathBuf>,
        #[arg(long = "mv")]
        custom_mv: Option<PathBuf>,
    },
    InspectEngine {
        engine: PathBuf,
    },
    /// Print the content identity required to bind a portable authoring schema to an engine.
    EngineIdentity {
        engine: PathBuf,
    },
    /// Compile and persist Sono-GCC code for an engine without rendering.
    CompileEngine {
        engine: PathBuf,
    },
    InspectLevel {
        level: PathBuf,
    },
    /// Export canonical Sonolus LevelData as deterministic gzip JSON.
    ExportLevelData {
        level: PathBuf,
        output: PathBuf,
        /// Explicitly replace an existing output file.
        #[arg(long)]
        overwrite: bool,
    },
    /// Run generic structural LevelData checks, optionally against an engine's real archetype list.
    ValidateLevelData {
        level: PathBuf,
        #[arg(long)]
        engine: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    InspectResources {
        scp: PathBuf,
    },
    Compatibility {
        engine: PathBuf,
        resources: PathBuf,
        level: PathBuf,
        /// Execute a bounded single-threaded Watch preprocess compatibility probe.
        #[arg(long)]
        dynamic: bool,
        /// Print a concise human-readable report (default preserves legacy JSON output).
        #[arg(long, conflicts_with = "json")]
        human_readable: bool,
        /// Print the versioned structured report as JSON.
        #[arg(long, conflicts_with = "human_readable")]
        json: bool,
    },
    /// Evaluate one WatchData node and serialize the resulting display list.
    EvaluateNode {
        engine: PathBuf,
        node: usize,
    },
    /// Execute the generic Watch host for one supplied LevelData frame.
    RunWatch {
        engine: PathBuf,
        level: PathBuf,
        /// JSON TraceConfig filters for the authoritative Watch execution.
        #[arg(long, requires = "watch_trace_output")]
        watch_trace_config: Option<PathBuf>,
        /// Bounded JSONL execution trace (default capacity 10000).
        #[arg(long)]
        watch_trace_output: Option<PathBuf>,
        #[arg(long)]
        resources: Option<PathBuf>,
        #[arg(long, default_value_t = 0.0)]
        time: f64,
        /// Watch execution mode (sono-gcc safely falls back for unsupported paths).
        #[arg(long = "vm", value_enum, default_value_t = ExecutionModeArg::Interpreter)]
        execution_mode: ExecutionModeArg,
        /// Permit documented approximations or reject underspecified Watch operations.
        #[arg(long, value_enum, default_value = "permissive")]
        watch_compatibility_policy: WatchCompatibilityPolicyArg,
        /// Seed used by the renderer's deterministic Random approximation.
        #[arg(long, default_value_t = renderer::runtime::DEFAULT_WATCH_RANDOM_SEED)]
        watch_random_seed: u64,
        /// Disable parallel UpdateParallel workers and use ordered single-thread Watch execution.
        #[arg(long, conflicts_with = "watch_workers")]
        single_threaded: bool,
        /// Limit persistent Watch workers (default: available machine parallelism).
        #[arg(long, value_name = "COUNT", conflicts_with = "single_threaded")]
        watch_workers: Option<usize>,
        /// Collect per-stage Watch CPU profile counters and timings.
        #[arg(long)]
        profile: bool,
        /// Skip per-operation BTreeMap accounting for production-like CPU benchmarks.
        #[arg(long)]
        no_vm_accounting: bool,
        /// Override a Level Option after EngineConfiguration defaults are bound (repeatable: INDEX=VALUE).
        #[arg(long = "level-option", value_name = "INDEX=VALUE", action = clap::ArgAction::Append)]
        level_options: Vec<String>,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
        #[command(flatten)]
        render_layers: RenderLayerArgs,
        /// Write a skin-textured PPM frame after Watch execution (requires --resources).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Write the executed display list, including Draw provenance, as JSON.
        #[arg(long)]
        display_list_output: Option<PathBuf>,
        /// Capture evaluated Watch node values for every executed Draw argument.
        #[arg(long, requires = "display_list_output")]
        trace_draws: bool,
        /// Write per-Draw transform, screen, atlas-UV, and painter-order diagnostics.
        #[arg(long, requires = "resources")]
        render_diagnostics_output: Option<PathBuf>,
        /// Capture a bounded trace when this entity callback reaches the VM limit.
        #[arg(long)]
        diagnostic_entity: Option<usize>,
        /// Callback stage for --diagnostic-entity (for example preprocess or update-sequential).
        #[arg(long)]
        diagnostic_stage: Option<String>,
        #[arg(long, default_value_t = 512)]
        diagnostic_events: usize,
    },
    /// Render a supplied display-list JSON file to a diagnostic PPM frame.
    RenderDisplayList {
        display_list: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 1280)]
        width: u32,
        #[arg(long, default_value_t = 720)]
        height: u32,
    },
    /// Render a deterministic Watch segment or whole chart to MP4 with level music.
    RenderVideo {
        engine: PathBuf,
        resources: PathBuf,
        level: PathBuf,
        music: PathBuf,
        output: PathBuf,
        /// Optional presentation asset; not used in gameplay rendering.
        #[arg(long)]
        cover: Option<PathBuf>,
        /// Custom video background on the BGM media timeline.
        #[arg(long = "mv")]
        custom_mv: Option<PathBuf>,
        /// Retain a selected MV path but disable video background use.
        #[arg(long)]
        no_mv: bool,
        /// MV presentation brightness, independent of the media timeline.
        #[arg(long, default_value_t = 100.0, value_parser = parse_mv_background)]
        mv_background: f64,
        /// Apply the Watch-produced Runtime Background transform to custom MV footage.
        #[arg(long)]
        apply_engine_background_rotation: bool,
        /// Optional separate SCP particle resource collection.
        #[arg(long)]
        particle_resources: Option<PathBuf>,
        #[arg(long)]
        particle_name: Option<String>,
        #[arg(long)]
        start_time: Option<f64>,
        #[arg(long, conflicts_with = "whole_chart")]
        duration: Option<f64>,
        /// Infer the chart end from Watch schedules, BGM, and known SFX tails.
        #[arg(long)]
        whole_chart: bool,
        /// Load a renderer-owned gameplay UI profile (JSON).
        #[arg(long)]
        ui_profile: Option<PathBuf>,
        /// Load a renderer-owned watermark profile (JSON).
        #[arg(long)]
        watermark_profile: Option<PathBuf>,
        /// Disable fixed attribution and the moving watermark together.
        #[arg(long, conflicts_with = "moving_watermark")]
        no_watermark: bool,
        /// Enable the profile's moving watermark as well as fixed attribution.
        #[arg(long, conflicts_with = "no_moving_watermark")]
        moving_watermark: bool,
        /// Disable the moving watermark while retaining fixed attribution.
        #[arg(long)]
        no_moving_watermark: bool,
        #[arg(long, default_value_t = 12)]
        fps: u32,
        /// H.264 constant-rate-factor quality (0 best/lossless, 51 lowest).
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=51))]
        video_crf: Option<u8>,
        /// H.264 speed/compression preset.
        #[arg(long, value_enum)]
        video_preset: Option<VideoPresetArg>,
        /// Load notification settings from a NotificationConfig JSON file.
        #[arg(long, value_name = "JSON")]
        notifications_config: Option<PathBuf>,
        /// Disable all notifications for this render.
        #[arg(long)]
        no_notifications: bool,
        #[arg(long, default_value_t = 640)]
        width: u32,
        #[arg(long, default_value_t = 360)]
        height: u32,
        /// Pixel backend used by shared preview and video rendering.
        #[arg(long, value_enum, default_value_t = BackendArg::Wgpu)]
        backend: BackendArg,
        /// Watch execution mode: interpreter (default) or sono-gcc.
        #[arg(long = "vm", value_enum, default_value_t = ExecutionModeArg::Interpreter)]
        execution_mode: ExecutionModeArg,
        /// Permit documented approximations or reject underspecified Watch operations.
        #[arg(long, value_enum, default_value = "permissive")]
        watch_compatibility_policy: WatchCompatibilityPolicyArg,
        /// Seed used by the renderer's deterministic Random approximation.
        #[arg(long, default_value_t = renderer::runtime::DEFAULT_WATCH_RANDOM_SEED)]
        watch_random_seed: u64,
        /// Disable parallel UpdateParallel workers and use ordered single-thread Watch execution.
        #[arg(long, conflicts_with = "watch_workers")]
        single_threaded: bool,
        /// Limit persistent Watch workers (default: available machine parallelism).
        #[arg(long, value_name = "COUNT", conflicts_with = "single_threaded")]
        watch_workers: Option<usize>,
        /// Collect render-video pipeline timings and workload counters.
        #[arg(long)]
        profile: bool,
        /// Print a detailed line for each profiled frame.
        #[arg(long, requires = "profile")]
        profile_frames: bool,
        /// Render a full preflight and compare draw/RGB hashes against streaming.
        #[arg(long)]
        validate_determinism: bool,
        /// Legacy no-op: SFX is collected by the rendering Watch execution.
        #[arg(long, conflicts_with = "concurrent_sfx_prepass")]
        sequential_sfx_prepass: bool,
        /// Legacy no-op: no independent SFX Watch session is created.
        #[arg(
            long,
            requires = "sequential_frames",
            conflicts_with = "sequential_sfx_prepass"
        )]
        concurrent_sfx_prepass: bool,
        /// Disable bounded Watch-to-render frame overlap for A/B benchmarking.
        #[arg(long)]
        sequential_frames: bool,
        /// Include Draw geometry/Unlerp values for a runtime entity in frame diagnostics.
        #[arg(long)]
        trace_entity: Option<usize>,
        /// Override a Level Option after EngineConfiguration defaults are bound (repeatable: INDEX=VALUE).
        #[arg(long = "level-option", value_name = "INDEX=VALUE", action = clap::ArgAction::Append)]
        level_options: Vec<String>,
        #[command(flatten)]
        render_layers: RenderLayerArgs,
    },
    /// Open the native desktop export application.
    Gui,
    /// Inspect and control the persistent render-job queue.
    Jobs {
        /// Override the shared platform queue store location.
        #[arg(long, global = true)]
        store: Option<PathBuf>,
        #[command(subcommand)]
        action: JobAction,
    },
    /// Browse durable render attempts and submit an exact historical rerender.
    History {
        /// Override the history directory.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Override the render queue store used by `history rerender`.
        #[arg(long)]
        queue_store: Option<PathBuf>,
        #[command(subcommand)]
        action: HistoryAction,
    },
    /// Export, inspect, verify, compare, or reproduce a resolved render manifest.
    Manifest {
        #[command(subcommand)]
        action: ManifestAction,
    },
    /// Inspect performance history or benchmark a renderer configuration.
    Performance {
        #[command(subcommand)]
        action: PerformanceAction,
    },
    /// Inspect or export the shared structured render diagnostics.
    Diagnostics {
        #[command(subcommand)]
        action: DiagnosticsAction,
    },
    /// Capture and compare versioned visual, audio, and semantic references.
    Regression {
        #[command(subcommand)]
        action: RegressionAction,
    },
    /// Create, select, and manage saved full-render configuration profiles.
    Profiles {
        #[command(subcommand)]
        action: ProfileAction,
    },
}

#[derive(Subcommand)]
enum MediaAction {
    /// Inspect one source URL without downloading media.
    InspectUrl {
        url: String,
        #[arg(long)]
        json: bool,
    },
    /// Acquire one item into a local directory and optionally assign it to a project.
    Download {
        url: String,
        #[arg(long, value_enum, default_value_t = MediaModeArg::Audio)]
        mode: MediaModeArg,
        #[arg(long)]
        output_dir: Option<PathBuf>,
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Report managed yt-dlp availability without installing or updating it.
    ToolInfo {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum MediaModeArg {
    Audio,
    AudioMp3,
    Video,
    VideoAudio,
}

impl From<MediaModeArg> for renderer::media_acquisition::AcquisitionMode {
    fn from(value: MediaModeArg) -> Self {
        match value {
            MediaModeArg::Audio => Self::Audio,
            MediaModeArg::AudioMp3 => Self::AudioMp3,
            MediaModeArg::Video => Self::Video,
            MediaModeArg::VideoAudio => Self::VideoAudio,
        }
    }
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Show the stored project, including unresolved references and unknown fields.
    Inspect { path: PathBuf },
    /// Parse inputs, resolve the required profile, and run compatibility preflight.
    Validate {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum JobAction {
    /// Queue a shared renderer JSON configuration and run the queue.
    Submit {
        /// Shared JSON config; optional when --profile is supplied.
        config: Option<PathBuf>,
        /// Resolve a saved full-render configuration profile.
        #[arg(long, conflicts_with = "config")]
        profile: Option<String>,
        #[arg(long, value_enum)]
        priority: Option<JobPriorityArg>,
        /// Persist the job without starting workers.
        #[arg(long)]
        enqueue_only: bool,
    },
    /// Execute all queued jobs using the queue's stored snapshots.
    Run {
        /// Optional configuration used for scheduler settings.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    List,
    Inspect {
        id: String,
    },
    Cancel {
        id: String,
    },
    Retry {
        id: String,
        #[arg(long)]
        enqueue_only: bool,
    },
    Remove {
        id: String,
    },
    Reorder {
        id: String,
        position: usize,
    },
    Pause,
    Resume,
    CancelQueued,
    ClearTerminal,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum HistoryStatusArg {
    Completed,
    Failed,
    Cancelled,
}

impl From<HistoryStatusArg> for renderer::render_history::HistoryStatus {
    fn from(value: HistoryStatusArg) -> Self {
        match value {
            HistoryStatusArg::Completed => Self::Completed,
            HistoryStatusArg::Failed => Self::Failed,
            HistoryStatusArg::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Subcommand)]
enum HistoryAction {
    /// List recent attempts; filters can be combined.
    List {
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        search: Option<String>,
        #[arg(long, value_enum)]
        status: Option<HistoryStatusArg>,
        #[arg(long)]
        engine: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Submit a new job from the recorded effective configuration.
    Rerender {
        id: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        enqueue_only: bool,
    },
    Delete {
        id: String,
    },
    Pin {
        id: String,
    },
    Unpin {
        id: String,
    },
    Clear {
        #[arg(long)]
        include_failures: bool,
    },
    /// Apply retention settings and immediately prune eligible entries.
    Prune {
        #[arg(long)]
        max_entries: Option<usize>,
        #[arg(long)]
        max_age_days: Option<u64>,
        #[arg(long)]
        delete_failures: bool,
    },
    Export {
        id: String,
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum ManifestAction {
    /// Export a portable manifest from one immutable history attempt.
    Export {
        history_id: String,
        output: PathBuf,
        #[arg(long)]
        history_store: Option<PathBuf>,
    },
    /// Show the manifest's provenance, asset identities, backend, and reference hashes.
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Hash local candidate files and explain each match or difference.
    Verify {
        path: PathBuf,
        /// Repair a path hint using ASSET_ID=FILE; repeat for multiple missing assets.
        #[arg(long = "asset", value_name = "ASSET_ID=FILE", action = clap::ArgAction::Append)]
        assets: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Compare two manifests by content, configuration, presentation, output, backend, and provenance.
    Diff {
        left: PathBuf,
        right: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Submit a fresh queue job from the manifest's exact effective configuration.
    Reproduce {
        path: PathBuf,
        /// Repair a path hint using ASSET_ID=FILE; repeat for multiple missing assets.
        #[arg(long = "asset", value_name = "ASSET_ID=FILE", action = clap::ArgAction::Append)]
        assets: Vec<String>,
        /// Required new output path; existing files are never replaced.
        #[arg(long, required = true)]
        output: PathBuf,
        /// Persist the reproduction job without starting queue workers.
        #[arg(long)]
        enqueue_only: bool,
        /// Override the persistent render queue store.
        #[arg(long)]
        queue_store: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    List,
    Show {
        selector: String,
    },
    Create {
        name: String,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Duplicate {
        selector: String,
        name: String,
    },
    Rename {
        selector: String,
        name: String,
    },
    Update {
        selector: String,
        #[arg(long)]
        config: PathBuf,
    },
    Reset {
        selector: String,
    },
    Select {
        selector: String,
    },
    Use {
        selector: String,
    },
    Deselect,
    Delete {
        selector: String,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    Export {
        selector: String,
        path: PathBuf,
    },
}

#[derive(Subcommand)]
enum PerformanceAction {
    /// List recent completed-render performance records.
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Compare two records only when their workload and environment match.
    Compare {
        left_id: String,
        right_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Repeat a real render from shared JSON configuration in a temporary output directory.
    Benchmark {
        config: PathBuf,
        #[arg(long, default_value_t = 1)]
        warmup: usize,
        #[arg(long, default_value_t = 3)]
        iterations: usize,
        /// Run matched Sono VM and Sono-GCC workloads. Cache state is reported as observed.
        #[arg(long)]
        compare_vm_gcc: bool,
        /// Repeat matched runs with deep profiling enabled to estimate instrumentation overhead.
        #[arg(long)]
        measure_telemetry_overhead: bool,
        /// Write the structured benchmark report to a new JSON file.
        #[arg(long, conflicts_with = "json")]
        output: Option<PathBuf>,
        /// Write the structured benchmark report to stdout.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DiagnosticsAction {
    /// Inspect or export one diagnostic sidecar by diagnostic ID.
    Record {
        id: String,
        #[arg(long, conflicts_with = "json")]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Inspect or export a persisted render job's diagnostic record.
    Job {
        id: String,
        /// Optional alternate render-job queue store.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Write a redacted JSON report atomically to a new file.
        #[arg(long, conflicts_with = "json")]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Analyze or export a completed authoritative performance record.
    Performance {
        id: String,
        /// Write a redacted JSON report atomically to a new file.
        #[arg(long, conflicts_with = "json")]
        output: Option<PathBuf>,
        /// Emit machine-readable JSON to stdout.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RegressionAction {
    /// Deliberately create or replace a reference after reviewing its contents.
    Update {
        config: PathBuf,
        /// Override the config's embedded gameplay UI with a renderer-owned profile.
        #[arg(long)]
        ui_profile: Option<PathBuf>,
        /// Override the config's embedded watermark with a renderer-owned profile.
        #[arg(long)]
        watermark_profile: Option<PathBuf>,
        #[arg(long, value_name = "DIRECTORY")]
        reference: PathBuf,
        #[arg(long, value_name = "ID")]
        fixture: String,
        /// Canonical Watch timeline checkpoint; repeat to capture multiple frames.
        #[arg(long = "checkpoint", required = true, action = clap::ArgAction::Append)]
        checkpoints: Vec<f64>,
        /// Optional WIDTHxHEIGHT checkpoint resolution; repeat for responsive layouts.
        #[arg(long = "resolution", action = clap::ArgAction::Append, value_parser = parse_regression_resolution)]
        resolutions: Vec<[u32; 2]>,
        /// Capture matched Sono VM and Sono-GCC paths. VM remains the semantic oracle.
        #[arg(long)]
        compare_vm_gcc: bool,
        /// Number of measured checkpoint timing observations (1..=5).
        #[arg(long, default_value_t = 3)]
        timing_samples: usize,
        /// Also render a configured video of at most 120 seconds and capture audio/media metadata.
        #[arg(long)]
        include_media: bool,
        /// Required before replacing an existing marked reference bundle.
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        json: bool,
    },
    /// Render stored checkpoints and compare all captured layers to a reference.
    Run {
        config: PathBuf,
        /// Supply the same UI profile used when this reference was created.
        #[arg(long)]
        ui_profile: Option<PathBuf>,
        /// Supply the same watermark profile used when this reference was created.
        #[arg(long)]
        watermark_profile: Option<PathBuf>,
        #[arg(long, value_name = "DIRECTORY")]
        reference: PathBuf,
        /// Retain current PNGs, comparison JSON, and bounded diff artifacts here.
        #[arg(long, value_name = "DIRECTORY")]
        output: Option<PathBuf>,
        /// Per-channel RGB tolerance, default exact (0); maximum is 2.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=2))]
        pixel_tolerance: u8,
        #[arg(long)]
        json: bool,
    },
    /// Show the schema, workload, modes, checkpoints, and artifact inventory.
    Inspect {
        reference: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

fn print_notification_diagnostics(config: &renderer::notifications::NotificationConfig) {
    for diagnostic in config.diagnostics() {
        log::warn!(target: "renderer::notifications", "{diagnostic}");
        eprintln!("notification configuration: {diagnostic}");
    }
}

fn flush_cli_notifications() {
    if !renderer::notifications::flush(std::time::Duration::from_secs(3)) {
        log::warn!(target: "renderer::notifications", "notification delivery did not finish before CLI shutdown; the export is complete");
        eprintln!(
            "notification delivery did not finish before CLI shutdown; the export is complete"
        );
    }
    for diagnostic in renderer::notifications::take_diagnostics() {
        log::warn!(target: "renderer::notifications", "{diagnostic}");
        eprintln!("notification: {diagnostic}");
    }
}

fn parse_mv_background(value: &str) -> std::result::Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "expected a percentage".to_owned())?;
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err("MV background must be in 0..=100".into())
    }
}

fn run_cache_action(action: CacheAction) -> Result<()> {
    use renderer::cache_manager as cache;
    match action {
        CacheAction::List { json } => {
            let inventory = cache::inventory()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inventory)?);
            } else {
                println!(
                    "Persistent cache entries ({} total, {}):",
                    inventory
                        .regenerable_disk
                        .iter()
                        .map(|cache| cache.entries)
                        .sum::<usize>(),
                    format_cache_bytes(inventory.total_disk_bytes)
                );
                if inventory
                    .regenerable_disk
                    .iter()
                    .all(|cache| cache.entries == 0)
                {
                    println!("  none");
                }
                for entry in &inventory.disk_entries {
                    println!(
                        "  {} | {} | {} | schema {} | {:?} | {}",
                        entry.id,
                        entry.category,
                        format_cache_bytes(entry.size_bytes),
                        entry
                            .schema
                            .map_or_else(|| "unknown".into(), |value| value.to_string()),
                        entry.status,
                        entry.target.as_deref().unwrap_or("target unknown"),
                    );
                    if let Some(created) = entry.created_unix_ms {
                        println!("    created: {}", format_cache_time(created));
                    }
                    if let Some(used) = entry.last_used_unix_ms {
                        println!("    last used: {}", format_cache_time(used));
                    }
                    if let Some(diagnostic) = &entry.diagnostic {
                        println!("    {diagnostic}");
                    }
                }
                for cache in &inventory.regenerable_disk {
                    println!(
                        "  {}: {} entries, {}",
                        cache.category,
                        cache.entries,
                        format_cache_bytes(cache.bytes)
                    );
                }
                println!("Process-local caches:");
                for item in &inventory.memory {
                    println!(
                        "  {}: {} entries, {}{}",
                        item.category,
                        item.entries,
                        item.retained_bytes
                            .map_or_else(|| "size unknown".into(), format_cache_bytes),
                        item.limit_bytes.map_or_else(String::new, |limit| format!(
                            " / {} limit",
                            format_cache_bytes(limit)
                        )),
                    );
                }
                println!("{}", inventory.durable_data_note);
            }
        }
        CacheAction::Stats { json } => {
            let inventory = cache::inventory()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&inventory)?);
            } else {
                println!(
                    "Persistent cache total: {} across {} entries",
                    format_cache_bytes(inventory.total_disk_bytes),
                    inventory
                        .regenerable_disk
                        .iter()
                        .map(|cache| cache.entries)
                        .sum::<usize>()
                );
                let mut categories = std::collections::BTreeMap::<&str, (usize, u64)>::new();
                for entry in &inventory.disk_entries {
                    let value = categories.entry(entry.category).or_default();
                    value.0 += 1;
                    value.1 = value.1.saturating_add(entry.size_bytes);
                }
                for (name, (count, bytes)) in categories {
                    println!("  {name}: {count} entries, {}", format_cache_bytes(bytes));
                }
                for cache in &inventory.regenerable_disk {
                    if cache.category != "Sono-GCC compiled programs" {
                        println!(
                            "  {}: {} entries, {}",
                            cache.category,
                            cache.entries,
                            format_cache_bytes(cache.bytes)
                        );
                    }
                }
                println!("Process-local cache memory:");
                for item in &inventory.memory {
                    println!(
                        "  {}: {} entries, {}{}",
                        item.category,
                        item.entries,
                        item.retained_bytes
                            .map_or_else(|| "size unknown".into(), format_cache_bytes),
                        item.limit_bytes.map_or_else(String::new, |limit| format!(
                            " / {} limit",
                            format_cache_bytes(limit)
                        )),
                    );
                }
                println!("{}", inventory.durable_data_note);
            }
        }
        CacheAction::Inspect { id } => {
            let entry = cache::inspect(&id)?;
            println!("{}", serde_json::to_string_pretty(&entry)?);
        }
        CacheAction::Validate { id, json } => {
            let entries = if let Some(id) = id {
                vec![cache::validate(&id)?]
            } else {
                cache::validate_all()?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for entry in &entries {
                    println!(
                        "{}: {:?} — {}",
                        entry.id,
                        entry.status,
                        entry
                            .diagnostic
                            .as_deref()
                            .unwrap_or("integrity checks passed")
                    );
                }
            }
            let invalid = entries
                .iter()
                .filter(|entry| entry.status != renderer::cache_manager::CacheEntryStatus::Valid)
                .count();
            if invalid > 0 {
                anyhow::bail!(
                    "cache validation found {invalid} incompatible or corrupt entr{}",
                    if invalid == 1 { "y" } else { "ies" }
                );
            }
        }
        CacheAction::Remove { id } => {
            let report = cache::remove(&id)?;
            println!(
                "Removed {} cache entry ({}).",
                report.removed_entries,
                format_cache_bytes(report.removed_bytes)
            );
        }
        CacheAction::Clear { disk_only } => {
            let report = cache::clear(!disk_only)?;
            println!(
                "Removed {} persistent entries ({}); skipped {} active entries; cleared {} process-local cache references ({} tempo-analysis entries).",
                report.removed_entries,
                format_cache_bytes(report.removed_bytes),
                report.skipped_active,
                report.cleared_memory_entries,
                report.cleared_tempo_analysis_entries,
            );
        }
        CacheAction::Prune { max_bytes } => {
            let report = cache::prune_to(max_bytes)?;
            println!(
                "Pruned {} entries ({}); skipped {} active entries to meet the requested {} limit.",
                report.removed_entries,
                format_cache_bytes(report.removed_bytes),
                report.skipped_active,
                format_cache_bytes(max_bytes),
            );
        }
        CacheAction::Rebuild { id } => {
            let report = cache::rebuild(&id)?;
            println!(
                "Rebuilt graph {}: {:?} cache, {} regions, {} / {} eligible operations.",
                report.cache_key,
                report.cache_hit,
                report.compiled_regions,
                report.compiled_operations,
                report.eligible_operations,
            );
            if let Some(warning) = report.cache_write_error {
                println!("Persistent cache warning: {warning}");
            }
        }
    }
    Ok(())
}

fn format_cache_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn format_cache_time(unix_ms: u128) -> String {
    let seconds = i64::try_from(unix_ms / 1000).unwrap_or(i64::MAX);
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = second_of_day / 3_600;
    let minute = second_of_day % 3_600 / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

#[derive(Debug)]
struct CliExitError {
    code: i32,
    message: String,
}

impl std::fmt::Display for CliExitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliExitError {}

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let result = if args.is_empty() || (args.len() == 1 && args[0] == "gui") {
        // Keep the native window startup on the process thread. CLI parsing and
        // asset inspection use a larger owned stack because Clap and image
        // decoders have deep, input-dependent call frames on Windows.
        renderer::gui::launch()
    } else {
        std::thread::Builder::new()
            .name("renderer-cli".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(main_result)
            .map_err(anyhow::Error::from)
            .and_then(|worker| {
                worker
                    .join()
                    .map_err(|_| anyhow::anyhow!("renderer CLI worker panicked"))?
            })
    };
    if let Err(error) = result {
        log::error!(target: "renderer::application", "{error:#}");
        eprintln!("{error:#}");
        let code = error
            .downcast_ref::<CliExitError>()
            .map_or(1, |exit| exit.code);
        std::process::exit(code);
    }
}

fn main_result() -> Result<()> {
    if std::env::args_os().len() == 1 {
        return renderer::gui::launch();
    }

    let cli = Cli::parse();
    run_cli(cli)
}

fn run_cli(cli: Cli) -> Result<()> {
    let profile_store = cli.profile_store;
    match cli.command {
        Command::Admin { action } => match action {
            AdminAction::Status { json } => {
                let config = renderer::admin::AdminServerConfig::load_default()?;
                let view = renderer::admin::AdminConfigView::from_config(&config);
                if json {
                    println!("{}", serde_json::to_string_pretty(&view)?);
                } else {
                    println!(
                        "Admin service: {}",
                        if config.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    );
                    println!("Configured bind: {}:{}", config.bind_address, config.port);
                    println!("Authentication: {}", view.authentication);
                    println!("Note: status reports saved configuration; use `admin run` to start a headless instance.");
                }
            }
            AdminAction::Validate => {
                let config = renderer::admin::AdminServerConfig::load_default()?;
                renderer::admin::validate_startup_security(&config)?;
                println!(
                    "Admin settings are valid. Enabled: {}; bind: {}:{}.",
                    config.enabled, config.bind_address, config.port
                );
            }
            AdminAction::Configure {
                enable,
                disable,
                bind,
                port,
            } => {
                let mut config = renderer::admin::AdminServerConfig::load_default()?;
                if enable {
                    config.enabled = true;
                }
                if disable {
                    config.enabled = false;
                }
                if let Some(bind) = bind {
                    config.bind_address = bind;
                }
                if let Some(port) = port {
                    config.port = port;
                }
                config.validate()?;
                renderer::admin::validate_startup_security(&config)?;
                config.save_default()?;
                println!(
                    "Saved admin service settings: {}; bind {}:{}; restart required.",
                    if config.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    config.bind_address,
                    config.port
                );
            }
            AdminAction::Run => {
                let config = renderer::admin::AdminServerConfig::load_default()?;
                anyhow::ensure!(config.enabled, "admin service is disabled; enable it in the native GUI or admin-server.json first");
                let manager = std::sync::Arc::new(renderer::job_manager::RenderJobManager::open(
                    renderer::job_manager::JobManagerConfig::default(),
                )?);
                manager.start()?;
                let server = renderer::admin::start(&config, manager)?
                    .context("admin service did not start")?;
                println!("Admin service listening at {}", server.url());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(tokio::signal::ctrl_c())?;
                server.shutdown();
            }
        },
        Command::Media { action } => run_media_action(action)?,
        Command::Project { action } => run_project_command(profile_store, action)?,
        Command::InspectPresentationAssetPack { path, json } => {
            run_presentation_command(path, json)?
        }
        Command::RenderProject {
            project: project_path,
            profile,
            output,
            enqueue_only,
            store,
        } => {
            let project = renderer::project::RenderProject::load(&project_path)?;
            let profiles = if profile.is_some() || project.profile.is_some() {
                Some(open_profile_manager(profile_store.clone())?)
            } else {
                None
            };
            let (mut config, profile_source) = project.resolve_config(
                &project_path,
                &renderer::render::RenderConfig::default(),
                profiles.as_ref(),
                profile.as_deref(),
            )?;
            if let Some(output) = output {
                config.output = Some(output);
            }
            let validation = project.validate_inputs(&project_path)?;
            if !validation.is_valid() {
                anyhow::bail!(
                    "project input validation failed:\n{}",
                    validation.errors.join("\n")
                );
            }
            config.validate_export_inputs()?;
            let manager = open_job_manager(store, config.job_manager.clone())?;
            let profile = profile_source.map(|profile| (profile.id, profile.name));
            let id = manager.enqueue_with_metadata(
                config,
                None,
                profile,
                renderer::job_manager::RenderJobMetadata {
                    project_path: Some(project_path.clone()),
                    ..Default::default()
                },
            )?;
            println!("queued project render job {id}");
            if !enqueue_only {
                manager.start()?;
                manager.wait_until_idle()?;
                let job = manager.inspect(&id)?;
                println!("{:?}: {}", job.state, job.output_path().unwrap().display());
                if job.state != renderer::job_manager::JobState::Completed {
                    anyhow::bail!(
                        "render job {id} ended as {:?}: {}",
                        job.state,
                        job.diagnostic.as_deref().unwrap_or("no diagnostic")
                    );
                }
                renderer::notifications::flush(std::time::Duration::from_secs(3));
            }
        }
        Command::RenderConfig { config, profile } => {
            let config = if let Some(selector) = profile {
                open_profile_manager(profile_store.clone())?.resolve(&selector)?
            } else {
                renderer::render::RenderConfig::load_config(
                    config.context("provide a config path or --profile")?,
                )?
            };
            print_notification_diagnostics(&config.notifications);
            config.validate_export_inputs()?;
            let output = config
                .output
                .clone()
                .context("renderer config must set an output MP4 path")?;
            let report =
                render_with_diagnostics(&config, renderer::export_progress::terminal_sink());
            flush_cli_notifications();
            let report = report?;
            print_render_warnings(&report);
            println!("wrote MP4 to {}", output.display());
            println!(
                "submitted deterministic frames: {}",
                report.submitted_frames
            );
            println!("Watch execution: {}", report.watch_execution_status);
            print_performance_record_summary(&report.performance);
            println!("render diagnostic: {}", report.diagnostics.id);
            println!("rendered RGB sequence SHA-1: {}", report.second_pass_hash);
            println!("SFX event sequence SHA-1: {}", report.sfx_event_sha1);
        }
        Command::ValidateConfig { config, profile } => {
            let config = if let Some(selector) = profile {
                open_profile_manager(profile_store.clone())?.resolve(&selector)?
            } else {
                renderer::render::RenderConfig::load_config(
                    config.context("provide a config path or --profile")?,
                )?
            };
            config.validate_export_inputs()?;
            print_notification_diagnostics(&config.notifications);
            println!(
                "valid renderer config: {}x{} at {} FPS; {} execution; UI {}; watermark {}",
                config.width,
                config.height,
                config.fps,
                match config.execution_mode {
                    renderer::sono_gcc::WatchExecutionMode::Interpreter => "Sono VM",
                    renderer::sono_gcc::WatchExecutionMode::SonoGcc => "Sono-GCC",
                },
                if config.ui.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if config.watermark.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
            );
        }
        Command::DefaultConfig { output } => {
            renderer::render::RenderConfig::default().save_config(&output)?;
            println!("wrote default renderer config to {}", output.display());
        }
        Command::ClearGccCache => {
            if renderer::sono_gcc::clear_persistent_cache()? {
                println!("Sono-GCC persistent cache cleared.");
            } else {
                println!("Sono-GCC persistent cache was already empty.");
            }
        }
        Command::Cache { action } => run_cache_action(action)?,
        Command::FfmpegInfo => {
            let installation = renderer::ffmpeg::resolve()?;
            let source = match &installation.source {
                renderer::ffmpeg::FfmpegSource::SharedAddons => "shared addons",
                renderer::ffmpeg::FfmpegSource::Managed => "managed",
            };
            eprintln!("FFmpeg source: {source}");
            eprintln!("FFmpeg: {}", installation.ffmpeg.display());
            eprintln!("FFprobe: {}", installation.ffprobe.display());
            println!("{}", serde_json::to_string_pretty(&installation)?);
        }
        Command::ValidateProject {
            engine,
            resources,
            level_data,
            music,
            cover,
            custom_mv,
        } => {
            let project = renderer::project::ProjectInputs {
                engine_package: engine,
                resource_package: resources,
                level_data,
                music,
                cover,
                custom_mv,
            }
            .validate()?;
            println!(
                "Project inputs parsed and files accessible (media not decoded): {} entities, {} resources, cover={}, custom_mv={}",
                project.level.entities.len(),
                project.resources.resources.len(),
                project.cover.is_some(),
                project.custom_mv.is_some()
            );
        }
        Command::InspectEngine { engine } => println!(
            "{}",
            serde_json::to_string_pretty(&compatibility::inspect_engine(&engine)?)?
        ),
        Command::EngineIdentity { engine } => {
            let package = formats::load_engine(&engine)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&renderer::chart_studio::engine_identity(&package)?)?
            );
        }
        Command::CompileEngine { engine } => {
            let package = formats::load_engine(&engine)?;
            let result = renderer::sono_gcc::precompile(&package.watch.nodes)?;
            println!(
                "Sono-GCC cache {}: {} regions, {} / {} eligible native operation nodes, {} Watch graph nodes",
                match result.cache_hit {
                    renderer::sono_gcc::CacheHit::None => "compiled and stored",
                    renderer::sono_gcc::CacheHit::Process => "process cache hit",
                    renderer::sono_gcc::CacheHit::Persistent => "persistent cache hit",
                },
                result.compiled_regions,
                result.compiled_operations,
                result.eligible_operations,
                result.graph_nodes
            );
            println!("cache key: {}", result.cache_key);
            println!(
                "timings: lookup {:.3}s, compile {:.3}s (region codegen {:.3}s, rustc {:.3}s), DLL load {:.3}s, cache write {:.3}s, graph identity {:.3}s, disk validation {:.3}s, metadata reconstruction {:.3}s, operation summary {:.3}s",
                result.timings.cache_lookup.as_secs_f64(),
                result.timings.compilation.as_secs_f64(),
                result.timings.region_codegen.as_secs_f64(),
                result.timings.rustc_compile.as_secs_f64(),
                result.timings.native_load.as_secs_f64(),
                result.timings.cache_write.as_secs_f64(),
                result.timings.graph_identity.as_secs_f64(),
                result.timings.disk_validation.as_secs_f64(),
                result.timings.metadata_reconstruction.as_secs_f64(),
                result.timings.operation_summary.as_secs_f64()
            );
            if let Some(error) = result.cache_write_error {
                println!("persistent cache warning: {error}");
            }
        }
        Command::InspectLevel { level } => println!(
            "{}",
            serde_json::to_string_pretty(&formats::load_level(&level)?)?
        ),
        Command::ExportLevelData {
            level,
            output,
            overwrite,
        } => {
            let mut level_data = formats::load_level(&level)?;
            if level.is_dir()
                || level.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("zip")
                })
            {
                // The legacy package adapter puts item.json in this model
                // field; it is package metadata, not standalone LevelData.
                level_data.metadata = None;
            }
            let report =
                renderer::chart_studio::export_level_gzip(&level_data, &output, overwrite)?;
            println!(
                "Exported {} entities to {} ({} JSON bytes, {} gzip bytes)",
                report.entities,
                report.path.display(),
                report.json_bytes,
                report.gzip_bytes
            );
            println!("SHA-256: {}", report.sha256);
        }
        Command::ValidateLevelData {
            level,
            engine,
            json,
        } => {
            let level = formats::load_level(&level)?;
            let engine = engine.as_deref().map(formats::load_engine).transpose()?;
            let report = renderer::chart_studio::validate_level(&level, engine.as_ref());
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{} entities: {}",
                    report.entity_count,
                    if report.is_valid() {
                        "structurally valid"
                    } else {
                        "invalid"
                    }
                );
                for warning in &report.warnings {
                    println!("warning: {warning}");
                }
                for error in &report.errors {
                    println!("error: {error}");
                }
            }
            if !report.is_valid() {
                anyhow::bail!("LevelData validation failed");
            }
        }
        Command::InspectResources { scp } => println!(
            "{}",
            serde_json::to_string_pretty(&formats::inspect_scp(&scp)?)?
        ),
        Command::Compatibility {
            engine,
            resources,
            level,
            dynamic,
            human_readable,
            json: _,
        } => {
            let report = compatibility::analyze_with_options(&engine, &resources, &level, dynamic)?;
            if human_readable {
                print!("{}", compatibility::render_human_readable(&report));
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }
        Command::EvaluateNode { engine, node } => {
            let package = formats::load_engine(&engine)?;
            let mut vm = renderer::runtime::WatchVm::new(&package.watch.nodes);
            let value = vm.execute(node)?;
            println!("value: {value}");
            println!(
                "display list:\n{}",
                serde_json::to_string_pretty(&vm.display_list)?
            );
        }
        Command::RunWatch {
            engine,
            level,
            watch_trace_config,
            watch_trace_output,
            resources,
            time,
            execution_mode,
            watch_compatibility_policy,
            watch_random_seed,
            single_threaded,
            watch_workers,
            profile,
            no_vm_accounting,
            level_options,
            width,
            height,
            render_layers,
            output,
            display_list_output,
            trace_draws,
            render_diagnostics_output,
            diagnostic_entity,
            diagnostic_stage,
            diagnostic_events,
        } => {
            let package = formats::load_engine(&engine)?;
            let level = formats::load_level(&level)?;
            let defaults: std::collections::BTreeMap<_, _> =
                package.metadata.resource_defaults().into_iter().collect();
            let resource_path = resources.as_deref();
            let has_skin_bindings = package
                .watch
                .skin
                .get("sprites")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let has_effect_bindings = package
                .watch
                .effect
                .get("clips")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let has_particle_bindings = package
                .watch
                .particle
                .get("effects")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|bindings| !bindings.is_empty());
            let needs_resources = has_skin_bindings
                || has_effect_bindings
                || has_particle_bindings
                || ["skins", "backgrounds", "effects", "particles"]
                    .iter()
                    .any(|category| defaults.contains_key(*category));
            let resource_path = match (resource_path, needs_resources) {
                (Some(path), _) => Some(path),
                (None, true) => {
                    let selected = defaults
                        .iter()
                        .map(|(category, name)| format!("{category}={name}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    anyhow::bail!(
                        "required resource package is missing (--resources); engine selections: {selected}"
                    )
                }
                (None, false) => None,
            };
            let mut runtime = renderer::watch_runtime::WatchRuntime::new(&package.watch, &level)?;
            runtime.set_compatibility_policy(watch_compatibility_policy.into());
            runtime.set_random_seed(watch_random_seed)?;
            let execution_trace = if watch_trace_output.is_some() {
                let config = watch_trace_config
                    .as_ref()
                    .map(|path| -> Result<_> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
                    .transpose()?
                    .unwrap_or_default();
                Some(renderer::watch_diagnostics::WatchTrace::new(config)?)
            } else {
                None
            };
            runtime.context.execution_trace = execution_trace.clone();
            if let Some(trace) = &execution_trace {
                trace
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Watch trace lock poisoned"))?
                    .particle_names = renderer::offline::particle_effect_bindings(&package.watch)?;
                trace
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Watch trace lock poisoned"))?
                    .effect_names = renderer::offline::effect_clip_bindings(&package.watch)?;
            }
            runtime.set_draw_tracing(trace_draws);
            runtime.set_execution_mode(execution_mode.into());
            runtime.set_parallel_updates(!single_threaded, watch_workers)?;
            runtime.set_profiling(profile);
            runtime.set_vm_accounting(!no_vm_accounting);
            println!("Watch execution: {}", runtime.execution_mode_status());
            println!("Watch scheduling: {}", runtime.parallel_updates_status());
            match (diagnostic_entity, diagnostic_stage) {
                (Some(entity_id), Some(stage)) => {
                    let stage = parse_lifecycle_stage(&stage)?;
                    runtime.set_diagnostic_target(Some(
                        renderer::watch_runtime::WatchDiagnosticTarget {
                            entity_id,
                            stage,
                            event_capacity: diagnostic_events,
                        },
                    ))?;
                }
                (None, None) => {}
                _ => anyhow::bail!(
                    "--diagnostic-entity and --diagnostic-stage must be provided together"
                ),
            }
            runtime.bind_engine_rom(&package.rom)?;
            runtime.bind_engine_option_defaults(&package.configuration)?;
            runtime.bind_engine_ui_configuration(&package.configuration)?;
            let option_overrides = level_options
                .iter()
                .map(|value| parse_level_option_override(value))
                .collect::<Result<Vec<_>>>()?;
            runtime.bind_engine_option_overrides(&package.configuration, &option_overrides)?;
            for (index, value) in &option_overrides {
                println!("Level Option[{index}] override: {value}");
            }
            if width == 0 || height == 0 {
                anyhow::bail!("Watch frame dimensions must be positive");
            }
            runtime.set_screen_aspect_ratio(f64::from(width) / f64::from(height))?;
            let mut particle_assets_for_render = None;
            let mut particle_bindings_for_render = std::collections::BTreeMap::new();
            let skin_assets = if let Some(resource_path) = resource_path {
                let skin_name = defaults
                    .get("skins")
                    .map(String::as_str)
                    .or(package.metadata.skin_name.as_deref());
                let skin_name = match (skin_name, has_skin_bindings) {
                    (Some(name), _) => Some(name),
                    (None, true) => anyhow::bail!(
                        "engine declares skin sprites but has no selected/default skin resource"
                    ),
                    (None, false) => None,
                };
                let assets = skin_name
                    .map(|name| {
                        formats::load_skin_assets(resource_path, name)
                            .with_context(|| format!("resolving selected skin resource {name:?}"))
                    })
                    .transpose()?;
                if let Some(assets) = assets.as_ref() {
                    let sprite_names = assets.sprites.keys().cloned().collect();
                    runtime.bind_skin_sprite_names(&sprite_names)?;
                    println!(
                        "selected skin: {} ({} sprites)",
                        skin_name.unwrap_or("unspecified"),
                        assets.sprites.len()
                    );
                }
                let effect_name = defaults
                    .get("effects")
                    .map(String::as_str)
                    .or(package.metadata.effect_name.as_deref());
                if effect_name.is_none() && has_effect_bindings {
                    anyhow::bail!(
                        "engine declares effect clips but has no selected/default effect resource"
                    );
                }
                if let Some(effect_name) = effect_name {
                    let names = formats::load_effect_clip_names(resource_path, effect_name)
                        .with_context(|| {
                            format!("resolving selected effect resource {effect_name:?}")
                        })?;
                    if package
                        .watch
                        .effect
                        .get("clips")
                        .and_then(serde_json::Value::as_array)
                        .is_some()
                    {
                        runtime.bind_effect_clip_names(&names)?;
                    }
                    println!(
                        "selected effect resource: {effect_name} ({} clips)",
                        names.len()
                    );
                }
                let particle_name = defaults
                    .get("particles")
                    .map(String::as_str)
                    .or(package.metadata.particle_name.as_deref());
                if particle_name.is_none() && has_particle_bindings {
                    anyhow::bail!(
                        "engine declares particle effects but has no selected/default particle resource"
                    );
                }
                if let Some(particle_name) = particle_name {
                    let names = formats::load_particle_effect_names(resource_path, particle_name)
                        .with_context(|| {
                        format!("resolving selected particle resource {particle_name:?}")
                    })?;
                    if package
                        .watch
                        .particle
                        .get("effects")
                        .and_then(serde_json::Value::as_array)
                        .is_some()
                    {
                        runtime.bind_particle_effect_names(&names)?;
                    }
                    if output.is_some() && render_layers.layers().particles {
                        let particle_assets =
                            formats::load_particle_assets(resource_path, particle_name)?;
                        let bindings = renderer::offline::particle_effect_bindings(&package.watch)?;
                        particle_bindings_for_render = bindings
                            .into_iter()
                            .filter(|(_, name)| particle_assets.effects.contains_key(name))
                            .collect();
                        particle_assets_for_render = Some(particle_assets);
                    }
                    println!(
                        "selected particle resource: {particle_name} ({} effects)",
                        names.len()
                    );
                }
                assets
            } else {
                None
            };
            let background_assets = match (
                resource_path,
                defaults
                    .get("backgrounds")
                    .map(String::as_str)
                    .or(package.metadata.background_name.as_deref()),
            ) {
                (Some(resources), Some(background_name)) => {
                    let assets = formats::load_background_assets(resources, background_name)
                        .with_context(|| {
                            format!("resolving selected background resource {background_name:?}")
                        })?;
                    let natural_aspect = f64::from(assets.width) / f64::from(assets.height);
                    let quad = assets
                        .data
                        .runtime_quad(f64::from(width) / f64::from(height), natural_aspect)?;
                    runtime.set_runtime_background_quad(quad)?;
                    println!(
                        "selected background: {background_name} ({}x{}, fit={})",
                        assets.width, assets.height, assets.data.fit
                    );
                    Some(assets)
                }
                (Some(_), None) | (None, _) => None,
            };
            let frame_started = std::time::Instant::now();
            let execution = runtime.frame(time);
            let frame_elapsed = frame_started.elapsed();
            if let (Some(trace), Some(path)) = (&execution_trace, &watch_trace_output) {
                trace
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Watch trace lock poisoned"))?
                    .write_jsonl(path)?;
            }
            let report = execution?;
            for warning in runtime.compatibility_warnings() {
                eprintln!("{warning}");
            }
            println!(
                "Watch frame execution elapsed: {:.3}s",
                frame_elapsed.as_secs_f64()
            );
            if profile {
                let profile = &report.runtime_profile;
                println!(
                    "Watch CPU profile: frame={:.3}ms preprocess={:.3}ms updateSpawn={:.3}ms scheduling={:.3}ms activation={:.3}ms updateSequential={:.3}ms updateParallel={:.3}ms report={:.3}ms; callbacks={} evals={} dispatches={} copiedMemoryEntries={}; parallelCallbacks={} orderedCallbacks={} speculativeCallbacks={} batches={} workerSlots={} workerBusy={:.3}ms workerSlotTime={:.3}ms schedulerWait~={:.3}ms",
                    profile.frame.as_secs_f64() * 1000.0,
                    profile.preprocess.as_secs_f64() * 1000.0,
                    profile.update_spawn.as_secs_f64() * 1000.0,
                    profile.scheduling.as_secs_f64() * 1000.0,
                    profile.activation.as_secs_f64() * 1000.0,
                    profile.update_sequential.as_secs_f64() * 1000.0,
                    profile.update_parallel.as_secs_f64() * 1000.0,
                    profile.report_materialization.as_secs_f64() * 1000.0,
                    profile.callbacks,
                    profile.evaluations,
                    profile.function_dispatches,
                    profile.memory_entries_copied,
                    profile.parallel_callbacks,
                    profile.parallel_ordered_callbacks,
                    profile.parallel_speculative_callbacks,
                    profile.parallel_batches,
                    profile.parallel_worker_slots,
                    profile.parallel_worker_busy.as_secs_f64() * 1000.0,
                    profile.parallel_worker_slot_time.as_secs_f64() * 1000.0,
                    profile.parallel_scheduler_wait.as_secs_f64() * 1000.0,
                );
                if profile.sono_gcc_regions_executed != 0 {
                    let size_buckets = SONO_GCC_REGION_SIZE_BUCKET_LABELS
                        .iter()
                        .enumerate()
                        .map(|(index, label)| {
                            format!(
                                "{label}:{} regions/{} cuts/{} VM fallbacks",
                                profile.sono_gcc_region_size_executions[index],
                                profile.sono_gcc_region_size_cut_calls[index],
                                profile.sono_gcc_small_region_bypasses[index]
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!(
                        "Sono-GCC native region sizes (native operation nodes: executions/cuts): {size_buckets}"
                    );
                    let mut cut_operations = profile
                        .sono_gcc_cut_operation_counts
                        .iter()
                        .collect::<Vec<_>>();
                    cut_operations.sort_by(|left, right| right.1.cmp(left.1));
                    let cut_summary = cut_operations
                        .into_iter()
                        .take(12)
                        .map(|(name, count)| format!("{name}={count}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Sono-GCC hottest VM cut operations: {cut_summary}");
                    let mut callbacks = profile
                        .sono_gcc_callback_hotspots
                        .iter()
                        .collect::<Vec<_>>();
                    callbacks.sort_by(|left, right| right.1 .0.cmp(&left.1 .0));
                    let callback_summary = callbacks
                        .into_iter()
                        .take(12)
                        .map(|(name, (regions, cuts))| {
                            format!("{name}={regions} regions/{cuts} cuts")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Sono-GCC hottest callbacks: {callback_summary}");
                }
            }
            println!(
                "Runtime Update [time, deltaTime, scaledTime, reserved]: {:?}; after callbacks={:?}; timescale={}",
                report.runtime_update, report.runtime_update_after_callbacks, report.timescale
            );
            println!(
                "runtime entities: {}, active entities: {}, callbacks: {}, Draw commands: {}",
                report.runtime_entity_count,
                report.active_entity_count,
                report.callbacks.len(),
                report.display_list.sprites.len()
            );
            println!(
                "raw Draw SHA-1: {}",
                renderer::offline::hash_draw_commands(&report.display_list)
            );
            println!(
                "Runtime Background quad (BL,TL,TR,BR): {:?}",
                report
                    .runtime_background_quad
                    .chunks_exact(2)
                    .map(|xy| [xy[0], xy[1]])
                    .collect::<Vec<_>>()
            );
            println!(
                "Runtime Skin Transform (row-major values 0..15): {:?}",
                report.runtime_skin_transform
            );
            let mut stages = std::collections::BTreeMap::new();
            for callback in &report.callbacks {
                *stages
                    .entry(format!("{:?}", callback.stage))
                    .or_insert(0usize) += 1;
            }
            println!(
                "resolved entities: {} / {}",
                runtime.resolved_level_entity_count(),
                level.entities.len()
            );
            println!("runtime entities: {}", runtime.entities.len());
            println!("callbacks executed: {}", report.callbacks.len());
            println!("VM node evaluations: {}", report.vm_evaluations);
            println!(
                "Sono-GCC regions executed this frame: {}",
                report.runtime_profile.sono_gcc_regions_executed
            );
            println!(
                "Sono-GCC VM-to-native scalar cut calls this frame: {}",
                report.runtime_profile.sono_gcc_vm_to_native_cut_calls
            );
            println!(
                "Sono-GCC regions needing heap cut-input storage this frame: {}",
                report.runtime_profile.sono_gcc_overflow_input_regions
            );
            println!("timeline: {:?}", report.timeline);
            println!("callbacks by stage: {}", serde_json::to_string(&stages)?);
            println!("new spawned entities: {}", report.spawned.len());
            println!(
                "Spawn requests produced: {}",
                report.spawn_requests_produced
            );
            println!(
                "active entities: {}",
                runtime
                    .entities
                    .iter()
                    .filter(|entity| entity.active)
                    .count()
            );
            let mut active_archetypes = std::collections::BTreeMap::new();
            for entity in runtime.entities.iter().filter(|entity| entity.active) {
                *active_archetypes
                    .entry(entity.archetype.clone())
                    .or_insert(0usize) += 1;
            }
            println!(
                "active archetypes: {}",
                serde_json::to_string(&active_archetypes)?
            );
            println!(
                "display-list sprites: {}",
                report.display_list.sprites.len()
            );
            println!(
                "Draw operations executed: {}",
                report.function_counts.get("Draw").copied().unwrap_or(0)
            );
            println!(
                "HasSkinSprite checks: {} (present={}, absent={})",
                report.skin_checks.len(),
                report
                    .skin_checks
                    .iter()
                    .filter(|(_, present)| *present)
                    .count(),
                report
                    .skin_checks
                    .iter()
                    .filter(|(_, present)| !*present)
                    .count()
            );
            if no_vm_accounting {
                println!("executed Watch operations: disabled by --no-vm-accounting");
            } else {
                println!(
                    "executed Watch operations: {}",
                    serde_json::to_string(&report.function_counts)?
                );
            }
            println!("Watch debug events:");
            for event in &report.debug_events {
                match event {
                    renderer::runtime::DebugEvent::Log {
                        value,
                        entity_id,
                        callback,
                        node,
                        ..
                    } => println!(
                        "  DebugLog entity={entity_id:?} callback={callback:?} node={node}: {}",
                        renderer::runtime::DebugEvent::display_value(*value)
                    ),
                    renderer::runtime::DebugEvent::Pause {
                        entity_id,
                        callback,
                        node,
                        ..
                    } => println!(
                        "  DebugPause entity={entity_id:?} callback={callback:?} node={node}"
                    ),
                }
            }
            for entity_id in &report.spawned {
                let entity = &runtime.entities[*entity_id];
                println!(
                    "spawned entity {}: archetype={}, active={}, initialized={}",
                    entity.id, entity.archetype, entity.active, entity.initialized
                );
            }
            for callback in report
                .callbacks
                .iter()
                .filter(|c| c.entity_id.is_some())
                .take(5)
            {
                println!(
                    "callback trace: entity={} archetype={} stage={:?} node={}",
                    callback.entity_id.unwrap_or_default(),
                    callback.archetype,
                    callback.stage,
                    callback.node
                );
            }
            let skin_bindings = skin_bindings(&package.watch)?;
            let mut draw_sources = std::collections::BTreeMap::new();
            for sprite in &report.display_list.sprites {
                if let Some(source) = &sprite.provenance {
                    *draw_sources
                        .entry((source.clone(), sprite.sprite_id))
                        .or_insert(0usize) += 1;
                }
            }
            for ((source, sprite_id), count) in draw_sources {
                let name = skin_bindings
                    .get(&sprite_id)
                    .map(String::as_str)
                    .unwrap_or("<unresolved>");
                println!(
                    "Draw provenance: entity={:?} archetype={:?} callback={:?} callback_node={:?} Draw_node={} sprite_id={sprite_id} sprite={name:?} commands={count}",
                    source.entity_id,
                    source.archetype,
                    source.callback,
                    source.callback_node,
                    source.draw_node
                );
            }
            if let Some(path) = display_list_output {
                let bytes = serde_json::to_vec_pretty(&report.display_list)?;
                std::fs::write(&path, bytes)
                    .with_context(|| format!("writing display list {}", path.display()))?;
                println!("wrote display list JSON to {}", path.display());
            }
            if let Some(path) = output {
                let skin = skin_assets
                    .as_ref()
                    .context("--output requires --resources with a valid skin package")?;
                let runtime_background = report
                    .runtime_background_quad
                    .chunks_exact(2)
                    .map(|coordinates| [coordinates[0], coordinates[1]])
                    .collect::<Vec<_>>();
                let runtime_background: [[f64; 2]; 4] = runtime_background
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Runtime Background quad has invalid length"))?;
                let background = background_assets
                    .as_ref()
                    .map(|assets| (assets, runtime_background));
                let mut ppm = report
                    .display_list
                    .render_skin_ppm_with_runtime_transform_and_background(
                        width,
                        height,
                        f64::from(width) / f64::from(height),
                        skin,
                        &skin_bindings,
                        &report.runtime_skin_transform,
                        background,
                    )?;
                if render_layers.layers().particles {
                    if let Some(assets) = particle_assets_for_render.as_ref() {
                        let instances = runtime
                            .context
                            .particle_instances
                            .read()
                            .map_err(|_| {
                                anyhow::anyhow!("particle instance state lock was poisoned")
                            })?
                            .instances()
                            .values()
                            .cloned()
                            .collect::<Vec<_>>();
                        let draws = renderer::particles::render_instances_traced(
                            assets,
                            &particle_bindings_for_render,
                            &instances,
                            time,
                            execution_trace.as_ref(),
                            Some((width, height, &report.runtime_particle_transform)),
                        )?;
                        let mut rgb = renderer::offline::ppm_rgb_payload(&ppm, width, height)?;
                        renderer::runtime::DisplayList::composite_particle_sprites(
                            &mut rgb,
                            width,
                            height,
                            f64::from(width) / f64::from(height),
                            assets,
                            &draws,
                            &report.runtime_particle_transform,
                        )?;
                        ppm = {
                            let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
                            bytes.extend_from_slice(&rgb);
                            bytes
                        };
                    }
                }
                let ui = render_layers.ui_config()?;
                if ui.enabled {
                    let mut rgb = renderer::offline::ppm_rgb_payload(&ppm, width, height)?;
                    renderer::render_ui::render_overlay(
                        &mut rgb,
                        width,
                        height,
                        report.timeline.unwrap_or(time),
                        0.0,
                        1.0,
                        &ui,
                        |provider| match provider {
                            renderer::render_ui::UiValueProvider::Fixed { value }
                            | renderer::render_ui::UiValueProvider::External { value } => {
                                Ok(*value)
                            }
                            renderer::render_ui::UiValueProvider::Memory { block_id, index } => {
                                Ok(runtime.global_memory.get(*block_id, *index))
                            }
                            renderer::render_ui::UiValueProvider::Progress => Ok(time),
                            renderer::render_ui::UiValueProvider::EngineMetric { key } => {
                                anyhow::bail!(
                                "engine metric provider {key:?} has no authoritative Watch source"
                            )
                            }
                            renderer::render_ui::UiValueProvider::Accuracy => anyhow::bail!(
                                "accuracy provider is unsupported without a verified Watch source"
                            ),
                            renderer::render_ui::UiValueProvider::JudgmentDerived { mapping } => {
                                anyhow::bail!(
                                    "judgment-derived provider {mapping:?} requires caller logic"
                                )
                            }
                        },
                    )?;
                    ppm = {
                        let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
                        bytes.extend_from_slice(&rgb);
                        bytes
                    };
                }
                println!(
                    "pre-encode RGB SHA-1: {}",
                    renderer::offline::hash_rgb(&renderer::offline::ppm_rgb_payload(
                        &ppm, width, height
                    )?)
                );
                std::fs::write(&path, ppm)
                    .with_context(|| format!("writing rendered frame {}", path.display()))?;
                println!(
                    "wrote skin-rendered {}x{} PPM to {}",
                    width,
                    height,
                    path.display()
                );
            }
            if let (Some(trace), Some(path)) = (&execution_trace, &watch_trace_output) {
                trace
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Watch trace lock poisoned"))?
                    .write_jsonl(path)?;
            }
            if let Some(path) = render_diagnostics_output {
                let skin = skin_assets
                    .as_ref()
                    .context("--render-diagnostics-output requires loaded --resources")?;
                let diagnostics = report
                    .display_list
                    .skin_render_diagnostics_with_runtime_transform(
                        width,
                        height,
                        f64::from(width) / f64::from(height),
                        skin,
                        &skin_bindings,
                        &report.runtime_skin_transform,
                    )?;
                std::fs::write(&path, serde_json::to_vec_pretty(&diagnostics)?)
                    .with_context(|| format!("writing render diagnostics {}", path.display()))?;
                println!("wrote per-Draw render diagnostics to {}", path.display());
            }
        }
        Command::RenderDisplayList {
            display_list,
            output,
            width,
            height,
        } => {
            let list: renderer::runtime::DisplayList =
                serde_json::from_slice(&std::fs::read(&display_list)?)?;
            std::fs::write(&output, list.render_ppm(width, height)?)?;
            println!("wrote {}x{} PPM to {}", width, height, output.display());
        }
        Command::RenderVideo {
            engine,
            resources,
            level,
            music,
            output,
            cover,
            custom_mv,
            no_mv,
            mv_background,
            apply_engine_background_rotation,
            particle_resources,
            particle_name,
            start_time,
            duration,
            whole_chart,
            ui_profile,
            watermark_profile,
            no_watermark,
            moving_watermark,
            no_moving_watermark,
            fps,
            video_crf,
            video_preset,
            notifications_config,
            no_notifications,
            width,
            height,
            trace_entity,
            level_options,
            render_layers,
            backend,
            execution_mode,
            watch_compatibility_policy,
            watch_random_seed,
            single_threaded,
            watch_workers,
            profile,
            profile_frames,
            validate_determinism,
            sequential_sfx_prepass,
            concurrent_sfx_prepass,
            sequential_frames,
        } => {
            let _optional_cover = cover;
            let option_overrides = level_options
                .iter()
                .map(|value| parse_level_option_override(value))
                .collect::<Result<Vec<_>>>()?;
            let mut ui = render_layers.ui_config()?;
            if let Some(path) = ui_profile {
                let mut profile_loader = renderer::render::RenderConfig::default();
                profile_loader.set_ui_profile(path)?;
                ui = profile_loader.ui;
                render_layers.apply_ui_overrides(&mut ui)?;
                ui.validate()?;
            }
            let mut watermark = renderer::watermark::WatermarkConfig::default();
            if let Some(path) = watermark_profile {
                let mut profile_loader = renderer::render::RenderConfig::default();
                profile_loader.set_watermark_profile(path)?;
                watermark = profile_loader.watermark;
            }
            if no_watermark {
                watermark.enabled = false;
                watermark.moving.enabled = false;
            }
            if moving_watermark {
                watermark.enabled = true;
                watermark.moving.enabled = true;
            }
            if no_moving_watermark {
                watermark.moving.enabled = false;
            }
            let mut notifications = if let Some(path) = notifications_config {
                match std::fs::read(&path) {
                    Ok(bytes) => renderer::notifications::NotificationConfig::from_json_or_disabled(&bytes),
                    Err(_) => renderer::notifications::NotificationConfig::disabled_with_warning(
                        "notification configuration file could not be read; notifications were disabled",
                    ),
                }
            } else {
                renderer::notifications::NotificationConfig::default()
            };
            if no_notifications {
                notifications.enabled = false;
            }
            print_notification_diagnostics(&notifications);
            let config = renderer::render::RenderConfig {
                engine,
                resources,
                level,
                skin: None,
                music: Some(music),
                mv: custom_mv,
                mv_enabled: !no_mv,
                mv_background,
                apply_engine_background_rotation,
                resource_overrides: renderer::render::ResourceOverrides {
                    particles: particle_resources,
                    particle_name,
                    ..Default::default()
                },
                output: Some(output.clone()),
                start_time: start_time.unwrap_or(if whole_chart { 0.0 } else { 14.0 }),
                duration: duration.unwrap_or(2.0),
                whole_chart,
                fps,
                width,
                height,
                video_crf: video_crf.unwrap_or(18),
                video_preset: video_preset
                    .map(Into::into)
                    .unwrap_or(renderer::render::VideoPreset::Medium),
                level_options: option_overrides
                    .into_iter()
                    .map(|(index, value)| renderer::render::LevelOptionValue { index, value })
                    .collect(),
                layers: render_layers.layers(),
                ui,
                watermark,
                notifications,
                job_manager: renderer::job_manager::JobManagerConfig::default(),
                trace_entity_id: trace_entity,
                backend: match backend {
                    BackendArg::Cpu => renderer::render::RenderBackend::Cpu,
                    BackendArg::Wgpu => renderer::render::RenderBackend::Wgpu,
                },
                execution_mode: execution_mode.into(),
                watch_compatibility_policy: watch_compatibility_policy.into(),
                watch_random_seed,
                parallel_watch_updates: !single_threaded,
                watch_workers,
                compile_sono_gcc_on_demand: true,
                profile,
                profile_frames,
                validate_determinism,
                concurrent_sfx_prepass: sequential_frames
                    && concurrent_sfx_prepass
                    && !sequential_sfx_prepass,
                frame_pipeline: !sequential_frames,
            };
            let report =
                render_with_diagnostics(&config, renderer::export_progress::terminal_sink());
            flush_cli_notifications();
            let report = report?;
            print_render_warnings(&report);
            println!("Watch execution: {}", report.watch_execution_status);
            print_performance_record_summary(&report.performance);
            println!("render diagnostic: {}", report.diagnostics.id);
            println!(
                "Sono-GCC scalar regions executed during rendering: {}",
                report.stream_sono_gcc_regions_executed
            );
            println!(
                "Sono-GCC VM-to-native scalar cut calls during rendering: {}",
                report.stream_sono_gcc_vm_to_native_cut_calls
            );
            println!("wrote MP4 to {}", output.display());
            println!(
                "submitted deterministic frames: {}",
                report.submitted_frames
            );
            println!("FFmpeg source: {}", report.ffmpeg_source);
            println!("FFmpeg: {}", report.ffmpeg_path);
            println!("FFprobe: {}", report.ffprobe_path);
            println!(
                "Watch timeline: {:.6}s..{:.6}s",
                report.timeline_start, report.timeline_end
            );
            println!("FPS: {}", report.fps);
            println!("Level bgmOffset: {}s", report.bgm_offset);
            if let Some(mv) = &report.mv {
                println!(
                    "MV: {} (canonical media time, contain fit, source audio ignored)",
                    mv.display()
                );
            }
            if let Some(end) = &report.chart_end {
                println!(
                    "Whole-chart end: {:.6}s ({})",
                    end.watch_end, end.confidence
                );
                println!("End evidence: {} finite entities, {} persistent non-input entities, {} discovery frames; BGM Watch end {:?}s, SFX end {:.6}s; persistent endpoint policy threshold {:.6} (spawning timeline units)",
                    end.finite_entity_count, end.persistent_entity_count, end.discovery_frames,
                    end.bgm_watch_end, end.sfx_watch_end, end.persistent_lifetime_threshold);
            }
            println!(
                "Requested start: {:.6}s; frame-aligned start: {:.6}s",
                report.timeline.requested_start, report.timeline.watch_start
            );
            if let Some(samples) = report.bgm_pcm_sample_frames {
                println!("BGM PCM: {samples} stereo sample frames at 44100 Hz; desired media interval [{:.6},{:.6})s; source duration {:.6}s",
                    report.timeline.output_to_media(0.0), report.timeline.output_to_media(report.timeline.duration), report.bgm_source_duration.unwrap_or(0.0));
            }
            if render_layers.layers().bgm {
                println!(
                    "Audio mapping: music source {:.6}s, leading silence {:.6}s",
                    report.audio_window.source_start, report.audio_window.leading_silence
                );
            }
            if report.deterministic_frame_hashes_match == Some(true) {
                println!(
                    "pre-encode deterministic frame hashes match: {} ({})",
                    true,
                    report.first_pass_hash.as_deref().unwrap_or("")
                );
            } else {
                println!(
                    "full draw/RGB determinism validation: not run (use --validate-determinism)"
                );
            }
            println!("rendered RGB sequence SHA-1: {}", report.second_pass_hash);
            println!(
                "SFX event sequence SHA-1: {} ({:?})",
                report.sfx_event_sha1, report.sfx_event_counts
            );
            println!(
                "SFX decoding: {} playback events, {} named resources used, {} new unique payloads decoded, {} FFmpeg decodes",
                report.sfx_playback_events,
                report.sfx_resources_used,
                report.sfx_decoded_unique_resources,
                report.sfx_ffmpeg_decodes
            );
            println!(
                "Watch playback traversals: {} (SFX collected during rendering)",
                report.watch_traversals
            );
            println!(
                "SFX event pass workload: {} runtime frames, {} callbacks, {} evaluations",
                report.event_pass_runtime_frames,
                report.event_pass_callbacks,
                report.event_pass_evaluations
            );
            println!(
                "stream workload: {} runtime frames, {} callbacks, {} evaluations",
                report.stream_runtime_frames, report.stream_callbacks, report.stream_evaluations
            );
            if let Some(pipeline) = &report.frame_pipeline {
                println!(
                    "frame pipeline: queue {}/{}, producer Watch {:.1}ms + snapshot {:.1}ms, blocked {:.1}ms; queue residence {:.1}ms, consumer waiting {:.1}ms, render {:.1}ms, FFmpeg output {:.1}ms; first frame {:.1}ms, render phase {:.1}ms",
                    pipeline.queue_high_water,
                    pipeline.queue_capacity,
                    pipeline.producer_watch_ms,
                    pipeline.producer_snapshot_ms,
                    pipeline.producer_blocked_ms,
                    pipeline.queue_residence_ms,
                    pipeline.consumer_waiting_ms,
                    pipeline.render_ms,
                    pipeline.output_ms,
                    pipeline.first_frame_latency_ms,
                    pipeline.render_phase_ms,
                );
            }
            println!("FFprobe:\n{}", serde_json::to_string_pretty(&report.probe)?);
            println!(
                "per-frame Watch/render diagnostics:\n{}",
                serde_json::to_string_pretty(&report.frame_diagnostics)?
            );
            println!("SFX mixed: {}", render_layers.layers().sfx);
        }
        Command::Gui => renderer::gui::launch()?,
        Command::Jobs { store, action } => run_job_command(store, profile_store, action)?,
        Command::History {
            store,
            queue_store,
            action,
        } => run_history_command(store, queue_store, action)?,
        Command::Manifest { action } => run_manifest_command(action)?,
        Command::Profiles { action } => run_profile_command(profile_store, action)?,
        Command::Performance { action } => run_performance_command(action)?,
        Command::Diagnostics { action } => run_diagnostics_command(action)?,
        Command::Regression { action } => run_regression_command(action)?,
    }
    Ok(())
}

fn run_media_action(action: MediaAction) -> Result<()> {
    use renderer::media_acquisition::{
        AcquisitionControl, AcquisitionMode, AcquisitionProgress, AcquisitionStage,
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    match action {
        MediaAction::ToolInfo { json } => {
            let info = renderer::media_acquisition::inspect_tool()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else if info.available {
                println!(
                    "yt-dlp {} is ready ({}).",
                    info.version.as_deref().unwrap_or("unknown"),
                    info.source
                );
                println!(
                    "Pinned release: {}; {}.",
                    info.pinned_version, info.update_policy
                );
            } else {
                println!("Managed yt-dlp is not installed. It is provisioned from the pinned official release when URL inspection is requested.");
                println!(
                    "Pinned release: {}; {}.",
                    info.pinned_version, info.update_policy
                );
            }
        }
        MediaAction::InspectUrl { url, json } => {
            let control = AcquisitionControl::default();
            let progress = Arc::new(Mutex::new(AcquisitionProgress::default()));
            let info = renderer::media_acquisition::inspect_url(&url, &control, &progress)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("Title: {}", info.title.as_deref().unwrap_or("unavailable"));
                println!(
                    "Uploader: {}",
                    info.uploader.as_deref().unwrap_or("unavailable")
                );
                println!(
                    "Duration: {}",
                    info.duration_seconds
                        .map(|value| format!("{value:.3}s"))
                        .unwrap_or_else(|| "unavailable".into())
                );
                println!(
                    "Formats: {}",
                    info.format_count
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".into())
                );
                println!(
                    "Playlist: {}; live/upcoming: {}",
                    info.is_playlist, info.is_live_or_upcoming
                );
            }
        }
        MediaAction::Download {
            url,
            mode,
            output_dir,
            project,
            json,
        } => {
            let mode: AcquisitionMode = mode.into();
            let mut project_doc = project
                .as_ref()
                .map(renderer::project::RenderProject::load)
                .transpose()?;
            let destination = output_dir.unwrap_or_else(|| {
                project
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map(|parent| parent.join("media"))
                    .unwrap_or_else(renderer::media_acquisition::default_media_directory)
            });
            let control = AcquisitionControl::default();
            let progress = Arc::new(Mutex::new(AcquisitionProgress::default()));
            let finished = Arc::new(AtomicBool::new(false));
            let watcher_finished = finished.clone();
            let watcher_progress = progress.clone();
            let monitor = if json {
                None
            } else {
                Some(std::thread::spawn(move || {
                    let mut previous = None;
                    while !watcher_finished.load(Ordering::Acquire) {
                        let snapshot = watcher_progress
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .clone();
                        let description = match snapshot.percent {
                            Some(percent) => format!("{} ({percent:.1}%)", snapshot.detail),
                            None => snapshot.detail,
                        };
                        if previous.as_deref() != Some(description.as_str()) {
                            eprintln!("media: {description}");
                            previous = Some(description);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(400));
                    }
                }))
            };
            let acquired =
                renderer::media_acquisition::acquire(&url, mode, &destination, &control, &progress);
            finished.store(true, Ordering::Release);
            if let Some(monitor) = monitor {
                let _ = monitor.join();
            }
            let acquired = acquired?;
            if let (Some(path), Some(project_doc)) = (project.as_ref(), project_doc.as_mut()) {
                assign_acquired_media(project_doc, mode, &acquired);
                project_doc
                    .save(path)
                    .context("saving project after media acquisition")?;
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&acquired)?);
            } else {
                println!("Acquired {} to {}", mode.label(), acquired.path.display());
                println!(
                    "Streams: audio={}, video={}; duration={:.3}s; SHA-256={}",
                    acquired.has_audio,
                    acquired.has_video,
                    acquired.duration_seconds,
                    acquired.sha256
                );
                if let Some(path) = project {
                    println!("Assigned to project {}", path.display());
                }
            }
            let mut final_progress = progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            final_progress.stage = AcquisitionStage::Complete;
        }
    }
    Ok(())
}

fn assign_acquired_media(
    project: &mut renderer::project::RenderProject,
    mode: renderer::media_acquisition::AcquisitionMode,
    media: &renderer::media_acquisition::AcquiredMedia,
) {
    if mode.has_audio() {
        project.inputs.music = Some(media.path.clone());
    }
    if mode.has_video() {
        project.inputs.mv = Some(media.path.clone());
    }
}

fn run_project_command(profile_store: Option<PathBuf>, action: ProjectAction) -> Result<()> {
    match action {
        ProjectAction::Inspect { path } => {
            let project = renderer::project::RenderProject::load(&path)?;
            println!("Project: {}", path.display());
            println!(
                "Schema: {} / version {}",
                project.format, project.schema_version
            );
            if let Some(name) = &project.metadata.name {
                println!("Name: {name}");
            }
            println!(
                "Profile: {}",
                project
                    .profile
                    .as_ref()
                    .map(|reference| format!(
                        "{} ({})",
                        reference.name.as_deref().unwrap_or("unnamed"),
                        reference.id.as_deref().unwrap_or("name reference")
                    ))
                    .unwrap_or_else(|| "none".into())
            );
            let resolved =
                project.resolved_inputs(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
            for (label, value) in [
                ("Engine", resolved.engine),
                ("Level", resolved.level),
                ("Resources", resolved.resources),
                ("Music", resolved.music),
                ("Cover", resolved.cover),
                ("MV", resolved.mv),
            ] {
                println!(
                    "{label}: {}",
                    value
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "(not set)".into())
                );
            }
            println!(
                "Overrides: {}",
                serde_json::to_string_pretty(&project.overrides)?
            );
            println!(
                "Unknown top-level fields preserved: {}",
                project.extra.len()
            );
        }
        ProjectAction::Validate { path, json } => {
            let project = renderer::project::RenderProject::load(&path)?;
            let profiles = if project.profile.is_some() {
                Some(open_profile_manager(profile_store)?)
            } else {
                None
            };
            let resolution = project.resolve_config(
                &path,
                &renderer::render::RenderConfig::default(),
                profiles.as_ref(),
                None,
            );
            let mut report = project.validate_inputs(&path)?;
            if let Err(error) = resolution {
                report.errors.push(format!(
                    "configuration/profile resolution failed: {error:#}"
                ));
            } else if let Ok((config, _)) = resolution {
                if let Err(error) = config.validate() {
                    report
                        .errors
                        .push(format!("configuration is invalid: {error:#}"));
                }
                if config.layers.bgm && config.music.as_ref().is_none_or(|music| !music.is_file()) {
                    report
                        .errors
                        .push("BGM is enabled but the project has no accessible music file".into());
                }
                if let Some(output) = config.output {
                    let parent = output
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."));
                    if parent.exists() && !parent.is_dir() {
                        report.errors.push(format!(
                            "output parent is not a directory: {}",
                            parent.display()
                        ));
                    }
                    if output.extension().is_some_and(|extension| {
                        !extension.to_string_lossy().eq_ignore_ascii_case("mp4")
                    }) {
                        report.warnings.push(format!(
                            "output extension is not .mp4: {}",
                            output.display()
                        ));
                    }
                    if output.exists() {
                        report.warnings.push(format!(
                            "output already exists and cannot be overwritten by the job queue: {}",
                            output.display()
                        ));
                    }
                } else {
                    report.warnings.push(
                        "no default output path is set; supply --output when rendering".into(),
                    );
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Project validation: {}",
                    if report.is_valid() {
                        "valid"
                    } else {
                        "needs attention"
                    }
                );
                if let Some(count) = report.level_entities {
                    println!("Level entities: {count}");
                }
                if let Some(count) = report.resource_count {
                    println!("Resources: {count}");
                }
                for error in &report.errors {
                    println!("ERROR: {error}");
                }
                for warning in &report.warnings {
                    println!("WARNING: {warning}");
                }
                if let Some(compatibility) = &report.compatibility {
                    println!(
                        "Compatibility analysis: {} assessments, {} findings",
                        compatibility.assessments.len(),
                        compatibility.structural_findings.len()
                    );
                }
            }
            if !report.is_valid() {
                anyhow::bail!("project validation failed");
            }
        }
    }
    Ok(())
}

fn open_job_manager(
    store: Option<PathBuf>,
    config: renderer::job_manager::JobManagerConfig,
) -> Result<renderer::job_manager::RenderJobManager> {
    match store {
        Some(path) => renderer::job_manager::RenderJobManager::open_at(path, config),
        None => renderer::job_manager::RenderJobManager::open(config),
    }
}

fn open_profile_manager(
    path: Option<PathBuf>,
) -> Result<renderer::config_profiles::ConfigProfileManager> {
    match path {
        Some(path) => renderer::config_profiles::ConfigProfileManager::open_at(path),
        None => renderer::config_profiles::ConfigProfileManager::open_default(),
    }
}

fn run_profile_command(store: Option<PathBuf>, action: ProfileAction) -> Result<()> {
    let manager = open_profile_manager(store)?;
    match action {
        ProfileAction::List => {
            let active = manager.active()?.map(|profile| profile.id);
            println!("ID\tACTIVE\tNAME\tRESOLUTION\tFPS\tEXECUTION");
            for profile in manager.list()? {
                println!(
                    "{}\t{}\t{}\t{}x{}\t{}\t{:?}",
                    profile.id,
                    if active.as_deref() == Some(&profile.id) {
                        "*"
                    } else {
                        ""
                    },
                    profile.name,
                    profile.config.width,
                    profile.config.height,
                    profile.config.fps,
                    profile.config.execution_mode
                );
            }
        }
        ProfileAction::Show { selector } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&manager.get(&selector)?)?
            );
        }
        ProfileAction::Create { name, config } => {
            let source = if let Some(path) = config {
                renderer::render::RenderConfig::load_config(path)?
            } else if let Some(active) = manager.active()? {
                active.config
            } else {
                renderer::render::RenderConfig::default()
            };
            let profile = manager.create(&name, source)?;
            println!("created profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Duplicate { selector, name } => {
            let profile = manager.duplicate(&selector, &name)?;
            println!("duplicated profile as {} ({})", profile.name, profile.id);
        }
        ProfileAction::Rename { selector, name } => {
            let profile = manager.rename(&selector, &name)?;
            println!("renamed profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Update { selector, config } => {
            let config = renderer::render::RenderConfig::load_config(config)?;
            let profile = manager.update(&selector, config)?;
            println!("updated profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Reset { selector } => {
            let profile = manager.reset(&selector)?;
            println!("reset profile {} to renderer defaults", profile.name);
        }
        ProfileAction::Select { selector } | ProfileAction::Use { selector } => {
            let profile = manager.select(&selector)?;
            println!("selected profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Deselect => {
            manager.select_current()?;
            println!("cleared the active profile; current configuration is unchanged");
        }
        ProfileAction::Delete { selector } => {
            let profile = manager.delete(&selector)?;
            println!("deleted profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Import { path, name } => {
            let profile = manager.import(&path, name.as_deref())?;
            println!("imported profile {} ({})", profile.name, profile.id);
        }
        ProfileAction::Export { selector, path } => {
            let profile = manager.get(&selector)?;
            manager.export(&selector, &path)?;
            println!("exported profile {} to {}", profile.name, path.display());
        }
    }
    Ok(())
}

fn print_performance_record_summary(record: &renderer::performance::RenderPerformanceRecord) {
    println!(
        "performance: {:.3}s execution; {} frames; record {}",
        record.execution_time_ms / 1000.0,
        record.frames,
        record.id
    );
    for stage in record
        .stage_timings
        .iter()
        .filter(|stage| stage.semantics == renderer::performance::StageSemantics::ExclusiveWallSpan)
    {
        println!("  {:<18} {:>9.2}ms", stage.name, stage.duration_ms);
    }
}

fn print_render_warnings(report: &renderer::video_export::ExportReport) {
    for finding in &report.diagnostics.findings {
        if finding.code == "particle_effect_skipped" {
            eprintln!("warning: {}", finding.summary);
        }
    }
}

fn render_with_diagnostics(
    config: &renderer::render::RenderConfig,
    sink: renderer::export_progress::ProgressSink,
) -> Result<renderer::video_export::ExportReport> {
    match config.render_video_with_progress(sink) {
        Ok(report) => Ok(report),
        Err(error) => {
            let chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
            let diagnostic = renderer::render_diagnostics::RenderDiagnosticRecord::from_failure(
                format!("diagnostic-cli-{}", renderer::performance::unix_time_ms()),
                None,
                config,
                None,
                Some("renderer".into()),
                &chain,
                renderer::render_diagnostics::DiagnosticOutcome::Failed,
            );
            eprintln!(
                "structured render diagnostic:\n{}",
                serde_json::to_string_pretty(&diagnostic)?
            );
            Err(error)
        }
    }
}

fn run_diagnostics_command(action: DiagnosticsAction) -> Result<()> {
    use renderer::render_diagnostics::RenderDiagnosticRecord;
    match action {
        DiagnosticsAction::Record { id, output, json } => {
            let diagnostic =
                renderer::render_diagnostics::RenderDiagnosticStore::open_default()?.get(&id)?;
            emit_diagnostic_report(diagnostic, output, json)?;
        }
        DiagnosticsAction::Job {
            id,
            store,
            output,
            json,
        } => {
            let manager = match store {
                Some(path) => renderer::job_manager::RenderJobManager::open_at(
                    path,
                    renderer::job_manager::JobManagerConfig::default(),
                )?,
                None => renderer::job_manager::RenderJobManager::open(
                    renderer::job_manager::JobManagerConfig::default(),
                )?,
            };
            let job = manager.inspect(&id)?;
            let diagnostic = if let Some(diagnostic) = job.diagnostic_report {
                diagnostic
            } else if let Some(performance_id) = job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.performance_record_id.as_deref())
            {
                let performance =
                    renderer::performance::PerformanceStore::open_default()?.get(performance_id)?;
                RenderDiagnosticRecord::from_performance(&performance)
            } else {
                let chain = job
                    .diagnostic
                    .as_deref()
                    .map(str::to_owned)
                    .into_iter()
                    .collect::<Vec<_>>();
                RenderDiagnosticRecord::from_failure(
                    format!("diagnostic-{}-legacy", job.id),
                    Some(job.id.clone()),
                    &job.config,
                    job.progress
                        .as_ref()
                        .map(|progress| format!("{:?}", progress.phase)),
                    job.failure_kind
                        .map(|kind| format!("{kind:?}").to_ascii_lowercase()),
                    &chain,
                    match job.state {
                        renderer::job_manager::JobState::Cancelled => {
                            renderer::render_diagnostics::DiagnosticOutcome::Cancelled
                        }
                        renderer::job_manager::JobState::Failed => {
                            renderer::render_diagnostics::DiagnosticOutcome::Failed
                        }
                        _ => anyhow::bail!("job {id} has no completed diagnostic record"),
                    },
                )
            };
            emit_diagnostic_report(diagnostic, output, json)?;
        }
        DiagnosticsAction::Performance { id, output, json } => {
            let performance = renderer::performance::PerformanceStore::open_default()?.get(&id)?;
            let diagnostic = renderer::render_diagnostics::RenderDiagnosticStore::open_default()
                .and_then(|store| store.get_for_performance(&id))
                .unwrap_or_else(|_| RenderDiagnosticRecord::from_performance(&performance));
            emit_diagnostic_report(diagnostic, output, json)?;
        }
    }
    Ok(())
}

fn emit_diagnostic_report(
    report: renderer::render_diagnostics::RenderDiagnosticRecord,
    output: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    report.validate_schema_version()?;
    if let Some(path) = output {
        renderer::render_diagnostics::save_diagnostic_atomic(&path, &report)?;
        println!("diagnostic report exported: {}", path.display());
    } else if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Diagnostic {}: {:?}", report.id, report.outcome);
        if let Some(workload) = &report.workload {
            println!(
                "Workload: {}x{} at {} FPS; requested {} / {}; effective backend {}",
                workload.resolution[0],
                workload.resolution[1],
                workload.fps,
                workload.requested_backend,
                workload.requested_execution_mode,
                report.effective_backend
            );
        }
        if let Some(phase) = &report.critical_path.largest_exclusive_phase {
            println!(
                "Largest disjoint phase: {phase} ({:.2} ms)",
                report.critical_path.phase_wall_ms.unwrap_or(0.0)
            );
        }
        for finding in &report.findings {
            println!("{:?}: {}", finding.kind, finding.summary);
            for evidence in &finding.evidence {
                println!("  - {evidence}");
            }
        }
        if let Some(error) = &report.error {
            println!(
                "Failure subsystem: {}; phase: {}",
                error.subsystem,
                error.phase.as_deref().unwrap_or("unknown")
            );
            for cause in &error.chain {
                println!("  cause: {cause}");
            }
        }
    }
    Ok(())
}

fn run_performance_command(action: PerformanceAction) -> Result<()> {
    let store = renderer::performance::PerformanceStore::open_default()?;
    match action {
        PerformanceAction::History { limit, json } => {
            let records = store.list(limit.clamp(1, renderer::performance::DEFAULT_RETENTION))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&records)?);
            } else {
                println!("ID\tCREATED (UTC ms)\tMODE\tCACHE\tEXECUTION\tFRAMES\tSIZE");
                for record in records {
                    println!(
                        "{}\t{}\t{}\t{}\t{:.3}s\t{}\t{}x{}",
                        record.id,
                        record.created_unix_time_ms,
                        record.workload.execution_mode,
                        record.gcc_cache_state.as_deref().unwrap_or("n/a"),
                        record.execution_time_ms / 1000.0,
                        record.frames,
                        record.workload.resolution[0],
                        record.workload.resolution[1]
                    );
                }
                eprintln!("performance history: {}", store.root().display());
            }
        }
        PerformanceAction::Compare {
            left_id,
            right_id,
            json,
        } => {
            let left = store.get(&left_id)?;
            let right = store.get(&right_id)?;
            let comparison = renderer::performance::compare_records(&left, &right);
            if json {
                println!("{}", serde_json::to_string_pretty(&comparison)?);
            } else {
                println!(
                    "{} → {}: {} ({:+.3}ms, {})",
                    comparison.left_id,
                    comparison.right_id,
                    comparison.classification,
                    comparison.absolute_difference_ms,
                    comparison.percentage_difference.map_or_else(
                        || "percentage unavailable".into(),
                        |value| format!("{value:+.2}%")
                    )
                );
                if !comparison.comparable {
                    println!("not comparable: {}", comparison.differences.join(", "));
                }
            }
        }
        PerformanceAction::Benchmark {
            config,
            warmup,
            iterations,
            compare_vm_gcc,
            measure_telemetry_overhead,
            output,
            json,
        } => {
            let config = renderer::render::RenderConfig::load_config(config)?;
            let report = renderer::performance::run_benchmark(
                config,
                warmup,
                iterations,
                compare_vm_gcc,
                measure_telemetry_overhead,
            )?;
            let serialized = serde_json::to_string_pretty(&report)?;
            if let Some(path) = output {
                renderer::performance::save_benchmark_report_atomic(&path, &report)?;
                println!("benchmark report: {}", path.display());
            } else if json {
                println!("{serialized}");
                return Ok(());
            } else {
                println!(
                    "Benchmark {} — {} warm-up, {} measured run(s) per mode",
                    report.benchmark_id, report.warmup_runs, report.measured_runs_per_backend
                );
                for backend in &report.backends {
                    println!(
                        "{}: median {:.2}ms; min {:.2}ms; mean {:.2}ms; max {:.2}ms; n={}{}",
                        backend.execution_mode,
                        backend.distribution.median_ms,
                        backend.distribution.min_ms,
                        backend.distribution.mean_ms,
                        backend.distribution.max_ms,
                        backend.distribution.samples,
                        backend
                            .distribution
                            .p95_ms
                            .map_or_else(String::new, |p95| format!("; p95 {:.2}ms", p95))
                    );
                    if let Some(observation) = &backend.initial_cache_observation {
                        println!(
                            "  initial Sono-GCC cache observation: {} ({})",
                            observation.gcc_cache_state.as_deref().unwrap_or("unknown"),
                            observation.id
                        );
                    }
                    for sample in &backend.samples {
                        println!(
                            "  sample {}: {:.2}ms; cache={}; effective={}",
                            sample.id,
                            sample.execution_time_ms,
                            sample.gcc_cache_state.as_deref().unwrap_or("n/a"),
                            sample.effective_backend_status
                        );
                    }
                }
                if let Some(comparison) = &report.vm_vs_sono_gcc {
                    println!(
                        "VM vs Sono-GCC median: {:+.2}ms ({}, comparable={})",
                        comparison.absolute_difference_ms,
                        comparison.classification,
                        comparison.comparable
                    );
                    if let Some(reason) = &comparison.reason {
                        println!("  comparison withheld: {reason}");
                    }
                }
                if let Some(estimate) = &report.telemetry_overhead {
                    println!(
                        "Detailed telemetry overhead estimate: {:+.2}ms / {:+.2}% ({})",
                        estimate.overhead_ms,
                        estimate.overhead_percent.unwrap_or(0.0),
                        estimate.classification
                    );
                }
            }
            eprintln!("small metric records: {}", store.root().display());
        }
    }
    Ok(())
}

fn run_regression_command(action: RegressionAction) -> Result<()> {
    match action {
        RegressionAction::Update {
            config,
            ui_profile,
            watermark_profile,
            reference,
            fixture,
            checkpoints,
            resolutions,
            compare_vm_gcc,
            timing_samples,
            include_media,
            replace,
            json,
        } => {
            let mut config = renderer::render::RenderConfig::load_config(config)?;
            apply_regression_profiles(&mut config, ui_profile, watermark_profile)?;
            let resolutions = if resolutions.is_empty() {
                vec![[config.width, config.height]]
            } else {
                resolutions
            };
            let options = renderer::regression::RegressionOptions {
                fixture_id: fixture,
                checkpoint_times: checkpoints,
                resolutions,
                compare_vm_gcc,
                timing_samples,
                include_media,
            };
            let report =
                renderer::regression::update_reference(&config, &options, &reference, replace)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "{} reference {} at {} ({} mode/resolution captures, {} frame checkpoints)",
                    if report.replaced_existing {
                        "Updated"
                    } else {
                        "Created"
                    },
                    report.fixture_id,
                    report.reference_directory.display(),
                    report.captured_modes,
                    report.captured_frames
                );
                if let Some(error) = &report.previous_reference_error {
                    println!("Previous marked reference could not be read: {error}");
                }
                println!("Observed changes before blessing the new reference:");
                for change in &report.changed_layers {
                    println!("  {change}");
                }
                if report.changed_layers.is_empty() {
                    println!("  first reference capture; inspect its PNGs and manifest before relying on it");
                }
            }
        }
        RegressionAction::Run {
            config,
            ui_profile,
            watermark_profile,
            reference,
            output,
            pixel_tolerance,
            json,
        } => {
            let mut config = renderer::render::RenderConfig::load_config(config)?;
            apply_regression_profiles(&mut config, ui_profile, watermark_profile)?;
            let report = renderer::regression::run_regression(
                &config,
                &reference,
                output.as_deref(),
                pixel_tolerance,
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Regression {}: {} (comparable={})",
                    report.fixture_id,
                    if !report.comparable || !report.coverage_differences.is_empty() {
                        "NOT COMPARABLE"
                    } else if report.passed
                        && report
                            .vm_vs_gcc
                            .iter()
                            .any(|comparison| !comparison.comparable)
                    {
                        "REFERENCE PASS; PARITY INCOMPLETE"
                    } else if report.passed {
                        "PASS"
                    } else {
                        "FAIL"
                    },
                    report.comparable
                );
                for difference in &report.workload_differences {
                    println!("Not comparable: {difference}");
                }
                for difference in &report.coverage_differences {
                    println!("Coverage mismatch: {difference}");
                }
                for difference in &report.environment_differences {
                    println!("Environment: {difference}");
                }
                for frame in &report.frames {
                    let mut changed = vec![
                        &frame.runtime_semantics,
                        &frame.watch_operation_counts,
                        &frame.display_list,
                        &frame.audio_events,
                        &frame.particle_events,
                        &frame.gameplay_ui_state,
                        &frame.watermark_pose,
                        &frame.pixels,
                    ]
                    .into_iter()
                    .filter(|layer| layer.changed)
                    .map(|layer| layer.name.as_str())
                    .collect::<Vec<_>>();
                    if frame.timing.classification == "changed" {
                        changed.push("frame_timing");
                    }
                    println!(
                        "{} {}x{} {}: {}",
                        frame.execution_mode,
                        frame.resolution[0],
                        frame.resolution[1],
                        frame.checkpoint_id,
                        if changed.is_empty() {
                            "all captured layers match".into()
                        } else {
                            format!("changed: {}", changed.join(", "))
                        }
                    );
                    if let Some(stats) = &frame.pixel_stats {
                        if frame.pixels.changed {
                            println!(
                                "  pixels: {}/{} ({:.3}%), max delta {}, mean delta {:.4}, bounds {:?}",
                                stats.changed_pixels,
                                stats.compared_pixels,
                                stats.changed_ratio * 100.0,
                                stats.max_channel_delta,
                                stats.mean_absolute_channel_delta,
                                stats.changed_bounds
                            );
                        }
                    }
                    if frame.timing.classification != "unavailable" {
                        println!(
                            "  checkpoint step: {:?} ms -> {:?} ms ({}, delta {:?} ms)",
                            frame.timing.expected_median_ms,
                            frame.timing.current_median_ms,
                            frame.timing.classification,
                            frame.timing.absolute_difference_ms
                        );
                    }
                    for artifact in &frame.artifacts {
                        println!("  artifact: {artifact}");
                    }
                }
                for comparison in &report.vm_vs_gcc {
                    println!(
                        "VM/GCC {}x{} {}: {}{}",
                        comparison.resolution[0],
                        comparison.resolution[1],
                        comparison.checkpoint_id,
                        if !comparison.comparable {
                            "unavailable"
                        } else if comparison.changed_layers.is_empty() {
                            "matching"
                        } else {
                            "different"
                        },
                        comparison
                            .reason
                            .as_ref()
                            .map_or_else(String::new, |reason| format!(" ({reason})"))
                    );
                    if !comparison.changed_layers.is_empty() {
                        println!("  layers: {}", comparison.changed_layers.join(", "));
                    }
                }
                for diff in &report.media {
                    println!(
                        "media/audio: {}",
                        if diff.changed { "changed" } else { "matching" }
                    );
                    if let Some(detail) = &diff.detail {
                        println!("  {detail}");
                    }
                }
                println!(
                    "Current PNGs and structured report: {}",
                    report.current_capture_directory.display()
                );
            }
            match report.exit_kind() {
                renderer::regression::RegressionExit::Passed => {}
                renderer::regression::RegressionExit::Failed => {
                    return Err(CliExitError {
                        code: 1,
                        message: "regression layers differ; inspect comparison.json and retained diff artifacts".into(),
                    }.into());
                }
                renderer::regression::RegressionExit::NotComparable => {
                    return Err(CliExitError {
                        code: 2,
                        message: if !report.coverage_differences.is_empty() {
                            format!(
                                "regression capture coverage differs: {}",
                                report.coverage_differences.join(", ")
                            )
                        } else if report.workload_differences.is_empty() {
                            "regression reference layers match, but one or more requested VM/GCC parity comparisons were unavailable".into()
                        } else {
                            format!(
                                "regression workload is not comparable: {}",
                                report.workload_differences.join(", ")
                            )
                        },
                    }
                    .into());
                }
            }
        }
        RegressionAction::Inspect { reference, json } => {
            let report = renderer::regression::load_reference(&reference)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Reference {} schema {} created {} at {}",
                    report.fixture_id,
                    report.schema_version,
                    report.created_unix_time_ms,
                    reference.display()
                );
                println!(
                    "{} fps, backend {}, UI {}, resolutions {:?}, checkpoints {:?}",
                    report.workload.fps,
                    report.workload.pixel_backend,
                    report.workload.gameplay_ui_mode,
                    report.workload.resolutions,
                    report.checkpoint_times
                );
                for mode in &report.modes {
                    println!(
                        "{} {}x{}: {} checkpoint(s), {} native region(s), media {}",
                        mode.execution_mode,
                        mode.resolution[0],
                        mode.resolution[1],
                        mode.frames.len(),
                        mode.native_regions_executed,
                        if mode.media.is_some() {
                            "captured"
                        } else {
                            "not captured"
                        }
                    );
                    for frame in &mode.frames {
                        println!(
                            "  {} at {:.6}s -> {}",
                            frame.checkpoint_id, frame.requested_time, frame.png_file
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn apply_regression_profiles(
    config: &mut renderer::render::RenderConfig,
    ui_profile: Option<PathBuf>,
    watermark_profile: Option<PathBuf>,
) -> Result<()> {
    if let Some(path) = ui_profile {
        config.set_ui_profile(path)?;
    }
    if let Some(path) = watermark_profile {
        config.set_watermark_profile(path)?;
    }
    Ok(())
}

fn parse_regression_resolution(value: &str) -> std::result::Result<[u32; 2], String> {
    let (width, height) = value
        .split_once('x')
        .or_else(|| value.split_once('X'))
        .ok_or_else(|| "resolution must be WIDTHxHEIGHT".to_owned())?;
    let width = width
        .parse::<u32>()
        .map_err(|_| "invalid resolution width".to_owned())?;
    let height = height
        .parse::<u32>()
        .map_err(|_| "invalid resolution height".to_owned())?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("resolution must be within 1..=8192 on each axis".into());
    }
    Ok([width, height])
}

fn run_presentation_command(path: PathBuf, json: bool) -> Result<()> {
    let pack = renderer::presentation_assets::PresentationAssetPack::load(&path)?;
    let ffmpeg = renderer::ffmpeg::resolve()?;
    let assets = pack.inspect_with_ffprobe(&path, &ffmpeg.ffprobe)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pack": pack,
                "source_file_count": assets.len(),
                "assets": assets,
            }))?
        );
    } else {
        println!("Asset pack: {} ({})", pack.name, pack.id);
        println!("Schema: {} / version {}", pack.format, pack.schema_version);
        println!("Sources: {}", assets.len());
        for asset in assets {
            let dimensions = asset
                .dimensions
                .map(|[width, height]| format!(" {width}x{height}"))
                .or_else(|| {
                    asset.video.as_ref().map(|video| {
                        format!(
                            " {}x{} {}fps {:.3}s {}{}",
                            video.width,
                            video.height,
                            video.frame_rate,
                            video.duration_seconds,
                            video.video_codec,
                            video
                                .audio_codec
                                .as_ref()
                                .map(|codec| format!(" + {codec}"))
                                .unwrap_or_default()
                        )
                    })
                })
                .unwrap_or_default();
            println!(
                "  {:<8} {:<36} {:>10} bytes{}  sha256 {}",
                format!("{:?}", asset.kind).to_lowercase(),
                asset.id,
                asset.size_bytes,
                dimensions,
                asset.sha256
            );
        }
    }

    Ok(())
}

fn run_job_command(
    store: Option<PathBuf>,
    profile_store: Option<PathBuf>,
    action: JobAction,
) -> Result<()> {
    use renderer::job_manager::{JobState, RenderJobManager};
    match action {
        JobAction::Submit {
            config,
            profile,
            priority,
            enqueue_only,
        } => {
            let (config, profile_source) = if let Some(selector) = profile {
                let source = open_profile_manager(profile_store)?.get(&selector)?;
                (source.config.clone(), Some(source))
            } else {
                let path = config.context("supply a config path or --profile")?;
                (renderer::render::RenderConfig::load_config(path)?, None)
            };
            let manager = open_job_manager(store, config.job_manager.clone())?;
            let id = if let Some(profile) = profile_source {
                manager.enqueue_from_profile(
                    config,
                    priority.map(Into::into),
                    profile.id,
                    profile.name,
                )?
            } else {
                manager.enqueue(config, priority.map(Into::into))?
            };
            println!("queued render job {id}");
            if !enqueue_only {
                manager.start()?;
                manager.wait_until_idle()?;
                let job = manager.inspect(&id)?;
                println!("{:?}: {}", job.state, job.output_path().unwrap().display());
                if job.state != JobState::Completed {
                    anyhow::bail!(
                        "render job {id} ended as {:?}: {}",
                        job.state,
                        job.diagnostic.as_deref().unwrap_or("no diagnostic")
                    );
                }
                renderer::notifications::flush(std::time::Duration::from_secs(3));
            } else {
                renderer::notifications::flush(std::time::Duration::from_millis(100));
            }
        }
        JobAction::Run { config } => {
            let loaded = config
                .as_ref()
                .map(renderer::render::RenderConfig::load_config)
                .transpose()?;
            let scheduler = loaded
                .as_ref()
                .map(|config| config.job_manager.clone())
                .unwrap_or_default();
            let manager = open_job_manager(store, scheduler)?;
            let pending = manager.jobs()?;
            if let Some(config) = loaded.as_ref() {
                manager.configure(config.job_manager.clone())?;
            } else {
                if let Some(job) = pending.iter().find(|job| job.state == JobState::Queued) {
                    manager.configure(job.config.job_manager.clone())?;
                }
            }
            if !pending
                .iter()
                .any(|job| job.state == JobState::Queued || job.state.is_active())
            {
                println!("no pending render jobs");
                return Ok(());
            }
            manager.start()?;
            manager.wait_until_idle()?;
            for job in manager.jobs()? {
                if job.state == JobState::Failed {
                    println!(
                        "{} failed: {}",
                        job.id,
                        job.diagnostic.as_deref().unwrap_or("unknown error")
                    );
                }
            }
            renderer::notifications::flush(std::time::Duration::from_secs(3));
        }
        JobAction::List => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            println!("ID\tSTATE\tPRIORITY\tATTEMPT\tPROFILE\tOUTPUT\tPROGRESS");
            for job in manager.jobs()? {
                let output = job
                    .output_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                let progress = job
                    .progress
                    .as_ref()
                    .and_then(|p| p.percentage)
                    .map(|p| format!("{p:.1}%"))
                    .unwrap_or_default();
                println!(
                    "{}\t{:?}\t{:?}\t{}\t{}\t{}\t{}",
                    job.id,
                    job.state,
                    job.priority,
                    job.attempt,
                    job.profile_name.as_deref().unwrap_or(""),
                    output,
                    progress
                );
            }
            eprintln!("queue store: {}", manager.store_path().display());
        }
        JobAction::Inspect { id } => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            println!("{}", serde_json::to_string_pretty(&manager.inspect(&id)?)?);
        }
        JobAction::Cancel { id } => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            manager.cancel(&id)?;
            println!("cancellation requested for {id}");
        }
        JobAction::Retry { id, enqueue_only } => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            let job = manager.inspect(&id)?;
            manager.configure(job.config.job_manager.clone())?;
            manager.retry(&id)?;
            if !enqueue_only {
                manager.start()?;
                manager.wait_until_idle()?;
                let job = manager.inspect(&id)?;
                println!("{:?}: {}", job.state, job.output_path().unwrap().display());
                if job.state != JobState::Completed {
                    anyhow::bail!(
                        "retry of {id} ended as {:?}: {}",
                        job.state,
                        job.diagnostic.as_deref().unwrap_or("no diagnostic")
                    );
                }
                renderer::notifications::flush(std::time::Duration::from_secs(3));
            } else {
                println!("retry queued for {id}");
            }
        }
        JobAction::Remove { id } => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            manager.remove(&id)?;
            println!("removed {id}");
        }
        JobAction::Reorder { id, position } => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            manager.reorder(&id, position)?;
            println!("moved {id} to queued position {position}");
        }
        JobAction::Pause => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            manager.pause()?;
            println!("queue paused; active jobs continue to a safe cancellation point");
        }
        JobAction::Resume => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            manager.resume()?;
            println!("queue resumed");
        }
        JobAction::CancelQueued => {
            let manager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            println!("cancelled {} queued jobs", manager.cancel_queued()?);
        }
        JobAction::ClearTerminal => {
            let manager: RenderJobManager =
                open_job_manager(store, renderer::job_manager::JobManagerConfig::default())?;
            println!("removed {} terminal jobs", manager.clear_terminal()?);
        }
    }
    Ok(())
}

fn run_history_command(
    store_path: Option<PathBuf>,
    queue_store: Option<PathBuf>,
    action: HistoryAction,
) -> Result<()> {
    use renderer::render_history::{
        default_store_path as default_history_store_path, HistoryQuery, HistoryRetentionPolicy,
        RenderHistoryStore,
    };

    let queue_path = queue_store
        .clone()
        .unwrap_or_else(renderer::job_manager::default_store_path);
    let history_path = store_path
        .or_else(|| std::env::var_os("SONO_RENDERER_HISTORY_DIR").map(PathBuf::from))
        .or_else(|| {
            queue_store
                .as_ref()
                .and_then(|path| path.parent().map(|parent| parent.join("history")))
        })
        .unwrap_or_else(default_history_store_path);
    let history = RenderHistoryStore::open_at(&history_path)?;
    // Opening the shared manager performs terminal-state recovery/backfill.
    // History browsing remains available if the independent queue store is
    // damaged or inaccessible.
    let mut recovery_manager = renderer::job_manager::RenderJobManager::open_at_with_history(
        &queue_path,
        history.root(),
        renderer::job_manager::JobManagerConfig::default(),
    )
    .ok();
    match action {
        HistoryAction::List {
            limit,
            search,
            status,
            engine,
            project,
            backend,
            json,
        } => {
            let query = HistoryQuery {
                text: search,
                status: status.map(Into::into),
                engine,
                project,
                backend,
                limit: Some(limit),
            };
            let page = history.list(&query);
            if json {
                println!("{}", serde_json::to_string_pretty(&page.entries)?);
            } else {
                println!("ID\tCOMPLETED_UNIX_MS\tSTATUS\tATTEMPT\tBACKEND\tPROFILE\tOUTPUT");
                for entry in &page.entries {
                    println!(
                        "{}\t{}\t{:?}\t{}\t{}\t{}\t{}",
                        entry.id,
                        entry.completed_unix_time_ms,
                        entry.status,
                        entry.attempt,
                        entry.execution_backend,
                        entry.profile_name.as_deref().unwrap_or(""),
                        entry
                            .output
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default()
                    );
                }
                println!("History store: {}", history.root().display());
            }
            for issue in page.issues {
                eprintln!(
                    "History entry skipped ({}): {}",
                    issue.path.display(),
                    issue.message
                );
            }
        }
        HistoryAction::Show { id, json } => {
            let entry = history.get(&id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("History ID: {}", entry.id);
                println!("Job / attempt: {} / {}", entry.job_id, entry.attempt);
                println!("Status: {:?}", entry.status);
                println!("Completed (Unix ms): {}", entry.completed_unix_time_ms);
                println!(
                    "Profile: {} ({})",
                    entry.profile_name.as_deref().unwrap_or("none"),
                    entry.profile_id.as_deref().unwrap_or("none")
                );
                println!(
                    "Project: {}",
                    entry
                        .project_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "none".into())
                );
                println!(
                    "Renderer: {} / {} / {}",
                    entry.renderer_version, entry.operating_system, entry.architecture
                );
                println!("Backend: {}", entry.execution_backend);
                println!(
                    "Output: {}",
                    entry
                        .output
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "none".into())
                );
                println!(
                    "Output exists: {} ({:?} bytes)",
                    entry.output.exists, entry.output.size_bytes
                );
                println!(
                    "Performance record: {}",
                    entry.performance_record_id.as_deref().unwrap_or("none")
                );
                println!(
                    "Diagnostic record: {}",
                    entry.diagnostic_record_id.as_deref().unwrap_or("none")
                );
                if let Some(performance_id) = entry.performance_record_id.as_deref() {
                    if let Ok(performance) = renderer::performance::PerformanceStore::open_default()
                        .and_then(|store| store.get(performance_id))
                    {
                        println!(
                            "Execution: {:.2} ms; queue wait: {}; total user latency: {}",
                            performance.execution_time_ms,
                            performance.queue_wait_time_ms.map_or_else(
                                || "not queued".into(),
                                |value| format!("{value:.2} ms")
                            ),
                            performance.total_user_latency_ms.map_or_else(
                                || "not recorded".into(),
                                |value| format!("{value:.2} ms")
                            )
                        );
                        for stage in &performance.stage_timings {
                            println!(
                                "  Stage {}: {:.2} ms ({:?})",
                                stage.name, stage.duration_ms, stage.semantics
                            );
                        }
                    } else {
                        println!("Performance details: linked record is unavailable or expired");
                    }
                }
                if let Some(diagnostic_id) = entry.diagnostic_record_id.as_deref() {
                    if let Ok(report) =
                        renderer::render_diagnostics::RenderDiagnosticStore::open_default()
                            .and_then(|store| store.get(diagnostic_id))
                    {
                        println!("Diagnostic findings:");
                        for finding in report.findings {
                            println!("  {:?}: {}", finding.kind, finding.summary);
                        }
                    } else {
                        println!("Diagnostic details: linked report is unavailable or expired");
                    }
                }
                println!(
                    "Cache: {}",
                    entry.cache_state.as_deref().unwrap_or("not reported")
                );
                println!("Fallback: {}", entry.fallback.as_deref().unwrap_or("none"));
                if let Some(error) = &entry.error {
                    println!("Error: {error}");
                }
                println!("Inputs:");
                for input in &entry.input_identities {
                    println!(
                        "  {}: {} [{}]",
                        input.role,
                        input.path.display(),
                        if input.exists { "available" } else { "missing" }
                    );
                }
                println!(
                    "Configuration snapshot:\n{}",
                    serde_json::to_string_pretty(&entry.config_snapshot)?
                );
            }
        }
        HistoryAction::Rerender {
            id,
            output,
            enqueue_only,
        } => {
            let entry = history.get(&id)?;
            let mut config =
                renderer::render_history::sanitize_config_snapshot(&entry.config_snapshot);
            let missing = renderer::render_history::missing_inputs(&config);
            if !missing.is_empty() {
                let details = missing
                    .iter()
                    .map(|(role, path)| format!("{role}: {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                anyhow::bail!(
                    "historical inputs are missing; no replacement was selected:\n{details}"
                );
            }
            if let Some(output) = output {
                config.output = Some(output);
            }
            let target = config
                .output
                .as_deref()
                .context("history entry has no output path; provide --output")?;
            if target.exists() {
                anyhow::bail!(
                    "historical output already exists; choose a new path with --output: {}",
                    target.display()
                );
            }
            if !target
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .is_dir()
            {
                anyhow::bail!("historical output directory is unavailable; choose a new path with --output: {}", target.display());
            }
            config.validate_export_inputs()?;
            let manager = match recovery_manager.take() {
                Some(manager) => manager,
                None => renderer::job_manager::RenderJobManager::open_at_with_history(
                    &queue_path,
                    history.root(),
                    config.job_manager.clone(),
                )?,
            };
            let job_id = manager.enqueue_with_metadata(
                config,
                None,
                entry.profile_id.clone().zip(entry.profile_name.clone()),
                renderer::job_manager::RenderJobMetadata {
                    project_path: entry.project_path.clone(),
                    original_job_id: Some(
                        entry
                            .original_job_id
                            .clone()
                            .unwrap_or_else(|| entry.job_id.clone()),
                    ),
                    rerendered_from_history_id: Some(entry.id.clone()),
                    ..Default::default()
                },
            )?;
            println!(
                "queued new rerender job {job_id} from history entry {}",
                entry.id
            );
            if !enqueue_only {
                manager.start()?;
                manager.wait_until_idle()?;
                let job = manager.inspect(&job_id)?;
                println!("{:?}: {}", job.state, job.output_path().unwrap().display());
                if job.state != renderer::job_manager::JobState::Completed {
                    anyhow::bail!(
                        "rerender job {job_id} ended as {:?}: {}",
                        job.state,
                        job.diagnostic.as_deref().unwrap_or("no diagnostic")
                    );
                }
                renderer::notifications::flush(std::time::Duration::from_secs(3));
            }
        }
        HistoryAction::Delete { id } => {
            history.delete(&id)?;
            println!("deleted history entry {id}");
        }
        HistoryAction::Pin { id } => {
            history.pin(&id, true)?;
            println!("pinned history entry {id}");
        }
        HistoryAction::Unpin { id } => {
            history.pin(&id, false)?;
            println!("unpinned history entry {id}");
        }
        HistoryAction::Clear { include_failures } => {
            let count = history.clear(!include_failures)?;
            println!(
                "removed {count} history entries{}",
                if include_failures {
                    ""
                } else {
                    "; failures and pinned entries were kept"
                }
            );
        }
        HistoryAction::Prune {
            max_entries,
            max_age_days,
            delete_failures,
        } => {
            let mut policy = history
                .retention()
                .unwrap_or_else(|_| HistoryRetentionPolicy::default());
            if let Some(max_entries) = max_entries {
                policy.max_entries = max_entries;
            }
            if max_age_days.is_some() {
                policy.max_age_days = max_age_days;
            }
            policy.keep_failures = !delete_failures;
            let removed = history.set_retention(&policy)?;
            println!("retention saved; pruned {removed} entries (max {}, age {:?} days, keep failures {})", policy.max_entries, policy.max_age_days, policy.keep_failures);
        }
        HistoryAction::Export { id, output } => {
            let entry = history.get(&id)?;
            renderer::render_history::save_history_export_atomic(&output, &entry)?;
            println!(
                "exported history entry {} to {}",
                entry.id,
                output.display()
            );
        }
    }
    Ok(())
}

fn run_manifest_command(action: ManifestAction) -> Result<()> {
    use renderer::render_manifest::{compare, RenderManifest, VerificationStatus};
    match action {
        ManifestAction::Export {
            history_id,
            output,
            history_store,
        } => {
            let history_path = history_store
                .or_else(|| std::env::var_os("SONO_RENDERER_HISTORY_DIR").map(PathBuf::from))
                .unwrap_or_else(renderer::render_history::default_store_path);
            let history = renderer::render_history::RenderHistoryStore::open_at(history_path)?;
            let started = std::time::Instant::now();
            let manifest =
                renderer::render_manifest::export_history_atomic(&history, &history_id, &output)?;
            println!(
                "exported manifest {} ({} active assets; SHA-256 {}) to {} in {:.2} ms",
                history_id,
                manifest.inputs.iter().filter(|input| input.active).count(),
                manifest.manifest_sha256,
                output.display(),
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        ManifestAction::Inspect { path, json } => {
            let manifest = RenderManifest::load(&path)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&manifest)?);
            } else {
                println!("Manifest SHA-256: {}", manifest.manifest_sha256);
                println!(
                    "Schema: {} / version {}",
                    manifest.format, manifest.schema_version
                );
                println!(
                    "Renderer/runtime: {} / {} / commit {}",
                    manifest.renderer.version,
                    manifest.renderer.runtime_semantics,
                    manifest.renderer.commit.as_deref().unwrap_or("unknown")
                );
                println!(
                    "Backend: requested {}, effective {} ({} / {})",
                    manifest.execution.requested_backend,
                    manifest.execution.effective_backend,
                    manifest.execution.target_os,
                    manifest.execution.target_architecture
                );
                println!(
                    "Provenance: history {}, job {}, profile {} ({})",
                    manifest
                        .provenance
                        .history_id
                        .as_deref()
                        .unwrap_or("unknown"),
                    manifest.provenance.job_id.as_deref().unwrap_or("unknown"),
                    manifest
                        .provenance
                        .profile_name
                        .as_deref()
                        .unwrap_or("none"),
                    manifest.provenance.profile_id.as_deref().unwrap_or("none")
                );
                println!("Inputs:");
                for input in &manifest.inputs {
                    println!(
                        "  {} [{}] {} — {}",
                        input.id,
                        if input.active { "active" } else { "inactive" },
                        input.sha256.as_deref().unwrap_or("identity unavailable"),
                        input
                            .path_hint
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "no path hint".into())
                    );
                }
                println!(
                    "Stored references: RGB frame sequence {}, SFX events {}, SFX PCM {}",
                    manifest
                        .references
                        .rgb_sequence_sha1
                        .as_deref()
                        .unwrap_or("unavailable"),
                    manifest
                        .references
                        .sfx_event_sha1
                        .as_deref()
                        .unwrap_or("unavailable"),
                    manifest
                        .references
                        .sfx_pcm_sha1
                        .as_deref()
                        .unwrap_or("unavailable")
                );
                println!("Output size: {:?} bytes", manifest.output.size_bytes);
            }
        }
        ManifestAction::Verify { path, assets, json } => {
            let manifest = RenderManifest::load(&path)?;
            let overrides = parse_manifest_assets(&assets)?;
            let report = manifest.verify(&overrides)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Manifest SHA-256: {}", report.manifest_sha256);
                println!("Schema supported: {}", report.schema_supported);
                println!(
                    "Exact backend available: {}",
                    report.can_reproduce_exact_backend
                );
                println!(
                    "Semantic inputs/config match: {}",
                    report.can_reproduce_semantic_inputs
                );
                println!(
                    "Stored reference coverage: {}",
                    report.strongest_stored_reference
                );
                for item in &report.items {
                    println!(
                        "{:?} | {:?} | {}{}",
                        item.status,
                        item.category,
                        item.component,
                        if item.affects_semantics {
                            " [semantic]"
                        } else {
                            " [environmental]"
                        }
                    );
                    println!("  {}", item.explanation);
                    if let Some(expected) = &item.expected {
                        println!("  expected: {expected}");
                    }
                    if let Some(actual) = &item.actual {
                        println!("  actual:   {actual}");
                    }
                }
            }
            if report
                .items
                .iter()
                .any(|item| item.status == VerificationStatus::Mismatch)
            {
                eprintln!("one or more manifest components differ");
            }
        }
        ManifestAction::Diff { left, right, json } => {
            let left = RenderManifest::load(&left)?;
            let right = RenderManifest::load(&right)?;
            let report = compare(&left, &right)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Left manifest:  {}", report.left_sha256);
                println!("Right manifest: {}", report.right_sha256);
                println!("Input content matches: {}", report.input_content_matches);
                println!(
                    "Effective configuration matches: {}",
                    report.semantic_config_matches
                );
                println!(
                    "Raw frame sequence matches: {:?}",
                    report.raw_frame_reference_matches
                );
                println!(
                    "Audio PCM reference matches: {:?}",
                    report.audio_reference_matches
                );
                if report.differences.is_empty() {
                    println!("No differences found in compared fields.");
                }
                for difference in &report.differences {
                    println!("{:?} | {}", difference.category, difference.component);
                    println!(
                        "  left:  {}",
                        difference.left.as_deref().unwrap_or("(not recorded)")
                    );
                    println!(
                        "  right: {}",
                        difference.right.as_deref().unwrap_or("(not recorded)")
                    );
                }
            }
        }
        ManifestAction::Reproduce {
            path,
            assets,
            output,
            enqueue_only,
            queue_store,
        } => {
            let manifest = RenderManifest::load(&path)?;
            let overrides = parse_manifest_assets(&assets)?;
            let config = manifest.build_reproduction_config(&overrides, &output)?;
            let queue_path = queue_store.unwrap_or_else(renderer::job_manager::default_store_path);
            let history_path = std::env::var_os("SONO_RENDERER_HISTORY_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(renderer::render_history::default_store_path);
            let manager = renderer::job_manager::RenderJobManager::open_at_with_history(
                queue_path,
                history_path,
                config.job_manager.clone(),
            )?;
            let profile = manifest
                .provenance
                .profile_id
                .clone()
                .zip(manifest.provenance.profile_name.clone());
            let job_id = manager.enqueue_with_metadata(
                config,
                None,
                profile,
                renderer::job_manager::RenderJobMetadata {
                    original_job_id: manifest.provenance.job_id.clone(),
                    rerendered_from_history_id: manifest.provenance.history_id.clone(),
                    reproduced_from_manifest_sha256: Some(manifest.manifest_sha256.clone()),
                    ..Default::default()
                },
            )?;
            println!(
                "queued new reproduction job {job_id} from manifest {}",
                manifest.manifest_sha256
            );
            if !enqueue_only {
                manager.start()?;
                manager.wait_until_idle()?;
                let job = manager.inspect(&job_id)?;
                println!("{:?}: {}", job.state, job.output_path().unwrap().display());
                if job.state != renderer::job_manager::JobState::Completed {
                    anyhow::bail!(
                        "manifest reproduction job {job_id} ended as {:?}: {}",
                        job.state,
                        job.diagnostic.as_deref().unwrap_or("no diagnostic")
                    );
                }
            }
        }
    }
    Ok(())
}

fn parse_manifest_assets(values: &[String]) -> Result<std::collections::BTreeMap<String, PathBuf>> {
    let mut assets = std::collections::BTreeMap::new();
    for value in values {
        let (id, path) = value
            .split_once('=')
            .context("asset repair must use ASSET_ID=FILE")?;
        if id.is_empty() || path.is_empty() {
            anyhow::bail!("asset repair must use a nonempty ASSET_ID=FILE");
        }
        if assets.insert(id.to_owned(), PathBuf::from(path)).is_some() {
            anyhow::bail!("asset ID {id:?} was specified more than once");
        }
    }
    Ok(assets)
}

fn skin_bindings(
    watch: &renderer::watch::WatchData,
) -> Result<std::collections::BTreeMap<u32, String>> {
    let bindings = watch
        .skin
        .get("sprites")
        .and_then(serde_json::Value::as_array)
        .context("EngineWatchData skin binding has no sprites array")?;
    bindings
        .iter()
        .map(|binding| {
            let name = binding
                .get("name")
                .and_then(serde_json::Value::as_str)
                .context("EngineWatchData sprite binding has no name")?;
            let id = binding
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| u32::try_from(id).ok())
                .context("EngineWatchData sprite binding ID is outside u32 range")?;
            Ok((id, name.to_owned()))
        })
        .collect()
}

fn parse_lifecycle_stage(value: &str) -> Result<renderer::watch_runtime::LifecycleStage> {
    use renderer::watch_runtime::LifecycleStage;
    match value.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "preprocess" => Ok(LifecycleStage::Preprocess),
        "spawntime" => Ok(LifecycleStage::SpawnTime),
        "despawntime" => Ok(LifecycleStage::DespawnTime),
        "initialize" => Ok(LifecycleStage::Initialize),
        "updatesequential" => Ok(LifecycleStage::UpdateSequential),
        "updateparallel" => Ok(LifecycleStage::UpdateParallel),
        "terminate" => Ok(LifecycleStage::Terminate),
        _ => anyhow::bail!("unknown diagnostic lifecycle stage {value:?}"),
    }
}

fn parse_level_option_override(value: &str) -> Result<(usize, f64)> {
    let (index, value) = value
        .split_once('=')
        .with_context(|| format!("invalid --level-option {value:?}; expected INDEX=VALUE"))?;
    let index = index
        .parse::<usize>()
        .with_context(|| format!("invalid Level Option index {index:?}"))?;
    let value = value
        .parse::<f64>()
        .with_context(|| format!("invalid Level Option value {value:?}"))?;
    if !value.is_finite() {
        anyhow::bail!("Level Option override must be finite");
    }
    Ok((index, value))
}

#[cfg(test)]
mod cli_tests {
    use super::{assign_acquired_media, parse_level_option_override};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn chart_studio_identity_validation_and_export_commands_parse() {
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "engine-identity", "engine.zip"])
                .unwrap()
                .command,
            super::Command::EngineIdentity { engine } if engine == PathBuf::from("engine.zip")
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer", "validate-level-data", "level.json.gz", "--engine", "engine.zip", "--json"
            ])
            .unwrap()
            .command,
            super::Command::ValidateLevelData { json: true, engine: Some(engine), .. }
                if engine == PathBuf::from("engine.zip")
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer", "export-level-data", "level.json.gz", "out.json.gz", "--overwrite"
            ])
            .unwrap()
            .command,
            super::Command::ExportLevelData { overwrite: true, output, .. }
                if output == PathBuf::from("out.json.gz")
        ));
    }

    #[test]
    fn media_acquisition_cli_parses_safe_modes_and_project_assignment() {
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "media",
                "inspect-url",
                "https://example.test/watch?v=1",
                "--json"
            ])
            .unwrap()
            .command,
            super::Command::Media {
                action: super::MediaAction::InspectUrl { json: true, .. }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "media",
                "download",
                "https://example.test/watch?v=1",
                "--mode",
                "video-audio",
                "--project",
                "song.sonoproject",
                "--json"
            ])
            .unwrap()
            .command,
            super::Command::Media {
                action: super::MediaAction::Download {
                    mode: super::MediaModeArg::VideoAudio,
                    project: Some(project),
                    json: true,
                    ..
                }
            } if project == std::path::Path::new("song.sonoproject")
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "media", "tool-info"])
                .unwrap()
                .command,
            super::Command::Media {
                action: super::MediaAction::ToolInfo { json: false }
            }
        ));
    }

    #[test]
    fn acquired_files_use_existing_project_music_and_mv_references() {
        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("song.sonoproject");
        let media_path = directory.path().join("media/acquired.mp4");
        std::fs::create_dir_all(media_path.parent().unwrap()).unwrap();
        std::fs::write(&media_path, b"fixture").unwrap();
        let media = renderer::media_acquisition::AcquiredMedia {
            path: media_path.clone(),
            sha256: "fixture-hash".into(),
            duration_seconds: 1.0,
            has_audio: true,
            has_video: true,
            format_name: "mov,mp4".into(),
        };
        let mut project = renderer::project::RenderProject::default();
        assign_acquired_media(
            &mut project,
            renderer::media_acquisition::AcquisitionMode::Audio,
            &media,
        );
        assert_eq!(project.inputs.music.as_deref(), Some(media_path.as_path()));
        assert_eq!(project.inputs.mv, None);
        assign_acquired_media(
            &mut project,
            renderer::media_acquisition::AcquisitionMode::VideoAudio,
            &media,
        );
        project.save(&project_path).unwrap();
        let reopened = renderer::project::RenderProject::load(&project_path).unwrap();
        assert_eq!(
            reopened.inputs.music,
            Some(PathBuf::from("media/acquired.mp4"))
        );
        assert_eq!(
            reopened.inputs.mv,
            Some(PathBuf::from("media/acquired.mp4"))
        );
    }

    #[test]
    fn shared_config_commands_and_additive_profile_flags_parse() {
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "render-config", "settings.json"])
                .unwrap()
                .command,
            super::Command::RenderConfig { config, .. }
                if config == Some(PathBuf::from("settings.json"))
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "validate-config", "settings.json"])
                .unwrap()
                .command,
            super::Command::ValidateConfig { .. }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "render-config", "--profile", "YouTube"])
                .unwrap()
                .command,
            super::Command::RenderConfig {
                config: None,
                profile: Some(profile)
            } if profile == "YouTube"
        ));
        assert!(super::Cli::try_parse_from([
            "renderer",
            "render-config",
            "settings.json",
            "--profile",
            "YouTube"
        ])
        .is_err());
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "default-config", "settings.json"])
                .unwrap()
                .command,
            super::Command::DefaultConfig { .. }
        ));
        let parsed = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--ui-profile",
            "hud.json",
            "--watermark-profile",
            "watermark.json",
            "--moving-watermark",
            "--video-crf",
            "23",
            "--video-preset",
            "slow",
            "--notifications-config",
            "notify.json",
            "--no-notifications",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            super::Command::RenderVideo {
                ui_profile: Some(_),
                watermark_profile: Some(_),
                moving_watermark: true,
                video_crf: Some(23),
                video_preset: Some(super::VideoPresetArg::Slow),
                notifications_config: Some(path),
                no_notifications: true,
                ..
            } if path == PathBuf::from("notify.json")
        ));
        let mut invalid_quality = vec![
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--video-crf",
            "52",
        ];
        assert!(super::Cli::try_parse_from(invalid_quality.drain(..)).is_err());
    }

    #[test]
    fn performance_history_compare_and_benchmark_commands_parse() {
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "compatibility",
                "engine.zip",
                "resources.scp",
                "level.json",
                "--dynamic",
                "--human-readable"
            ])
            .unwrap()
            .command,
            super::Command::Compatibility {
                dynamic: true,
                human_readable: true,
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "performance", "history", "--json"])
                .unwrap()
                .command,
            super::Command::Performance {
                action: super::PerformanceAction::History { json: true, .. }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "performance",
                "compare",
                "old-id",
                "new-id",
                "--json"
            ])
            .unwrap()
            .command,
            super::Command::Performance {
                action: super::PerformanceAction::Compare { json: true, .. }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "performance",
                "benchmark",
                "config.json",
                "--compare-vm-gcc",
                "--measure-telemetry-overhead",
                "--iterations",
                "2"
            ])
            .unwrap()
            .command,
            super::Command::Performance {
                action: super::PerformanceAction::Benchmark {
                    compare_vm_gcc: true,
                    measure_telemetry_overhead: true,
                    iterations: 2,
                    ..
                }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "diagnostics",
                "job",
                "job-123",
                "--output",
                "diagnostic.json"
            ])
            .unwrap()
            .command,
            super::Command::Diagnostics {
                action: super::DiagnosticsAction::Job {
                    output: Some(_),
                    ..
                }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "diagnostics",
                "performance",
                "record-456",
                "--json"
            ])
            .unwrap()
            .command,
            super::Command::Diagnostics {
                action: super::DiagnosticsAction::Performance { json: true, .. }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "diagnostics", "record", "diagnostic-456"])
                .unwrap()
                .command,
            super::Command::Diagnostics {
                action: super::DiagnosticsAction::Record { .. }
            }
        ));
    }

    #[test]
    fn whole_chart_accepts_optional_start_and_conflicts_with_duration() {
        let base = [
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
        ];
        let mut args = base.to_vec();
        args.extend(["--whole-chart"]);
        match super::Cli::try_parse_from(&args).unwrap().command {
            super::Command::RenderVideo {
                whole_chart,
                start_time,
                duration,
                ..
            } => {
                assert!(whole_chart);
                assert_eq!(start_time, None);
                assert_eq!(duration, None);
            }
            _ => panic!("expected render-video"),
        }
        args.extend(["--start-time", "5"]);
        assert!(super::Cli::try_parse_from(&args).is_ok());
        args.extend(["--duration", "1"]);
        assert!(super::Cli::try_parse_from(&args).is_err());
    }

    #[test]
    fn parses_level_option_override() {
        assert_eq!(parse_level_option_override("22=0").unwrap(), (22, 0.0));
        assert_eq!(parse_level_option_override("1=10.8").unwrap(), (1, 10.8));
    }

    #[test]
    fn rejects_malformed_level_option_override() {
        assert!(parse_level_option_override("22").is_err());
        assert!(parse_level_option_override("x=1").is_err());
        assert!(parse_level_option_override("1=abc").is_err());
        assert!(parse_level_option_override("1=NaN").is_err());
    }

    #[test]
    fn regression_commands_parse_profiles_checkpoints_and_resolutions() {
        let update = super::Cli::try_parse_from([
            "renderer",
            "regression",
            "update",
            "render.json",
            "--reference",
            "refs/larp",
            "--fixture",
            "larp",
            "--checkpoint",
            "24.833",
            "--checkpoint",
            "25.417",
            "--resolution",
            "1280x720",
            "--resolution",
            "1024X768",
            "--compare-vm-gcc",
            "--ui-profile",
            "ui-profiles/default.json",
        ])
        .unwrap();
        assert!(matches!(
            update.command,
            super::Command::Regression {
                action: super::RegressionAction::Update {
                    fixture,
                    checkpoints,
                    resolutions,
                    compare_vm_gcc: true,
                    ui_profile: Some(_),
                    ..
                }
            } if fixture == "larp"
                && checkpoints == vec![24.833, 25.417]
                && resolutions == vec![[1280, 720], [1024, 768]]
        ));
        assert_eq!(
            super::parse_regression_resolution("640x360").unwrap(),
            [640, 360]
        );
        assert!(super::parse_regression_resolution("8192x8192").is_ok());
        assert!(super::parse_regression_resolution("0x720").is_err());
        assert!(super::parse_regression_resolution("wide").is_err());

        let run = super::Cli::try_parse_from([
            "renderer",
            "regression",
            "run",
            "render.json",
            "--reference",
            "refs/larp",
            "--pixel-tolerance",
            "1",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            run.command,
            super::Command::Regression {
                action: super::RegressionAction::Run {
                    pixel_tolerance: 1,
                    json: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn cache_manager_cli_commands_parse_and_keep_existing_clear_alias() {
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "cache", "list", "--json"])
                .unwrap()
                .command,
            super::Command::Cache {
                action: super::CacheAction::List { json: true }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([
                "renderer",
                "cache",
                "validate",
                "appdata:0123456789abcdef0123456789abcdef01234567",
                "--json"
            ])
            .unwrap()
            .command,
            super::Command::Cache {
                action: super::CacheAction::Validate {
                    id: Some(_),
                    json: true
                }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "cache", "clear", "--disk-only"])
                .unwrap()
                .command,
            super::Command::Cache {
                action: super::CacheAction::Clear { disk_only: true }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "cache", "prune", "--max-bytes", "1234"])
                .unwrap()
                .command,
            super::Command::Cache {
                action: super::CacheAction::Prune { max_bytes: 1234 }
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "clear-gcc-cache"])
                .unwrap()
                .command,
            super::Command::ClearGccCache
        ));
        assert_eq!(super::format_cache_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            super::format_cache_time(951_782_400_000),
            "2000-02-29 00:00:00 UTC"
        );
    }

    #[test]
    fn run_watch_accepts_repeated_level_option_flags() {
        let cli = super::Cli::try_parse_from([
            "renderer",
            "run-watch",
            "engine.zip",
            "level.json.gz",
            "--level-option",
            "1=10.8",
            "--level-option",
            "22=0",
        ])
        .unwrap();
        match cli.command {
            super::Command::RunWatch { level_options, .. } => {
                assert_eq!(level_options, ["1=10.8", "22=0"]);
            }
            _ => panic!("expected run-watch command"),
        }
    }

    #[test]
    fn render_video_accepts_repeated_level_option_flags() {
        let cli = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json.gz",
            "music.mp3",
            "output.mp4",
            "--level-option",
            "1=10.8",
            "--level-option",
            "22=0",
        ])
        .unwrap();
        match cli.command {
            super::Command::RenderVideo { level_options, .. } => {
                let parsed = level_options
                    .iter()
                    .map(|value| parse_level_option_override(value).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(parsed, [(1, 10.8), (22, 0.0)]);
            }
            _ => panic!("expected render-video command"),
        }
    }

    #[test]
    fn execution_mode_flag_is_explicit_and_defaults_to_interpreter() {
        let video = [
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
        ];
        assert!(matches!(
            super::Cli::try_parse_from(video).unwrap().command,
            super::Command::RenderVideo {
                execution_mode: super::ExecutionModeArg::Interpreter,
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([video.as_slice(), &["--vm", "sono-gcc"]].concat())
                .unwrap()
                .command,
            super::Command::RenderVideo {
                execution_mode: super::ExecutionModeArg::SonoGcc,
                ..
            }
        ));

        let watch = ["renderer", "run-watch", "engine.zip", "level.json"];
        assert!(matches!(
            super::Cli::try_parse_from(watch).unwrap().command,
            super::Command::RunWatch {
                execution_mode: super::ExecutionModeArg::Interpreter,
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([watch.as_slice(), &["--vm", "sono-gcc"]].concat())
                .unwrap()
                .command,
            super::Command::RunWatch {
                execution_mode: super::ExecutionModeArg::SonoGcc,
                ..
            }
        ));
    }

    #[test]
    fn watch_compatibility_policy_and_random_seed_flags_parse_for_watch_and_render() {
        let watch = super::Cli::try_parse_from([
            "renderer",
            "run-watch",
            "engine.zip",
            "level.json",
            "--watch-compatibility-policy",
            "strict",
            "--watch-random-seed",
            "17",
        ])
        .unwrap();
        assert!(matches!(
            watch.command,
            super::Command::RunWatch {
                watch_compatibility_policy: super::WatchCompatibilityPolicyArg::Strict,
                watch_random_seed: 17,
                ..
            }
        ));

        let render = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--watch-compatibility-policy",
            "permissive",
            "--watch-random-seed",
            "23",
        ])
        .unwrap();
        assert!(matches!(
            render.command,
            super::Command::RenderVideo {
                watch_compatibility_policy: super::WatchCompatibilityPolicyArg::Permissive,
                watch_random_seed: 23,
                ..
            }
        ));
    }

    #[test]
    fn watch_scheduler_defaults_parallel_and_has_explicit_legacy_opt_out() {
        let watch = ["renderer", "run-watch", "engine.zip", "level.json"];
        assert!(matches!(
            super::Cli::try_parse_from(watch).unwrap().command,
            super::Command::RunWatch {
                single_threaded: false,
                watch_workers: None,
                profile: false,
                no_vm_accounting: false,
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from(
                [watch.as_slice(), &["--single-threaded", "--profile"]].concat()
            )
            .unwrap()
            .command,
            super::Command::RunWatch {
                single_threaded: true,
                profile: true,
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([watch.as_slice(), &["--watch-workers", "3"]].concat())
                .unwrap()
                .command,
            super::Command::RunWatch {
                single_threaded: false,
                watch_workers: Some(3),
                ..
            }
        ));
        assert!(matches!(
            super::Cli::try_parse_from([watch.as_slice(), &["--no-vm-accounting"]].concat())
                .unwrap()
                .command,
            super::Command::RunWatch {
                no_vm_accounting: true,
                ..
            }
        ));
        assert!(super::Cli::try_parse_from([
            "renderer",
            "run-watch",
            "engine.zip",
            "level.json",
            "--single-threaded",
            "--watch-workers",
            "2"
        ])
        .is_err());
    }

    #[test]
    fn compile_engine_is_an_explicit_cache_prewarm_command() {
        assert!(matches!(
            super::Cli::try_parse_from(["renderer", "compile-engine", "engine.zip"])
                .unwrap()
                .command,
            super::Command::CompileEngine { engine }
                if engine == PathBuf::from("engine.zip")
        ));
    }

    #[test]
    fn render_video_profile_flags_are_opt_in_and_frames_require_profile() {
        let base = [
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
        ];
        assert!(super::Cli::try_parse_from(base).is_ok());
        let validation = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--validate-determinism",
        ])
        .unwrap();
        assert!(matches!(
            validation.command,
            super::Command::RenderVideo {
                validate_determinism: true,
                ..
            }
        ));
        let sequential = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--sequential-sfx-prepass",
        ])
        .unwrap();
        assert!(matches!(
            sequential.command,
            super::Command::RenderVideo {
                sequential_sfx_prepass: true,
                ..
            }
        ));
        let sequential_frames = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--sequential-frames",
        ])
        .unwrap();
        assert!(matches!(
            sequential_frames.command,
            super::Command::RenderVideo {
                sequential_frames: true,
                ..
            }
        ));
        assert!(super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--sequential-frames",
            "--concurrent-sfx-prepass",
        ])
        .is_ok());
        assert!(super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--concurrent-sfx-prepass",
            "--sequential-sfx-prepass",
        ])
        .is_err());
        let profiled = super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--profile",
            "--profile-frames",
        ])
        .unwrap();
        match profiled.command {
            super::Command::RenderVideo {
                backend,
                profile,
                profile_frames,
                ..
            } => {
                assert!(profile && profile_frames);
                assert!(matches!(backend, super::BackendArg::Wgpu));
            }
            _ => panic!("expected render-video"),
        }
        assert!(super::Cli::try_parse_from([
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
            "--profile-frames"
        ])
        .is_err());
    }
}

#[cfg(test)]
mod desktop_cli_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn mv_percent_and_particle_sources_parse_in_shared_cli() {
        for value in ["0", "40.5", "100"] {
            let cli = Cli::try_parse_from([
                "renderer",
                "render-video",
                "e.zip",
                "r.scp",
                "l.data",
                "bgm.mp3",
                "out.mp4",
                "--mv-background",
                value,
                "--particle-resources",
                "p.scp",
                "--particle-name",
                "custom",
            ])
            .unwrap();
            match cli.command {
                Command::RenderVideo {
                    mv_background,
                    particle_resources,
                    particle_name,
                    ..
                } => {
                    assert_eq!(mv_background, value.parse::<f64>().unwrap());
                    assert_eq!(particle_resources, Some(PathBuf::from("p.scp")));
                    assert_eq!(particle_name.as_deref(), Some("custom"));
                }
                _ => panic!(),
            }
        }
        for value in ["-1", "100.1", "NaN", "inf"] {
            assert!(Cli::try_parse_from([
                "renderer",
                "render-video",
                "e",
                "r",
                "l",
                "b",
                "o",
                "--mv-background",
                value
            ])
            .is_err());
        }
    }

    #[test]
    fn engine_background_rotation_cli_flag_is_opt_in() {
        let args = [
            "renderer",
            "render-video",
            "engine.zip",
            "resources.scp",
            "level.json",
            "music.mp3",
            "out.mp4",
        ];
        assert!(matches!(
            Cli::try_parse_from(args).unwrap().command,
            Command::RenderVideo {
                apply_engine_background_rotation: false,
                ..
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(
                [args.as_slice(), &["--apply-engine-background-rotation"]].concat()
            )
            .unwrap()
            .command,
            Command::RenderVideo {
                apply_engine_background_rotation: true,
                ..
            }
        ));
    }
}

#[cfg(test)]
mod job_cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn jobs_subcommands_parse_with_store_override() {
        assert!(matches!(
            Cli::try_parse_from([
                "renderer",
                "jobs",
                "--store",
                "queue.json",
                "submit",
                "config.json",
                "--priority",
                "high",
                "--enqueue-only",
            ])
            .unwrap()
            .command,
            Command::Jobs {
                store: Some(store),
                action: JobAction::Submit {
                    priority: Some(JobPriorityArg::High),
                    enqueue_only: true,
                    ..
                }
            } if store == PathBuf::from("queue.json")
        ));
        assert!(matches!(
            Cli::try_parse_from(["renderer", "jobs", "cancel", "job-123"])
                .unwrap()
                .command,
            Command::Jobs {
                action: JobAction::Cancel { id },
                ..
            } if id == "job-123"
        ));
    }

    #[test]
    fn history_commands_parse_filters_rerender_and_machine_output() {
        assert!(matches!(
            Cli::try_parse_from([
                "renderer", "history", "list", "--search", "chart", "--status", "failed", "--json"
            ])
            .unwrap()
            .command,
            Command::History {
                action: HistoryAction::List {
                    status: Some(HistoryStatusArg::Failed),
                    json: true,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "renderer", "history", "rerender", "history-job-1-attempt-1",
                "--output", "new.mp4", "--enqueue-only"
            ]).unwrap().command,
            Command::History { action: HistoryAction::Rerender { enqueue_only: true, output: Some(output), .. }, .. }
                if output == PathBuf::from("new.mp4")
        ));
    }

    #[test]
    fn history_rerender_cli_submits_a_new_queue_job_from_the_saved_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let queue_path = dir.path().join("jobs.json");
        let history_path = dir.path().join("history");
        let mut config = renderer::render::RenderConfig::default();
        let engine = dir.path().join("engine.zip");
        let resources = dir.path().join("resources.scp");
        let level = dir.path().join("level.data");
        std::fs::write(&engine, b"engine").unwrap();
        std::fs::write(&resources, b"resources").unwrap();
        std::fs::write(&level, b"level").unwrap();
        config.engine = engine;
        config.resources = resources;
        config.level = level;
        config.layers.bgm = false;
        config.output = Some(dir.path().join("old.mp4"));
        let manager = renderer::job_manager::RenderJobManager::open_at(
            &queue_path,
            renderer::job_manager::JobManagerConfig::default(),
        )
        .unwrap();
        let original = manager.enqueue(config, None).unwrap();
        manager.cancel(&original).unwrap();
        let history = manager.history_store().unwrap();
        let source = history
            .list(&renderer::render_history::HistoryQuery::default())
            .entries
            .remove(0);
        assert_eq!(
            source.status,
            renderer::render_history::HistoryStatus::Cancelled
        );
        drop(manager);

        let original_engine = source.config_snapshot.engine.clone();
        std::fs::remove_file(&original_engine).unwrap();
        let missing_error = run_history_command(
            Some(history_path.clone()),
            Some(queue_path.clone()),
            HistoryAction::Rerender {
                id: source.id.clone(),
                output: Some(dir.path().join("missing-input.mp4")),
                enqueue_only: true,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(missing_error.contains("engine"), "{missing_error}");
        std::fs::write(&original_engine, b"engine").unwrap();
        let old_output = source.output.path.as_ref().unwrap();
        std::fs::write(old_output, b"existing output").unwrap();
        let collision_error = run_history_command(
            Some(history_path.clone()),
            Some(queue_path.clone()),
            HistoryAction::Rerender {
                id: source.id.clone(),
                output: None,
                enqueue_only: true,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(
            collision_error.contains("already exists"),
            "{collision_error}"
        );

        run_history_command(
            Some(history_path),
            Some(queue_path.clone()),
            HistoryAction::Rerender {
                id: source.id.clone(),
                output: Some(dir.path().join("new.mp4")),
                enqueue_only: true,
            },
        )
        .unwrap();
        let reopened = renderer::job_manager::RenderJobManager::open_at(
            &queue_path,
            renderer::job_manager::JobManagerConfig::default(),
        )
        .unwrap();
        let rerender = reopened
            .jobs()
            .unwrap()
            .into_iter()
            .find(|job| job.state == renderer::job_manager::JobState::Queued)
            .unwrap();
        assert_ne!(rerender.id, source.job_id);
        assert_eq!(
            rerender.rerendered_from_history_id.as_deref(),
            Some(source.id.as_str())
        );
        assert_eq!(
            rerender.original_job_id.as_deref(),
            Some(source.job_id.as_str())
        );
        assert_eq!(rerender.config.output, Some(dir.path().join("new.mp4")));
    }

    #[test]
    fn cli_submit_list_cancel_and_inspect_use_the_shared_persistent_manager() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine.zip");
        let resources = dir.path().join("resources.scp");
        let level = dir.path().join("level.data");
        std::fs::write(&engine, b"engine").unwrap();
        std::fs::write(&resources, b"resources").unwrap();
        std::fs::write(&level, b"level").unwrap();
        let mut config = renderer::render::RenderConfig::default();
        config.engine = engine;
        config.resources = resources;
        config.level = level;
        config.layers.bgm = false;
        config.output = Some(dir.path().join("queued.mp4"));
        let config_path = dir.path().join("renderer.json");
        config.save_config(&config_path).unwrap();
        let store = dir.path().join("jobs.json");

        run_job_command(
            Some(store.clone()),
            None,
            JobAction::Submit {
                config: Some(config_path),
                profile: None,
                priority: Some(JobPriorityArg::Low),
                enqueue_only: true,
            },
        )
        .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
        let id = persisted["jobs"][0]["id"].as_str().unwrap().to_owned();
        run_job_command(Some(store.clone()), None, JobAction::List).unwrap();
        run_job_command(
            Some(store.clone()),
            None,
            JobAction::Inspect { id: id.clone() },
        )
        .unwrap();
        run_job_command(
            Some(store.clone()),
            None,
            JobAction::Cancel { id: id.clone() },
        )
        .unwrap();
        let manager = renderer::job_manager::RenderJobManager::open_at(
            &store,
            renderer::job_manager::JobManagerConfig::default(),
        )
        .unwrap();
        assert_eq!(
            manager.inspect(&id).unwrap().state,
            renderer::job_manager::JobState::Cancelled
        );
    }

    #[test]
    fn profile_cli_supports_crud_import_export_and_selection() {
        let dir = tempfile::tempdir().unwrap();
        let profile_store = dir.path().join("profiles.json");
        let manager =
            renderer::config_profiles::ConfigProfileManager::open_at(&profile_store).unwrap();
        let config_path = dir.path().join("settings.json");
        let mut config = renderer::render::RenderConfig::default();
        config.width = 1280;
        config.execution_mode = renderer::sono_gcc::WatchExecutionMode::SonoGcc;
        config.save_config(&config_path).unwrap();

        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Create {
                name: "High Quality".into(),
                config: Some(config_path.clone()),
            },
        )
        .unwrap();
        let original = manager.get("high quality").unwrap();
        assert_eq!(original.config.width, 1280);
        assert_eq!(
            original.config.execution_mode,
            renderer::sono_gcc::WatchExecutionMode::SonoGcc
        );
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Duplicate {
                selector: original.id.clone(),
                name: "High Quality Copy".into(),
            },
        )
        .unwrap();
        let duplicate = manager.get("High Quality Copy").unwrap();
        assert_ne!(original.id, duplicate.id);
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Rename {
                selector: duplicate.id.clone(),
                name: "Draft".into(),
            },
        )
        .unwrap();
        let edited_path = dir.path().join("edited.json");
        let mut edited = original.config.clone();
        edited.width = 1920;
        edited.save_config(&edited_path).unwrap();
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Update {
                selector: original.id.clone(),
                config: edited_path,
            },
        )
        .unwrap();
        assert_eq!(manager.get(&original.id).unwrap().config.width, 1920);

        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Select {
                selector: original.id.clone(),
            },
        )
        .unwrap();
        assert_eq!(manager.active().unwrap().unwrap().id, original.id);
        let export_path = dir.path().join("profile-export.json");
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Export {
                selector: original.id.clone(),
                path: export_path.clone(),
            },
        )
        .unwrap();
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Import {
                path: export_path,
                name: Some("Portable Copy".into()),
            },
        )
        .unwrap();
        assert_eq!(manager.get("Portable Copy").unwrap().config.width, 1920);
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Reset {
                selector: "Draft".into(),
            },
        )
        .unwrap();
        assert_eq!(manager.get("Draft").unwrap().config, Default::default());
        run_profile_command(Some(profile_store.clone()), ProfileAction::Deselect).unwrap();
        assert!(manager.active().unwrap().is_none());
        run_profile_command(
            Some(profile_store.clone()),
            ProfileAction::Delete {
                selector: "Draft".into(),
            },
        )
        .unwrap();
        assert!(manager.get("Draft").is_err());
    }

    #[test]
    fn cli_profile_job_keeps_resolved_snapshot_after_profile_changes_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let engine = dir.path().join("engine.zip");
        let resources = dir.path().join("resources.scp");
        let level = dir.path().join("level.data");
        for path in [&engine, &resources, &level] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let mut config = renderer::render::RenderConfig::default();
        config.engine = engine;
        config.resources = resources;
        config.level = level;
        config.output = Some(dir.path().join("from-profile.mp4"));
        config.layers.bgm = false;
        config.width = 640;
        let profile_path = dir.path().join("profiles.json");
        let profiles =
            renderer::config_profiles::ConfigProfileManager::open_at(&profile_path).unwrap();
        let profile = profiles.create("Queue Preview", config.clone()).unwrap();
        let queue_path = dir.path().join("jobs.json");
        run_job_command(
            Some(queue_path.clone()),
            Some(profile_path),
            JobAction::Submit {
                config: None,
                profile: Some(profile.id.clone()),
                priority: None,
                enqueue_only: true,
            },
        )
        .unwrap();
        let queue = renderer::job_manager::RenderJobManager::open_at(
            &queue_path,
            renderer::job_manager::JobManagerConfig::default(),
        )
        .unwrap();
        let job = queue.jobs().unwrap().remove(0);
        assert_eq!(job.profile_id.as_deref(), Some(profile.id.as_str()));
        assert_eq!(job.profile_name.as_deref(), Some("Queue Preview"));
        assert_eq!(job.config, config);

        let mut changed = config.clone();
        changed.width = 1280;
        profiles.update(&profile.id, changed).unwrap();
        profiles.delete(&profile.id).unwrap();
        let retained = queue.inspect(&job.id).unwrap();
        assert_eq!(retained.config.width, 640);
        assert_eq!(retained.profile_id.as_deref(), Some(profile.id.as_str()));
    }
}

#[cfg(test)]
mod presentation_asset_cli_tests {
    use super::*;

    #[test]
    fn asset_pack_inspection_command_parses_human_and_structured_modes() {
        let human = Cli::try_parse_from([
            "renderer",
            "inspect-presentation-asset-pack",
            "assets/presentation-pack.json",
        ])
        .unwrap();
        assert!(matches!(
            human.command,
            Command::InspectPresentationAssetPack { json: false, .. }
        ));

        let structured = Cli::try_parse_from([
            "renderer",
            "inspect-presentation-asset-pack",
            "assets/presentation-pack.json",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            structured.command,
            Command::InspectPresentationAssetPack { json: true, .. }
        ));
    }
}

#[cfg(test)]
mod manifest_cli_tests {
    use super::*;

    #[test]
    fn manifest_cli_supports_export_inspect_verify_diff_and_reproduction() {
        let parsed = Cli::try_parse_from([
            "renderer",
            "manifest",
            "export",
            "history-job-attempt-1",
            "capture.json",
            "--history-store",
            "history",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Manifest {
                action: ManifestAction::Export { .. }
            }
        ));

        let parsed = Cli::try_parse_from([
            "renderer",
            "manifest",
            "verify",
            "capture.json",
            "--asset",
            "engine=C:/charts/engine.zip",
            "--json",
        ])
        .unwrap();
        assert!(matches!(
            parsed.command,
            Command::Manifest {
                action: ManifestAction::Verify {
                    assets,
                    json: true,
                    ..
                }
            } if assets == vec!["engine=C:/charts/engine.zip"]
        ));

        assert!(matches!(
            Cli::try_parse_from(["renderer", "manifest", "diff", "old.json", "new.json", "--json"])
                .unwrap()
                .command,
            Command::Manifest {
                action: ManifestAction::Diff { json: true, .. }
            }
        ));

        assert!(matches!(
            Cli::try_parse_from([
                "renderer",
                "manifest",
                "reproduce",
                "capture.json",
                "--asset",
                "engine=engine.zip",
                "--output",
                "new.mp4",
                "--enqueue-only"
            ])
            .unwrap()
            .command,
            Command::Manifest {
                action: ManifestAction::Reproduce {
                    enqueue_only: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn manifest_asset_repair_parser_rejects_ambiguous_duplicate_slots() {
        assert!(parse_manifest_assets(&["engine=a.zip".into(), "engine=b.zip".into()]).is_err());
        assert!(parse_manifest_assets(&["engine".into()]).is_err());
        assert_eq!(
            parse_manifest_assets(&["ui.asset:logo=C:/chart/logo.png".into()])
                .unwrap()
                .get("ui.asset:logo"),
            Some(&PathBuf::from("C:/chart/logo.png"))
        );
    }
}
