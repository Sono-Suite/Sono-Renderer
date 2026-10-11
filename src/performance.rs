//! Authoritative render-performance records, comparison, benchmarking, and history.
//!
//! Export phases are disjoint wall-clock spans. Optional frame-level profile
//! measurements are attribution data and may be nested or overlap; consumers
//! must not add them to the phase totals. Pipeline worker totals are likewise
//! reported as overlapping lane work alongside the pipeline wall span.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const PERFORMANCE_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_RETENTION: usize = 500;
const MAX_BENCHMARK_RUNS: usize = 20;
static NEXT_RECORD_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageSemantics {
    /// Disjoint export phase span; these rows partition execution wall time.
    ExclusiveWallSpan,
    /// Per-frame/per-worker attribution; nested or concurrent work may overlap.
    OverlappingAttribution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StageTiming {
    pub name: String,
    pub duration_ms: f64,
    pub samples: u64,
    pub semantics: StageSemantics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadIdentity {
    pub engine_fingerprint: String,
    pub resources_fingerprint: String,
    pub level_fingerprint: String,
    pub music_fingerprint: String,
    pub effective_config_fingerprint: String,
    pub resolution: [u32; 2],
    pub fps: u32,
    pub start_time_micros: i64,
    pub duration_micros: i64,
    pub backend: String,
    pub execution_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineOverlap {
    pub wall_span_ms: f64,
    pub producer_watch_ms: f64,
    pub producer_snapshot_ms: f64,
    pub producer_blocked_ms: f64,
    pub queue_residence_ms: f64,
    pub consumer_waiting_ms: f64,
    pub render_lane_ms: f64,
    pub output_lane_ms: f64,
    pub queue_capacity: usize,
    pub queue_high_water: usize,
    pub render_workers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceEnvironment {
    pub renderer_version: String,
    pub renderer_commit: Option<String>,
    pub operating_system: String,
    pub architecture: String,
    pub cpu_identity: Option<String>,
    /// wgpu adapter identity is reported by deep frame profiling when available.
    pub gpu_identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SonoGccCompilationMetrics {
    pub cache_state: String,
    pub cache_lookup_ms: f64,
    pub compilation_ms: f64,
    pub native_load_ms: f64,
    pub graph_identity_ms: f64,
    pub disk_validation_ms: f64,
    pub metadata_reconstruction_ms: f64,
    pub cache_write_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderPerformanceRecord {
    pub schema_version: u32,
    pub id: String,
    pub created_unix_time_ms: u128,
    pub started_unix_time_ms: u128,
    pub completed_unix_time_ms: u128,
    /// Monotonic wall duration for renderer export execution only.
    pub execution_time_ms: f64,
    /// Queue residence from an in-process monotonic enqueue instant, when known.
    pub queue_wait_time_ms: Option<f64>,
    /// Queue wait plus execution; excludes later inspection/user delay.
    pub total_user_latency_ms: Option<f64>,
    pub job_id: Option<String>,
    pub benchmark_id: Option<String>,
    pub benchmark_role: Option<String>,
    pub workload: WorkloadIdentity,
    pub effective_backend_status: String,
    pub gcc_cache_state: Option<String>,
    /// Compiler/runtime identity captured by the renderer process that ran
    /// this attempt, not reconstructed later when history is exported.
    #[serde(default)]
    pub gcc_codegen_version: Option<String>,
    #[serde(default)]
    pub gcc_runtime_abi: Option<String>,
    pub sono_gcc_compilation: Option<SonoGccCompilationMetrics>,
    pub stage_timings: Vec<StageTiming>,
    pub pipeline_overlap: Option<PipelineOverlap>,
    pub frames: u64,
    pub watch_runtime_frames: u64,
    pub watch_callbacks: u64,
    pub watch_evaluations: u64,
    pub sono_gcc_regions_executed: u64,
    pub profiling_detail_enabled: bool,
    pub environment: PerformanceEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkDistribution {
    pub samples: usize,
    pub min_ms: f64,
    pub median_ms: f64,
    pub mean_ms: f64,
    pub max_ms: f64,
    /// Nearest-rank p95, only populated for at least 20 measured samples.
    pub p95_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkBackendResult {
    pub execution_mode: String,
    /// First Sono-GCC startup observation, excluded from the warm distribution.
    pub initial_cache_observation: Option<RenderPerformanceRecord>,
    pub samples: Vec<RenderPerformanceRecord>,
    pub distribution: BenchmarkDistribution,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackendMedianComparison {
    pub interpreter_median_ms: f64,
    pub sono_gcc_median_ms: f64,
    /// Positive means Sono-GCC took longer.
    pub absolute_difference_ms: f64,
    pub percentage_difference: Option<f64>,
    pub comparable: bool,
    pub reason: Option<String>,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TelemetryOverheadEstimate {
    pub lightweight_median_ms: f64,
    pub detailed_median_ms: f64,
    pub overhead_ms: f64,
    pub overhead_percent: Option<f64>,
    pub classification: String,
    pub detailed_samples: Vec<RenderPerformanceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub created_unix_time_ms: u128,
    pub warmup_runs: usize,
    pub measured_runs_per_backend: usize,
    pub backends: Vec<BenchmarkBackendResult>,
    pub vm_vs_sono_gcc: Option<BackendMedianComparison>,
    pub telemetry_overhead: Option<TelemetryOverheadEstimate>,
}

pub fn save_benchmark_report_atomic(
    path: impl AsRef<Path>,
    report: &BenchmarkReport,
) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        bail!("refusing to overwrite benchmark report: {}", path.display());
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating report directory {}", parent.display()))?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let bytes = serde_json::to_vec_pretty(report)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".renderer-benchmark-")
        .suffix(".json.tmp")
        .tempfile_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)
        .map_err(|error| error.error)
        .with_context(|| format!("publishing benchmark report {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceComparison {
    pub left_id: String,
    pub right_id: String,
    pub comparable: bool,
    pub differences: Vec<String>,
    pub left_execution_ms: f64,
    pub right_execution_ms: f64,
    pub absolute_difference_ms: f64,
    /// Positive means the right record took longer.
    pub percentage_difference: Option<f64>,
    /// Changes under 5% or 2ms are treated as noise, not a regression.
    pub classification: String,
}

impl RenderPerformanceRecord {
    pub fn new(
        config: &crate::render::RenderConfig,
        started_unix_time_ms: u128,
        completed_unix_time_ms: u128,
        execution_time: Duration,
        stage_timings: Vec<StageTiming>,
        pipeline_overlap: Option<PipelineOverlap>,
        submitted_frames: u64,
        watch_runtime_frames: u64,
        watch_callbacks: u64,
        watch_evaluations: u64,
        sono_gcc_regions_executed: u64,
        effective_backend_status: String,
    ) -> Self {
        let created_unix_time_ms = unix_time_ms();
        let cache_state = if config.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
            if effective_backend_status.contains("persistent cache hit") {
                Some("persistent_hit".to_owned())
            } else if effective_backend_status.contains("process cache hit") {
                Some("process_hit".to_owned())
            } else if effective_backend_status.contains("Sono-GCC ready (compiled)") {
                Some("miss_compiled".to_owned())
            } else if effective_backend_status.contains("Sono-GCC unavailable")
                || effective_backend_status.contains("using Sono VM")
            {
                Some("fallback".to_owned())
            } else {
                Some("unknown".to_owned())
            }
        } else {
            None
        };
        let (gcc_codegen_version, gcc_runtime_abi) =
            if config.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                let (codegen, abi) = crate::sono_gcc::compiler_identity();
                (Some(codegen.to_owned()), Some(abi.to_owned()))
            } else {
                (None, None)
            };
        Self {
            schema_version: PERFORMANCE_SCHEMA_VERSION,
            id: unique_id(created_unix_time_ms),
            created_unix_time_ms,
            started_unix_time_ms,
            completed_unix_time_ms,
            execution_time_ms: execution_time.as_secs_f64() * 1000.0,
            queue_wait_time_ms: None,
            total_user_latency_ms: None,
            job_id: None,
            benchmark_id: None,
            benchmark_role: None,
            workload: workload_identity(config),
            effective_backend_status,
            gcc_cache_state: cache_state,
            gcc_codegen_version,
            gcc_runtime_abi,
            sono_gcc_compilation: None,
            stage_timings,
            pipeline_overlap,
            frames: submitted_frames,
            watch_runtime_frames,
            watch_callbacks,
            watch_evaluations,
            sono_gcc_regions_executed,
            profiling_detail_enabled: config.profile,
            environment: environment_metadata(),
        }
    }
}

pub fn compare_records(
    left: &RenderPerformanceRecord,
    right: &RenderPerformanceRecord,
) -> PerformanceComparison {
    let mut differences = Vec::new();
    let a = &left.workload;
    let b = &right.workload;
    macro_rules! compare_field {
        ($field:ident, $label:literal) => {
            if a.$field != b.$field {
                differences.push($label.to_owned());
            }
        };
    }
    compare_field!(engine_fingerprint, "engine identity");
    compare_field!(resources_fingerprint, "resources identity");
    compare_field!(level_fingerprint, "level identity");
    compare_field!(music_fingerprint, "music identity");
    compare_field!(effective_config_fingerprint, "effective configuration");
    compare_field!(resolution, "resolution");
    compare_field!(fps, "FPS");
    compare_field!(start_time_micros, "start time");
    compare_field!(duration_micros, "duration");
    compare_field!(backend, "render backend");
    if left.environment.renderer_version != right.environment.renderer_version
        || left.environment.renderer_commit != right.environment.renderer_commit
    {
        differences.push("renderer build".into());
    }
    if left.environment.operating_system != right.environment.operating_system
        || left.environment.architecture != right.environment.architecture
        || left.environment.cpu_identity != right.environment.cpu_identity
        || left.environment.gpu_identity != right.environment.gpu_identity
    {
        differences.push("execution environment".into());
    }
    if left.job_id.is_some() != right.job_id.is_some() {
        differences.push("execution scope (direct versus queued job)".into());
    }
    let left_mode = left.workload.execution_mode.as_str();
    let right_mode = right.workload.execution_mode.as_str();
    if left_mode != right_mode {
        // A VM/GCC comparison is meaningful only if the GCC request actually
        // ran native code. Sono-GCC may safely fall back to the VM.
        let left_native = confirmed_native_gcc(left);
        let right_native = confirmed_native_gcc(right);
        if !matches!(
            (left_native, right_native),
            (Some(false), Some(true)) | (Some(true), Some(false))
        ) {
            differences.push("effective Watch execution backend".into());
        }
    } else if left_mode == "sono_gcc" && confirmed_native_gcc(left) != confirmed_native_gcc(right) {
        differences.push("effective Watch execution backend".into());
    }
    let absolute_difference_ms = right.execution_time_ms - left.execution_time_ms;
    let percentage_difference = (left.execution_time_ms > 0.0)
        .then_some(absolute_difference_ms / left.execution_time_ms * 100.0);
    let comparable = differences.is_empty();
    let classification = if !comparable {
        "not_comparable"
    } else if absolute_difference_ms.abs() <= 2.0
        || percentage_difference.is_some_and(|value| value.abs() < 5.0)
    {
        "within_noise"
    } else if absolute_difference_ms > 0.0 {
        "slower"
    } else {
        "faster"
    };
    PerformanceComparison {
        left_id: left.id.clone(),
        right_id: right.id.clone(),
        comparable,
        differences,
        left_execution_ms: left.execution_time_ms,
        right_execution_ms: right.execution_time_ms,
        absolute_difference_ms,
        percentage_difference,
        classification: classification.into(),
    }
}

fn confirmed_native_gcc(record: &RenderPerformanceRecord) -> Option<bool> {
    match record.workload.execution_mode.as_str() {
        "interpreter" => Some(false),
        "sono_gcc" => match record.gcc_cache_state.as_deref() {
            Some("miss_compiled" | "process_hit" | "persistent_hit") => Some(true),
            Some("fallback") => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn compare_backend_medians(
    vm: &BenchmarkBackendResult,
    gcc: &BenchmarkBackendResult,
) -> BackendMedianComparison {
    let absolute_difference_ms = gcc.distribution.median_ms - vm.distribution.median_ms;
    let percentage_difference = (vm.distribution.median_ms > 0.0)
        .then_some(absolute_difference_ms / vm.distribution.median_ms * 100.0);
    let gcc_has_fallback = gcc
        .samples
        .iter()
        .any(|sample| sample.gcc_cache_state.as_deref() == Some("fallback"));
    let gcc_has_unconfirmed_backend = gcc
        .samples
        .iter()
        .any(|sample| confirmed_native_gcc(sample) != Some(true));
    let comparable = !gcc_has_fallback && !gcc_has_unconfirmed_backend;
    let reason = if gcc_has_fallback {
        Some("one or more measured Sono-GCC runs fell back to the Sono VM".into())
    } else if gcc_has_unconfirmed_backend {
        Some("native Sono-GCC execution was not confirmed for every measured run".into())
    } else {
        None
    };
    let classification = if !comparable {
        "not_comparable_effective_backend"
    } else if absolute_difference_ms.abs() <= 2.0
        || percentage_difference.is_some_and(|value| value.abs() < 5.0)
    {
        "within_noise"
    } else if absolute_difference_ms > 0.0 {
        "sono_gcc_slower"
    } else {
        "sono_gcc_faster"
    };
    BackendMedianComparison {
        interpreter_median_ms: vm.distribution.median_ms,
        sono_gcc_median_ms: gcc.distribution.median_ms,
        absolute_difference_ms,
        percentage_difference,
        comparable,
        reason,
        classification: classification.into(),
    }
}

pub fn distribution(values: &[f64]) -> Option<BenchmarkDistribution> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let median_ms = if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    };
    Some(BenchmarkDistribution {
        samples: sorted.len(),
        min_ms: sorted[0],
        median_ms,
        mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        max_ms: *sorted.last().unwrap_or(&0.0),
        p95_ms: (sorted.len() >= 20)
            .then(|| sorted[((sorted.len() as f64 * 0.95).ceil() as usize).min(sorted.len()) - 1]),
    })
}

#[derive(Debug, Clone)]
pub struct PerformanceStore {
    root: PathBuf,
    retention: usize,
}

impl PerformanceStore {
    pub fn open_default() -> Result<Self> {
        Self::open_at(default_store_path(), DEFAULT_RETENTION)
    }

    pub fn open_at(path: impl Into<PathBuf>, retention: usize) -> Result<Self> {
        let root = path.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating performance history {}", root.display()))?;
        Ok(Self {
            root,
            retention: retention.max(1),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self, record: &RenderPerformanceRecord) -> Result<()> {
        validate_record(record)?;
        let path = self.record_path(&record.id)?;
        let bytes = serde_json::to_vec_pretty(record)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".performance-")
            .suffix(".tmp")
            .tempfile_in(&self.root)
            .context("creating atomic performance-record temporary file")?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("publishing performance record {}", path.display()))?;
        self.prune()?;
        Ok(())
    }

    pub fn list(&self, limit: usize) -> Result<Vec<RenderPerformanceRecord>> {
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let bytes = fs::read(entry.path()).with_context(|| {
                    format!("reading performance record {}", entry.path().display())
                })?;
                let record: RenderPerformanceRecord =
                    serde_json::from_slice(&bytes).with_context(|| {
                        format!("decoding performance record {}", entry.path().display())
                    })?;
                validate_record(&record)?;
                records.push(record);
            }
        }
        records.sort_by_key(|record| (record.created_unix_time_ms, record.id.clone()));
        records.reverse();
        records.truncate(limit);
        Ok(records)
    }

    pub fn get(&self, id: &str) -> Result<RenderPerformanceRecord> {
        let path = self.record_path(id)?;
        let bytes = fs::read(&path).with_context(|| format!("reading performance record {id}"))?;
        let record: RenderPerformanceRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("decoding performance record {id}"))?;
        validate_record(&record)?;
        Ok(record)
    }

    fn record_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            bail!("invalid performance record ID");
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    fn prune(&self) -> Result<()> {
        let records = self.list(usize::MAX)?;
        for expired in records.into_iter().skip(self.retention) {
            fs::remove_file(self.record_path(&expired.id)?)
                .with_context(|| format!("removing expired performance record {}", expired.id))?;
        }
        Ok(())
    }
}

pub fn run_benchmark(
    config: crate::render::RenderConfig,
    warmup_runs: usize,
    measured_runs: usize,
    compare_vm_gcc: bool,
    measure_telemetry_overhead: bool,
) -> Result<BenchmarkReport> {
    if warmup_runs > 10 {
        bail!("benchmark warm-up count must be at most 10");
    }
    if !(1..=MAX_BENCHMARK_RUNS).contains(&measured_runs) {
        bail!("benchmark measured run count must be in 1..={MAX_BENCHMARK_RUNS}");
    }
    let temporary = tempfile::tempdir().context("creating isolated benchmark output directory")?;
    let mut validation = config.clone();
    validation.output = Some(temporary.path().join("validation.mp4"));
    validation.validate_export_inputs()?;
    let mut modes = vec![config.execution_mode];
    if compare_vm_gcc {
        modes = vec![
            crate::sono_gcc::WatchExecutionMode::Interpreter,
            crate::sono_gcc::WatchExecutionMode::SonoGcc,
        ];
    }
    let benchmark_id = unique_id(unix_time_ms());
    let store = PerformanceStore::open_default()?;
    let label = |mode| match mode {
        crate::sono_gcc::WatchExecutionMode::Interpreter => "interpreter",
        crate::sono_gcc::WatchExecutionMode::SonoGcc => "sono_gcc",
    };
    let mut initial_cache_observations = BTreeMap::new();
    for mode in &modes {
        if *mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
            let mut sample = config.clone();
            sample.execution_mode = *mode;
            sample.compile_sono_gcc_on_demand = true;
            sample.profile = false;
            sample.profile_frames = false;
            sample.output = Some(temporary.path().join("sono_gcc-cache-observation.mp4"));
            let report = sample
                .render_video_unrecorded()
                .context("Sono-GCC initial cache observation failed")?;
            let mut record = report.performance;
            record.benchmark_id = Some(benchmark_id.clone());
            record.benchmark_role = Some("initial_cache_observation".into());
            store.save(&record)?;
            initial_cache_observations.insert(label(*mode).to_owned(), record);
        }
    }
    for warmup in 0..warmup_runs {
        let mut order = modes.clone();
        if warmup % 2 == 1 {
            order.reverse();
        }
        for mode in order {
            let backend = label(mode);
            let mut sample = config.clone();
            sample.execution_mode = mode;
            if mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                sample.compile_sono_gcc_on_demand = true;
            }
            sample.profile = false;
            sample.profile_frames = false;
            sample.output = Some(
                temporary
                    .path()
                    .join(format!("{backend}-warmup-{warmup}.mp4")),
            );
            sample
                .render_video_unrecorded()
                .with_context(|| format!("{backend} benchmark warm-up {} failed", warmup + 1))?;
        }
    }
    let mut samples_by_backend = modes
        .iter()
        .map(|mode| (label(*mode).to_owned(), Vec::with_capacity(measured_runs)))
        .collect::<BTreeMap<_, _>>();
    for iteration in 0..measured_runs {
        let mut order = modes.clone();
        if iteration % 2 == 1 {
            order.reverse();
        }
        for mode in order {
            let backend = label(mode);
            let mut sample = config.clone();
            sample.execution_mode = mode;
            if mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                sample.compile_sono_gcc_on_demand = true;
            }
            sample.profile = false;
            sample.profile_frames = false;
            sample.output = Some(
                temporary
                    .path()
                    .join(format!("{backend}-sample-{iteration}.mp4")),
            );
            let report = sample
                .render_video_unrecorded()
                .with_context(|| format!("{backend} benchmark sample {} failed", iteration + 1))?;
            let mut record = report.performance;
            record.benchmark_id = Some(benchmark_id.clone());
            record.benchmark_role = Some("measured".into());
            store.save(&record)?;
            samples_by_backend
                .get_mut(backend)
                .expect("backend sample vector exists")
                .push(record);
        }
    }
    let mut backends = Vec::new();
    for mode in modes {
        let backend = label(mode).to_owned();
        let samples = samples_by_backend.remove(&backend).unwrap_or_default();
        let distribution = distribution(
            &samples
                .iter()
                .map(|record| record.execution_time_ms)
                .collect::<Vec<_>>(),
        )
        .expect("validated benchmark has at least one measured run");
        backends.push(BenchmarkBackendResult {
            execution_mode: backend.clone(),
            initial_cache_observation: initial_cache_observations.remove(&backend),
            samples,
            distribution,
        });
    }
    let vm = backends
        .iter()
        .find(|result| result.execution_mode == "interpreter");
    let gcc = backends
        .iter()
        .find(|result| result.execution_mode == "sono_gcc");
    let vm_vs_sono_gcc = vm
        .zip(gcc)
        .map(|(vm, gcc)| compare_backend_medians(vm, gcc));
    let telemetry_overhead = if measure_telemetry_overhead {
        let baseline_mode = match config.execution_mode {
            crate::sono_gcc::WatchExecutionMode::Interpreter => "interpreter",
            crate::sono_gcc::WatchExecutionMode::SonoGcc => "sono_gcc",
        };
        let baseline = backends
            .iter()
            .find(|backend| backend.execution_mode == baseline_mode)
            .context("selected execution mode is missing from benchmark results")?;
        for warmup in 0..warmup_runs {
            let mut sample = config.clone();
            if sample.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                sample.compile_sono_gcc_on_demand = true;
            }
            sample.profile = true;
            sample.profile_frames = false;
            sample.output = Some(
                temporary
                    .path()
                    .join(format!("profile-warmup-{warmup}.mp4")),
            );
            sample
                .render_video_unrecorded()
                .with_context(|| format!("detailed telemetry warm-up {} failed", warmup + 1))?;
        }
        let mut detailed_samples = Vec::with_capacity(measured_runs);
        for iteration in 0..measured_runs {
            let mut sample = config.clone();
            if sample.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                sample.compile_sono_gcc_on_demand = true;
            }
            sample.profile = true;
            sample.profile_frames = false;
            sample.output = Some(
                temporary
                    .path()
                    .join(format!("profile-sample-{iteration}.mp4")),
            );
            let report = sample
                .render_video_unrecorded()
                .with_context(|| format!("detailed telemetry sample {} failed", iteration + 1))?;
            let mut record = report.performance;
            record.benchmark_id = Some(benchmark_id.clone());
            record.benchmark_role = Some("detailed_telemetry".into());
            store.save(&record)?;
            detailed_samples.push(record);
        }
        let detailed_distribution = distribution(
            &detailed_samples
                .iter()
                .map(|record| record.execution_time_ms)
                .collect::<Vec<_>>(),
        )
        .expect("validated benchmark has measured runs");
        let overhead_ms = detailed_distribution.median_ms - baseline.distribution.median_ms;
        let overhead_percent = (baseline.distribution.median_ms > 0.0)
            .then_some(overhead_ms / baseline.distribution.median_ms * 100.0);
        let classification = if overhead_ms.abs() <= 2.0
            || overhead_percent.is_some_and(|value| value.abs() < 5.0)
        {
            "within_noise"
        } else if overhead_ms > 0.0 {
            "detailed_profile_added_cost"
        } else {
            "measurement_noise_or_cache_shift"
        };
        Some(TelemetryOverheadEstimate {
            lightweight_median_ms: baseline.distribution.median_ms,
            detailed_median_ms: detailed_distribution.median_ms,
            overhead_ms,
            overhead_percent,
            classification: classification.into(),
            detailed_samples,
        })
    } else {
        None
    };
    Ok(BenchmarkReport {
        schema_version: PERFORMANCE_SCHEMA_VERSION,
        benchmark_id,
        created_unix_time_ms: unix_time_ms(),
        warmup_runs,
        measured_runs_per_backend: measured_runs,
        backends,
        vm_vs_sono_gcc,
        telemetry_overhead,
    })
}

pub fn default_store_path() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!(
            "sono-renderer-performance-tests-{}",
            std::process::id()
        ));
    }
    #[cfg(not(test))]
    {
        crate::store_paths::state_root().join("performance")
    }
}

pub fn stage(name: impl Into<String>, duration: Duration, samples: u64) -> StageTiming {
    StageTiming {
        name: name.into(),
        duration_ms: duration.as_secs_f64() * 1000.0,
        samples,
        semantics: StageSemantics::ExclusiveWallSpan,
    }
}

pub fn pipeline(stats: &crate::video_export::FramePipelineStats) -> PipelineOverlap {
    PipelineOverlap {
        wall_span_ms: stats.render_phase_ms,
        producer_watch_ms: stats.producer_watch_ms,
        producer_snapshot_ms: stats.producer_snapshot_ms,
        producer_blocked_ms: stats.producer_blocked_ms,
        queue_residence_ms: stats.queue_residence_ms,
        consumer_waiting_ms: stats.consumer_waiting_ms,
        render_lane_ms: stats.render_ms,
        output_lane_ms: stats.output_ms,
        queue_capacity: stats.queue_capacity,
        queue_high_water: stats.queue_high_water,
        render_workers: stats.render_workers,
    }
}

pub fn attribution_stage(name: impl Into<String>, duration: Duration, samples: u64) -> StageTiming {
    StageTiming {
        name: name.into(),
        duration_ms: duration.as_secs_f64() * 1000.0,
        samples,
        semantics: StageSemantics::OverlappingAttribution,
    }
}

fn validate_record(record: &RenderPerformanceRecord) -> Result<()> {
    if record.schema_version != PERFORMANCE_SCHEMA_VERSION {
        bail!(
            "unsupported performance-record schema version {}",
            record.schema_version
        );
    }
    if record.id.is_empty() {
        bail!("performance record has an empty ID");
    }
    if !record.execution_time_ms.is_finite() || record.execution_time_ms < 0.0 {
        bail!("performance record has an invalid execution duration");
    }
    Ok(())
}

pub(crate) fn workload_identity(config: &crate::render::RenderConfig) -> WorkloadIdentity {
    let mut effective = config.clone();
    effective.output = None;
    // VM/GCC is the comparison variable, while the rest of the effective
    // renderer configuration defines whether these are the same workload.
    effective.execution_mode = crate::sono_gcc::WatchExecutionMode::Interpreter;
    effective.compile_sono_gcc_on_demand = true;
    // Never persist notification credentials or even include them in a fingerprint.
    effective.notifications = Default::default();
    effective.job_manager = Default::default();
    let serialized = serde_json::to_vec(&effective).unwrap_or_default();
    WorkloadIdentity {
        engine_fingerprint: file_fingerprint(&config.engine),
        resources_fingerprint: file_fingerprint(&config.resources),
        level_fingerprint: file_fingerprint(&config.level),
        music_fingerprint: config
            .music
            .as_deref()
            .map_or_else(|| "disabled".into(), file_fingerprint),
        effective_config_fingerprint: digest(&serialized),
        resolution: [config.width, config.height],
        fps: config.fps,
        start_time_micros: finite_micros(config.start_time),
        duration_micros: finite_micros(config.duration),
        backend: format!("{:?}", config.backend).to_lowercase(),
        execution_mode: match config.execution_mode {
            crate::sono_gcc::WatchExecutionMode::Interpreter => "interpreter",
            crate::sono_gcc::WatchExecutionMode::SonoGcc => "sono_gcc",
        }
        .into(),
    }
}

fn file_fingerprint(path: &Path) -> String {
    let mut value = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if let Ok(metadata) = fs::metadata(path) {
        value.push_str(&format!("|{}|", metadata.len()));
        if let Ok(modified) = metadata.modified() {
            value.push_str(
                &modified
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string(),
            );
        }
    }
    digest(value.as_bytes())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn finite_micros(seconds: f64) -> i64 {
    if seconds.is_finite() {
        (seconds * 1_000_000.0).clamp(i64::MIN as f64, i64::MAX as f64) as i64
    } else {
        0
    }
}

fn environment_metadata() -> PerformanceEnvironment {
    PerformanceEnvironment {
        renderer_version: env!("CARGO_PKG_VERSION").into(),
        renderer_commit: option_env!("SONO_RENDERER_GIT_COMMIT").map(str::to_owned),
        operating_system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        cpu_identity: std::env::var("PROCESSOR_IDENTIFIER")
            .ok()
            .filter(|value| !value.is_empty()),
        gpu_identity: None,
    }
}

fn unique_id(timestamp: u128) -> String {
    let sequence = NEXT_RECORD_ID.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:x}-{:x}-{sequence:x}", std::process::id())
}

pub fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, duration: f64) -> RenderPerformanceRecord {
        let config = crate::render::RenderConfig::default();
        let mut record = RenderPerformanceRecord::new(
            &config,
            100,
            200,
            Duration::from_secs_f64(duration / 1000.0),
            vec![stage("Preparing", Duration::from_millis(5), 1)],
            None,
            0,
            0,
            0,
            0,
            0,
            "Sono VM / interpreter selected".into(),
        );
        record.id = id.into();
        record
    }

    #[test]
    fn performance_records_round_trip_and_reject_unknown_schema() {
        let source = record("round-trip", 12.5);
        let value = serde_json::to_value(&source).unwrap();
        let decoded: RenderPerformanceRecord = serde_json::from_value(value).unwrap();
        assert_eq!(source, decoded);
        let mut unknown = decoded;
        unknown.schema_version += 1;
        assert!(validate_record(&unknown).is_err());
    }

    #[test]
    fn compiler_identity_is_captured_only_for_gcc_execution_mode() {
        let mut config = crate::render::RenderConfig::default();
        let vm = RenderPerformanceRecord::new(
            &config,
            100,
            200,
            Duration::from_millis(1),
            Vec::new(),
            None,
            0,
            0,
            0,
            0,
            0,
            "Sono VM / interpreter".into(),
        );
        assert!(vm.gcc_codegen_version.is_none());
        assert!(vm.gcc_runtime_abi.is_none());

        config.execution_mode = crate::sono_gcc::WatchExecutionMode::SonoGcc;
        let gcc = RenderPerformanceRecord::new(
            &config,
            100,
            200,
            Duration::from_millis(1),
            Vec::new(),
            None,
            0,
            0,
            0,
            0,
            0,
            "Sono-GCC ready (test identity)".into(),
        );
        let identity = crate::sono_gcc::compiler_identity();
        assert_eq!(gcc.gcc_codegen_version.as_deref(), Some(identity.0));
        assert_eq!(gcc.gcc_runtime_abi.as_deref(), Some(identity.1));
    }

    #[test]
    fn prior_performance_schema_one_records_default_new_compiler_fields() {
        let mut value = serde_json::to_value(record("legacy", 4.0)).unwrap();
        value.as_object_mut().unwrap().remove("gcc_codegen_version");
        value.as_object_mut().unwrap().remove("gcc_runtime_abi");
        let decoded: RenderPerformanceRecord = serde_json::from_value(value).unwrap();
        assert!(decoded.gcc_codegen_version.is_none());
        assert!(decoded.gcc_runtime_abi.is_none());
    }

    #[test]
    fn persistence_is_atomic_versioned_and_retained() {
        let dir = tempfile::tempdir().unwrap();
        let store = PerformanceStore::open_at(dir.path(), 2).unwrap();
        let mut one = record("one", 1.0);
        one.created_unix_time_ms = 1;
        let mut two = record("two", 2.0);
        two.created_unix_time_ms = 2;
        let mut three = record("three", 3.0);
        three.created_unix_time_ms = 3;
        store.save(&one).unwrap();
        store.save(&two).unwrap();
        store.save(&three).unwrap();
        let listed = store.list(20).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "three");
        assert_eq!(store.get("two").unwrap().execution_time_ms, 2.0);
        assert!(store.get("../two").is_err());
    }

    #[test]
    fn benchmark_report_export_is_atomic_and_refuses_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("benchmark.json");
        let report = BenchmarkReport {
            schema_version: PERFORMANCE_SCHEMA_VERSION,
            benchmark_id: "benchmark-test".into(),
            created_unix_time_ms: 123,
            warmup_runs: 1,
            measured_runs_per_backend: 3,
            backends: Vec::new(),
            vm_vs_sono_gcc: None,
            telemetry_overhead: None,
        };
        save_benchmark_report_atomic(&path, &report).unwrap();
        let saved: BenchmarkReport = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, report);
        assert!(save_benchmark_report_atomic(&path, &report).is_err());
    }

    #[test]
    fn comparisons_require_matching_workloads_and_ignore_small_noise() {
        let left = record("left", 100.0);
        let right = record("right", 103.0);
        let comparison = compare_records(&left, &right);
        assert!(comparison.comparable);
        assert_eq!(comparison.classification, "within_noise");
        let mut unrelated = right;
        unrelated.workload.fps = 60;
        let comparison = compare_records(&left, &unrelated);
        assert!(!comparison.comparable);
        assert!(comparison.differences.iter().any(|item| item == "FPS"));
    }

    #[test]
    fn vm_gcc_comparison_requires_confirmed_native_gcc_execution() {
        let vm = record("vm", 100.0);
        let mut fallback = record("gcc-fallback", 90.0);
        fallback.workload.execution_mode = "sono_gcc".into();
        fallback.gcc_cache_state = Some("fallback".into());
        let comparison = compare_records(&vm, &fallback);
        assert!(!comparison.comparable);
        assert!(comparison
            .differences
            .iter()
            .any(|difference| difference == "effective Watch execution backend"));

        let vm_benchmark = BenchmarkBackendResult {
            execution_mode: "interpreter".into(),
            initial_cache_observation: None,
            samples: vec![vm.clone()],
            distribution: distribution(&[vm.execution_time_ms]).unwrap(),
        };
        let fallback_benchmark = BenchmarkBackendResult {
            execution_mode: "sono_gcc".into(),
            initial_cache_observation: None,
            samples: vec![fallback.clone()],
            distribution: distribution(&[fallback.execution_time_ms]).unwrap(),
        };
        let benchmark_comparison = compare_backend_medians(&vm_benchmark, &fallback_benchmark);
        assert!(!benchmark_comparison.comparable);
        assert_eq!(
            benchmark_comparison.classification,
            "not_comparable_effective_backend"
        );

        let mut native = fallback;
        native.id = "gcc-native".into();
        native.gcc_cache_state = Some("process_hit".into());
        assert!(compare_records(&vm, &native).comparable);
    }

    #[test]
    fn distribution_requires_twenty_samples_for_p95() {
        let small = distribution(&[10.0, 20.0, 30.0]).unwrap();
        assert_eq!(small.median_ms, 20.0);
        assert_eq!(small.p95_ms, None);
        let large = distribution(&(1..=20).map(f64::from).collect::<Vec<_>>()).unwrap();
        assert_eq!(large.p95_ms, Some(19.0));
    }

    #[test]
    fn phase_spans_and_overlapping_attribution_have_distinct_semantics() {
        let phase = stage("Rendering", Duration::from_millis(25), 1);
        let attribution = attribution_stage("Watch VM", Duration::from_millis(40), 10);
        assert_eq!(phase.semantics, StageSemantics::ExclusiveWallSpan);
        assert_eq!(
            attribution.semantics,
            StageSemantics::OverlappingAttribution
        );
        // Attribution is explicitly allowed to exceed the execution wall span;
        // consumers never fold it into the disjoint phase partition.
        assert!(attribution.duration_ms > phase.duration_ms);
    }

    #[test]
    fn queue_wait_and_execution_remain_separate_record_fields() {
        let mut record = record("queue", 80.0);
        record.queue_wait_time_ms = Some(200.0);
        record.total_user_latency_ms = Some(280.0);
        let decoded: RenderPerformanceRecord =
            serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(decoded.execution_time_ms, 80.0);
        assert_eq!(decoded.queue_wait_time_ms, Some(200.0));
        assert_eq!(decoded.total_user_latency_ms, Some(280.0));
    }
}
