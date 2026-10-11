//! Persistent render jobs shared by the desktop application and CLI.
//!
//! The queue owns scheduling and job state, while each worker delegates the
//! actual export to the existing `RenderConfig` renderer entry point.

use crate::{
    export_control::ExportControl,
    export_progress::{ExportPhase, ExportProgress},
    render::{RenderBackend, RenderConfig},
    sono_gcc::WatchExecutionMode,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const STORE_VERSION: u32 = 1;
const LOG_LIMIT: usize = 300;
const TERMINAL_QUEUE_LIMIT: usize = 100;
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(2);

pub use crate::job_config::{JobManagerConfig, JobPriority};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Compiling,
    Rendering,
    Encoding,
    Completed,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Compiling | Self::Rendering | Self::Encoding
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureKind {
    Transient,
    Configuration,
    Renderer,
    ExternalResource,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobLogEntry {
    pub unix_time_ms: u128,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderJobOutcome {
    pub submitted_frames: u64,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub rgb_sequence_sha1: String,
    pub sfx_event_sha1: String,
    #[serde(default)]
    pub sfx_pcm_sha1: Option<String>,
    pub watch_execution_status: String,
    pub ffmpeg_source: String,
    #[serde(default)]
    pub performance_record_id: Option<String>,
}

impl RenderJobOutcome {
    fn summary(&self, output: &Path) -> String {
        format!(
            "{} frames rendered over {:.3}s; RGB SHA-1 {}; SFX SHA-1 {}; output published to {}",
            self.submitted_frames,
            self.timeline_end - self.timeline_start,
            self.rgb_sequence_sha1,
            self.sfx_event_sha1,
            output.display()
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderJob {
    pub id: String,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_name: Option<String>,
    #[serde(default)]
    pub project_path: Option<PathBuf>,
    #[serde(default)]
    pub original_job_id: Option<String>,
    #[serde(default)]
    pub retried_from_history_id: Option<String>,
    #[serde(default)]
    pub rerendered_from_history_id: Option<String>,
    #[serde(default)]
    pub reproduced_from_manifest_sha256: Option<String>,
    pub state: JobState,
    pub priority: JobPriority,
    pub queue_order: u64,
    #[serde(default)]
    pub updated_unix_time_ns: u128,
    pub submitted_unix_time_ms: u128,
    pub started_unix_time_ms: Option<u128>,
    #[serde(default)]
    pub worker_process_id: Option<u32>,
    pub finished_unix_time_ms: Option<u128>,
    pub attempt: u32,
    pub previous_attempt_diagnostics: Vec<String>,
    pub renderer_version: String,
    pub config: RenderConfig,
    pub execution_backend: String,
    pub progress: Option<ExportProgress>,
    #[serde(default)]
    pub outcome: Option<RenderJobOutcome>,
    #[serde(default)]
    pub diagnostic_report: Option<crate::render_diagnostics::RenderDiagnosticRecord>,
    pub logs: Vec<JobLogEntry>,
    pub diagnostic: Option<String>,
    pub failure_kind: Option<JobFailureKind>,
    pub cancellation_requested: bool,
    #[serde(default)]
    pub staging_path_owned: bool,
    /// Queue acknowledgment that this terminal attempt has a durable history record.
    #[serde(default)]
    pub history_persisted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RenderJobMetadata {
    pub project_path: Option<PathBuf>,
    pub original_job_id: Option<String>,
    pub retried_from_history_id: Option<String>,
    pub rerendered_from_history_id: Option<String>,
    pub reproduced_from_manifest_sha256: Option<String>,
}

impl RenderJob {
    pub fn output_path(&self) -> Option<&Path> {
        self.config.output.as_deref()
    }
}

impl From<JobState> for crate::render_history::HistoryStatus {
    fn from(state: JobState) -> Self {
        match state {
            JobState::Completed => Self::Completed,
            JobState::Cancelled => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

// Queue-owned fields are projected into the durable history schema here. The
// history store owns validation and persistence, but stays independent of the
// live queue model and its lifecycle.
impl crate::render_history::RenderHistoryRecord {
    pub fn from_job(job: &RenderJob) -> Self {
        use crate::render_history::{
            history_id, input_identities, sanitize_config_snapshot, HistoryOutputMetadata,
            HISTORY_FORMAT, HISTORY_SCHEMA_VERSION,
        };
        use std::collections::BTreeMap;

        let id = history_id(&job.id, job.attempt);
        let config_snapshot = sanitize_config_snapshot(&job.config);
        let input_identities = input_identities(&config_snapshot);
        let output_path = config_snapshot.output.clone();
        let output_meta = output_path
            .as_deref()
            .and_then(|path| fs::metadata(path).ok());
        let report = job.diagnostic_report.as_ref();
        let performance_record_id = job
            .outcome
            .as_ref()
            .and_then(|outcome| outcome.performance_record_id.clone())
            .or_else(|| report.and_then(|report| report.performance_record_id.clone()));
        let mut warnings: Vec<String> = report
            .map(|report| {
                report
                    .findings
                    .iter()
                    .filter(|finding| {
                        finding.kind == crate::render_diagnostics::FindingKind::Recommendation
                            || finding.code.contains("warning")
                            || finding.code.contains("risk")
                    })
                    .map(|finding| finding.summary.clone())
                    .collect()
            })
            .unwrap_or_default();
        for log in &job.logs {
            let lower = log.message.to_ascii_lowercase();
            if lower.contains("warning") || lower.contains("fallback") {
                warnings.push(log.message.clone());
            }
        }
        warnings.sort();
        warnings.dedup();
        let fallback = report.and_then(|report| {
            report
                .findings
                .iter()
                .find(|finding| finding.code == "backend_fallback")
                .map(|finding| finding.summary.clone())
        });
        Self {
            format: HISTORY_FORMAT.into(),
            schema_version: HISTORY_SCHEMA_VERSION,
            id,
            job_id: job.id.clone(),
            attempt: job.attempt,
            project_path: job.project_path.clone(),
            profile_id: job.profile_id.clone(),
            profile_name: job.profile_name.clone(),
            original_job_id: job.original_job_id.clone().or_else(|| Some(job.id.clone())),
            retried_from_history_id: job.retried_from_history_id.clone(),
            rerendered_from_history_id: job.rerendered_from_history_id.clone(),
            reproduced_from_manifest_sha256: job.reproduced_from_manifest_sha256.clone(),
            renderer_version: job.renderer_version.clone(),
            operating_system: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            execution_backend: job.execution_backend.clone(),
            status: job.state.into(),
            started_unix_time_ms: job.started_unix_time_ms,
            completed_unix_time_ms: job
                .finished_unix_time_ms
                .unwrap_or_else(crate::render_history::unix_time_ms),
            config_snapshot,
            input_identities,
            output: HistoryOutputMetadata {
                path: output_path,
                exists: output_meta.is_some(),
                size_bytes: output_meta.map(|metadata| metadata.len()),
            },
            rgb_sequence_sha1: job
                .outcome
                .as_ref()
                .map(|outcome| outcome.rgb_sequence_sha1.clone()),
            sfx_event_sha1: job
                .outcome
                .as_ref()
                .map(|outcome| outcome.sfx_event_sha1.clone()),
            sfx_pcm_sha1: job
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.sfx_pcm_sha1.clone()),
            performance_record_id,
            diagnostic_record_id: report.map(|report| report.id.clone()),
            effective_backend_status: report.map(|report| report.effective_backend.clone()),
            cache_state: report.and_then(|report| report.cache_state.clone()),
            fallback,
            warnings,
            error: job.diagnostic.clone(),
            pinned: false,
            extra: BTreeMap::new(),
        }
    }
}

impl crate::render_history::RenderHistoryStore {
    pub fn save_attempt(
        &self,
        job: &RenderJob,
    ) -> Result<crate::render_history::RenderHistoryRecord> {
        if !job.state.is_terminal() {
            bail!("only terminal render attempts can be added to history");
        }
        let record = crate::render_history::RenderHistoryRecord::from_job(job);
        self.save_immutable(&record)?;
        self.prune()?;
        Ok(record)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct PersistedQueue {
    version: u32,
    next_order: u64,
    paused: bool,
    updated_unix_time_ns: u128,
    scheduler_configured: bool,
    max_concurrent_jobs: usize,
    default_priority: JobPriority,
    removed_job_ids: Vec<String>,
    jobs: Vec<RenderJob>,
}

impl Default for PersistedQueue {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            next_order: 1,
            paused: false,
            updated_unix_time_ns: 0,
            scheduler_configured: false,
            max_concurrent_jobs: 1,
            default_priority: JobPriority::Normal,
            removed_job_ids: Vec::new(),
            jobs: Vec::new(),
        }
    }
}

struct Shared {
    state: Mutex<PersistedQueue>,
    changed: Condvar,
    controls: Mutex<HashMap<String, ExportControl>>,
    max_concurrent: AtomicUsize,
    stopping: AtomicBool,
    started: AtomicBool,
    store_path: PathBuf,
    last_progress_persist: Mutex<HashMap<String, Instant>>,
    progress_notification_state: Mutex<HashMap<String, (u8, Duration)>>,
    queued_monotonic: Mutex<HashMap<String, Instant>>,
    history_path: PathBuf,
}

/// A persistent manager. Open the same store path in the GUI and CLI to share
/// the queue across application restarts.
pub struct RenderJobManager {
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    config: Mutex<JobManagerConfig>,
}

impl RenderJobManager {
    pub fn open(config: JobManagerConfig) -> Result<Self> {
        Self::open_at(default_store_path(), config)
    }

    pub fn open_at(path: impl Into<PathBuf>, config: JobManagerConfig) -> Result<Self> {
        let store_path = path.into();
        let history_path = history_path_for_queue(&store_path);
        Self::open_at_with_history(store_path, history_path, config)
    }

    pub fn open_at_with_history(
        path: impl Into<PathBuf>,
        history_path: impl Into<PathBuf>,
        config: JobManagerConfig,
    ) -> Result<Self> {
        config.validate()?;
        let store_path = path.into();
        let history_path = history_path.into();
        let mut state = load_queue(&store_path)?;
        if state.version != STORE_VERSION {
            bail!(
                "unsupported render-job store version {} at {}",
                state.version,
                store_path.display()
            );
        }
        let config = if state.scheduler_configured {
            JobManagerConfig {
                max_concurrent_jobs: state.max_concurrent_jobs,
                default_priority: state.default_priority,
            }
        } else {
            config
        };
        config.validate()?;
        let now = unix_time_ms();
        let mut recovered = false;
        for job in &mut state.jobs {
            if job.state.is_active() && !job.worker_process_id.is_some_and(process_is_running) {
                let diagnostic = "Interrupted: the previous process stopped before render completion was confirmed.";
                job.state = JobState::Failed;
                job.finished_unix_time_ms = Some(now);
                job.worker_process_id = None;
                job.failure_kind = Some(JobFailureKind::Interrupted);
                job.diagnostic = Some(diagnostic.into());
                job.diagnostic_report = Some(
                    crate::render_diagnostics::RenderDiagnosticRecord::from_failure(
                        format!("diagnostic-{}-interrupted", job.id),
                        Some(job.id.clone()),
                        &job.config,
                        job.progress
                            .as_ref()
                            .map(|progress| format!("{:?}", progress.phase)),
                        Some("interrupted".into()),
                        &[diagnostic.into()],
                        crate::render_diagnostics::DiagnosticOutcome::Interrupted,
                    ),
                );
                job.cancellation_requested = false;
                push_log(job, diagnostic);
                recovered = true;
                release_claim(&store_path, &job.id);
            } else if job.state == JobState::Queued {
                release_stale_claim(&store_path, &job.id);
            } else if job.state.is_terminal() {
                release_claim(&store_path, &job.id);
            }
        }
        let manager = Self {
            shared: Arc::new(Shared {
                state: Mutex::new(state),
                changed: Condvar::new(),
                controls: Mutex::new(HashMap::new()),
                max_concurrent: AtomicUsize::new(config.max_concurrent_jobs),
                stopping: AtomicBool::new(false),
                started: AtomicBool::new(false),
                store_path,
                last_progress_persist: Mutex::new(HashMap::new()),
                progress_notification_state: Mutex::new(HashMap::new()),
                queued_monotonic: Mutex::new(HashMap::new()),
                history_path,
            }),
            workers: Mutex::new(Vec::new()),
            config: Mutex::new(config),
        };
        if recovered {
            manager.persist()?;
        }
        // Recover entries if a process stopped after committing terminal queue
        // state but before writing the independent history sidecar.
        let history_store = manager.history_store().ok();
        if let Ok(jobs) = manager.jobs() {
            for job in jobs.into_iter().filter(|job| job.state.is_terminal()) {
                if history_store.is_some() {
                    if let Err(error) = persist_terminal_history(&manager.shared, &job) {
                        record_log(
                            &manager.shared,
                            &job.id,
                            format!("History entry was not saved: {error:#}"),
                        );
                    }
                }
            }
        }
        if history_store.is_some() {
            prune_terminal_queue(&manager.shared);
        }
        Ok(manager)
    }

    pub fn history_store(&self) -> Result<crate::render_history::RenderHistoryStore> {
        crate::render_history::RenderHistoryStore::open_at(&self.shared.history_path)
    }

    pub fn store_path(&self) -> &Path {
        &self.shared.store_path
    }

    pub fn config(&self) -> JobManagerConfig {
        self.config
            .lock()
            .expect("job config lock poisoned")
            .clone()
    }

    /// Updates scheduler settings and grows the worker pool when requested.
    /// Lowering the limit takes effect as soon as existing jobs finish.
    pub fn configure(&self, config: JobManagerConfig) -> Result<()> {
        config.validate()?;
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        state.scheduler_configured = true;
        state.max_concurrent_jobs = config.max_concurrent_jobs;
        state.default_priority = config.default_priority;
        state.updated_unix_time_ns = unix_time_ns();
        save_queue(&self.shared.store_path, &state)?;
        drop(state);
        self.shared
            .max_concurrent
            .store(config.max_concurrent_jobs, Ordering::Release);
        *self
            .config
            .lock()
            .map_err(|_| anyhow::anyhow!("job config lock poisoned"))? = config.clone();
        if self.shared.started.load(Ordering::Acquire) {
            self.ensure_workers(config.max_concurrent_jobs)?;
        }
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Starts workers once. A GUI starts on launch to resume persisted queued
    /// jobs; CLI list/inspect commands can open the store without executing it.
    pub fn start(&self) -> Result<()> {
        self.shared.started.store(true, Ordering::Release);
        self.ensure_workers(self.shared.max_concurrent.load(Ordering::Acquire))?;
        self.shared.changed.notify_all();
        Ok(())
    }

    fn ensure_workers(&self, count: usize) -> Result<()> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| anyhow::anyhow!("job worker list lock poisoned"))?;
        while workers.len() < count {
            let shared = self.shared.clone();
            let worker_number = workers.len() + 1;
            let worker = thread::Builder::new()
                .name(format!("render-job-worker-{worker_number}"))
                .spawn(move || worker_loop(shared))
                .context("starting render-job worker")?;
            workers.push(worker);
        }
        Ok(())
    }

    pub fn enqueue(&self, config: RenderConfig, priority: Option<JobPriority>) -> Result<String> {
        self.enqueue_internal(config, priority, None, RenderJobMetadata::default())
    }

    /// Enqueue a resolved profile config and retain its source identity.
    /// Rendering still consumes only the immutable config snapshot.
    pub fn enqueue_from_profile(
        &self,
        config: RenderConfig,
        priority: Option<JobPriority>,
        profile_id: String,
        profile_name: String,
    ) -> Result<String> {
        self.enqueue_internal(
            config,
            priority,
            Some((profile_id, profile_name)),
            RenderJobMetadata::default(),
        )
    }

    /// Submit a render with provenance used by persistent history.
    pub fn enqueue_with_metadata(
        &self,
        config: RenderConfig,
        priority: Option<JobPriority>,
        profile: Option<(String, String)>,
        metadata: RenderJobMetadata,
    ) -> Result<String> {
        self.enqueue_internal(config, priority, profile, metadata)
    }

    fn enqueue_internal(
        &self,
        config: RenderConfig,
        priority: Option<JobPriority>,
        profile: Option<(String, String)>,
        metadata: RenderJobMetadata,
    ) -> Result<String> {
        self.configure(config.job_manager.clone())?;
        config.validate_export_inputs()?;
        let output = config
            .output
            .as_deref()
            .context("render job configuration must include an output path")?;
        ensure_output_parent(output)?;
        if output.exists() {
            bail!(
                "refusing to replace existing output file: {}",
                output.display()
            );
        }
        ensure_output_not_input(output, &config)?;
        let collision_key = path_collision_key(output)?;
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        if state.jobs.iter().any(|job| {
            !job.state.is_terminal()
                && job
                    .output_path()
                    .and_then(|path| path_collision_key(path).ok())
                    .is_some_and(|key| key == collision_key)
        }) {
            bail!(
                "an active or queued render job already reserves {}",
                output.display()
            );
        }
        let id = unique_job_id(&state.jobs);
        let order = state.next_order;
        state.next_order = state.next_order.saturating_add(1);
        let priority = priority.unwrap_or_else(|| self.config().default_priority);
        let backend = execution_backend(&config);
        let mut job = RenderJob {
            id: id.clone(),
            profile_id: profile.as_ref().map(|(id, _)| id.clone()),
            profile_name: profile.map(|(_, name)| name),
            project_path: metadata.project_path,
            original_job_id: metadata.original_job_id.or_else(|| Some(id.clone())),
            retried_from_history_id: metadata.retried_from_history_id,
            rerendered_from_history_id: metadata.rerendered_from_history_id,
            reproduced_from_manifest_sha256: metadata.reproduced_from_manifest_sha256,
            state: JobState::Queued,
            priority,
            queue_order: order,
            updated_unix_time_ns: unix_time_ns(),
            submitted_unix_time_ms: unix_time_ms(),
            started_unix_time_ms: None,
            worker_process_id: None,
            finished_unix_time_ms: None,
            attempt: 0,
            previous_attempt_diagnostics: Vec::new(),
            renderer_version: env!("CARGO_PKG_VERSION").into(),
            config,
            execution_backend: backend,
            progress: None,
            outcome: None,
            diagnostic_report: None,
            logs: Vec::new(),
            diagnostic: None,
            failure_kind: None,
            cancellation_requested: false,
            staging_path_owned: false,
            history_persisted: false,
        };
        push_log(&mut job, "Job queued.");
        state.jobs.push(job.clone());
        if let Err(error) = save_queue(&self.shared.store_path, &state) {
            state.jobs.retain(|queued| queued.id != id);
            state.next_order = order;
            return Err(error);
        }
        if let Ok(mut queued) = self.shared.queued_monotonic.lock() {
            queued.insert(id.clone(), Instant::now());
        }
        drop(state);
        emit_job_event(
            &job,
            crate::notifications::RendererEventKind::JobQueued,
            None,
        );
        self.shared.changed.notify_all();
        Ok(id)
    }

    pub fn jobs(&self) -> Result<Vec<RenderJob>> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let mut jobs = state.jobs.clone();
        jobs.sort_by(|a, b| match (a.state.is_active(), b.state.is_active()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.queue_order.cmp(&b.queue_order),
        });
        Ok(jobs)
    }

    pub fn inspect(&self, id: &str) -> Result<RenderJob> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        state
            .jobs
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .with_context(|| format!("render job {id:?} was not found"))
    }

    pub fn pause(&self) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        state.paused = true;
        state.updated_unix_time_ns = unix_time_ns();
        save_queue(&self.shared.store_path, &state)?;
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        state.paused = false;
        state.updated_unix_time_ns = unix_time_ns();
        save_queue(&self.shared.store_path, &state)?;
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn is_paused(&self) -> Result<bool> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        Ok(state.paused)
    }

    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let job = find_job_mut(&mut state, id)?;
        if job.state == JobState::Queued {
            job.state = JobState::Cancelled;
            job.finished_unix_time_ms = Some(unix_time_ms());
            job.cancellation_requested = true;
            push_log(job, "Queued job cancelled before execution.");
            let cancelled = job.clone();
            save_queue(&self.shared.store_path, &state)?;
            drop(state);
            record_terminal_history(&self.shared, &cancelled);
            emit_job_event(
                &cancelled,
                crate::notifications::RendererEventKind::JobCancelled,
                None,
            );
            return Ok(());
        }
        if !job.state.is_active() {
            bail!("job {id} is already terminal ({:?})", job.state);
        }
        job.cancellation_requested = true;
        push_log(
            job,
            "Cancellation requested; finishing the smallest safe renderer unit.",
        );
        save_queue(&self.shared.store_path, &state)?;
        drop(state);
        if let Some(control) = self
            .shared
            .controls
            .lock()
            .map_err(|_| anyhow::anyhow!("job control lock poisoned"))?
            .get(id)
        {
            control.cancel();
        }
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn retry(&self, id: &str) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let order = state.next_order;
        state.next_order = state.next_order.saturating_add(1);
        let job = find_job_mut(&mut state, id)?;
        if job.state != JobState::Failed {
            bail!(
                "only failed jobs can be retried (job {id} is {:?})",
                job.state
            );
        }
        if job.output_path().is_some_and(Path::exists) {
            bail!("retry output already exists; move or remove it before retrying job {id}");
        }
        let previous_history_id = crate::render_history::history_id(&job.id, job.attempt);
        // Preserve the failed attempt before changing this queue row back to a
        // pending state. History persistence is idempotent by attempt ID.
        self.history_store()?.save_attempt(job)?;
        if let Some(diagnostic) = job.diagnostic.take() {
            job.previous_attempt_diagnostics.push(diagnostic);
        }
        job.state = JobState::Queued;
        job.queue_order = order;
        job.started_unix_time_ms = None;
        job.finished_unix_time_ms = None;
        job.progress = None;
        job.outcome = None;
        job.diagnostic_report = None;
        job.failure_kind = None;
        job.cancellation_requested = false;
        job.retried_from_history_id = Some(previous_history_id);
        job.history_persisted = false;
        push_log(job, "Retry queued; previous attempt diagnostics retained.");
        let queued = job.clone();
        save_queue(&self.shared.store_path, &state)?;
        if let Ok(mut monotonic) = self.shared.queued_monotonic.lock() {
            monotonic.insert(id.to_owned(), Instant::now());
        }
        drop(state);
        emit_job_event(
            &queued,
            crate::notifications::RendererEventKind::JobQueued,
            None,
        );
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let Some(index) = state.jobs.iter().position(|job| job.id == id) else {
            bail!("render job {id:?} was not found");
        };
        if !state.jobs[index].state.is_terminal() {
            bail!("cannot remove non-terminal job {id}");
        }
        state.jobs.remove(index);
        state.removed_job_ids.push(id.to_owned());
        save_queue(&self.shared.store_path, &state)
    }

    pub fn clear_terminal(&self) -> Result<usize> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let before = state.jobs.len();
        let removed_ids = state
            .jobs
            .iter()
            .filter(|job| job.state.is_terminal())
            .map(|job| job.id.clone())
            .collect::<Vec<_>>();
        state.removed_job_ids.extend(removed_ids);
        state.jobs.retain(|job| !job.state.is_terminal());
        let removed = before - state.jobs.len();
        save_queue(&self.shared.store_path, &state)?;
        Ok(removed)
    }

    pub fn cancel_queued(&self) -> Result<usize> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let mut cancelled_jobs = Vec::new();
        for job in &mut state.jobs {
            if job.state == JobState::Queued {
                job.state = JobState::Cancelled;
                job.finished_unix_time_ms = Some(unix_time_ms());
                job.cancellation_requested = true;
                push_log(job, "Queued job cancelled before execution.");
                cancelled_jobs.push(job.clone());
            }
        }
        let count = cancelled_jobs.len();
        save_queue(&self.shared.store_path, &state)?;
        drop(state);
        for job in cancelled_jobs {
            if let Err(error) = persist_terminal_history(&self.shared, &job) {
                record_log(
                    &self.shared,
                    &job.id,
                    format!("History entry was not saved: {error:#}"),
                );
            }
            emit_job_event(
                &job,
                crate::notifications::RendererEventKind::JobCancelled,
                None,
            );
        }
        prune_terminal_queue(&self.shared);
        self.shared.changed.notify_all();
        Ok(count)
    }

    pub fn reorder(&self, id: &str, position: usize) -> Result<()> {
        let mut state = self.lock_state()?;
        refresh_queue_locked(&self.shared.store_path, &mut state)?;
        let mut queued: Vec<_> = state
            .jobs
            .iter()
            .filter(|job| job.state == JobState::Queued)
            .map(|job| (job.queue_order, job.id.clone()))
            .collect();
        let Some(from) = queued.iter().position(|(_, job_id)| job_id == id) else {
            bail!("job {id} is not queued");
        };
        let item = queued.remove(from);
        queued.insert(position.min(queued.len()), item);
        for (rank, (_, job_id)) in queued.iter().enumerate() {
            if let Some(job) = state.jobs.iter_mut().find(|job| &job.id == job_id) {
                job.queue_order = rank as u64 + 1;
                touch(job);
            }
        }
        state.next_order = queued.len() as u64 + 1;
        save_queue(&self.shared.store_path, &state)?;
        self.shared.changed.notify_all();
        Ok(())
    }

    /// Wait for every queued and active job to finish. Failed jobs do not stop
    /// unrelated work; callers inspect each result after the queue drains.
    pub fn wait_until_idle(&self) -> Result<()> {
        loop {
            let mut state = self.lock_state()?;
            refresh_queue_locked(&self.shared.store_path, &mut state)?;
            if state.paused && state.jobs.iter().any(|job| job.state == JobState::Queued) {
                bail!("render queue is paused with pending jobs; resume it before running");
            }
            if !state
                .jobs
                .iter()
                .any(|job| job.state.is_active() || job.state == JobState::Queued)
            {
                return Ok(());
            }
            drop(
                self.shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(200))
                    .map_err(|_| anyhow::anyhow!("render queue lock poisoned"))?,
            );
        }
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, PersistedQueue>> {
        self.shared
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("render queue state lock poisoned"))
    }

    fn persist(&self) -> Result<()> {
        let state = self.lock_state()?;
        save_queue(&self.shared.store_path, &state)
    }
}

impl Drop for RenderJobManager {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::Release);
        if let Ok(controls) = self.shared.controls.lock() {
            for control in controls.values() {
                control.cancel();
            }
        }
        self.shared.changed.notify_all();
        if let Ok(workers) = self.workers.get_mut() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        }
    }
}

fn worker_loop(shared: Arc<Shared>) {
    loop {
        let claimed = {
            let mut state = match shared.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            loop {
                if shared.stopping.load(Ordering::Acquire) {
                    return;
                }
                if refresh_queue_locked(&shared.store_path, &mut state).is_err() {
                    let (next, _) = match shared
                        .changed
                        .wait_timeout(state, Duration::from_millis(250))
                    {
                        Ok(result) => result,
                        Err(_) => return,
                    };
                    state = next;
                    continue;
                }
                shared
                    .max_concurrent
                    .store(state.max_concurrent_jobs, Ordering::Release);
                let active = state
                    .jobs
                    .iter()
                    .filter(|job| job.state.is_active())
                    .count();
                if !state.paused && active < shared.max_concurrent.load(Ordering::Acquire) {
                    if let Some(index) = next_claimable_job(&state, &shared.store_path) {
                        let job_id = state.jobs[index].id.clone();
                        if !acquire_claim(&shared.store_path, &job_id) {
                            let (next, _) = match shared
                                .changed
                                .wait_timeout(state, Duration::from_millis(250))
                            {
                                Ok(result) => result,
                                Err(_) => return,
                            };
                            state = next;
                            continue;
                        }
                        let claimed = {
                            let job = &mut state.jobs[index];
                            job.state = JobState::Preparing;
                            job.attempt = job.attempt.saturating_add(1);
                            job.started_unix_time_ms = Some(unix_time_ms());
                            job.worker_process_id = Some(std::process::id());
                            job.finished_unix_time_ms = None;
                            job.cancellation_requested = false;
                            push_log(
                                job,
                                format!(
                                    "Attempt {} started; preparing renderer inputs.",
                                    job.attempt
                                ),
                            );
                            job.clone()
                        };
                        if save_queue(&shared.store_path, &state).is_err() {
                            release_claim(&shared.store_path, &job_id);
                            if let Some(job) = state.jobs.get_mut(index) {
                                job.state = JobState::Failed;
                                job.finished_unix_time_ms = Some(unix_time_ms());
                                job.failure_kind = Some(JobFailureKind::ExternalResource);
                                job.diagnostic =
                                    Some("could not persist claimed render job".into());
                            }
                            if let Some(job) = state.jobs.get_mut(index) {
                                push_log(job, "Job failed because the queue store could not save its claimed state.");
                                let _ = save_queue(&shared.store_path, &state);
                            }
                            let failed = state.jobs.get(index).cloned();
                            drop(state);
                            if let Some(failed) = failed {
                                record_terminal_history(&shared, &failed);
                            }
                            shared.changed.notify_all();
                            break None;
                        }
                        break Some(claimed);
                    }
                }
                let (next, _) = match shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(250))
                {
                    Ok(result) => result,
                    Err(_) => return,
                };
                state = next;
            }
        };
        let Some(job) = claimed else { continue };
        emit_job_event(
            &job,
            crate::notifications::RendererEventKind::JobStarted,
            None,
        );
        emit_job_event(
            &job,
            crate::notifications::RendererEventKind::RenderStarted,
            None,
        );
        execute_job(shared.clone(), job);
        shared.changed.notify_all();
    }
}

#[cfg(test)]
fn next_queued_job(state: &PersistedQueue) -> Option<usize> {
    next_job_index(state, |_| true)
}

fn next_claimable_job(state: &PersistedQueue, store: &Path) -> Option<usize> {
    next_job_index(state, |job| {
        let path = claim_path(store, &job.id);
        if !path.exists() {
            return true;
        }
        release_stale_claim(store, &job.id);
        !path.exists()
    })
}

fn next_job_index(state: &PersistedQueue, eligible: impl Fn(&RenderJob) -> bool) -> Option<usize> {
    let now = unix_time_ms();
    state
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.state == JobState::Queued && eligible(job))
        .max_by_key(|(_, job)| {
            let age_seconds = now
                .saturating_sub(job.submitted_unix_time_ms)
                .saturating_div(1_000) as i64;
            (
                job.priority.age_bias_seconds() + age_seconds,
                std::cmp::Reverse(job.queue_order),
            )
        })
        .map(|(index, _)| index)
}

fn execute_job(shared: Arc<Shared>, job: RenderJob) {
    let queued_at = shared
        .queued_monotonic
        .lock()
        .ok()
        .and_then(|mut queued| queued.remove(&job.id));
    let job_execution_started = Instant::now();
    let queue_wait = queued_at.map(|queued| job_execution_started.duration_since(queued));
    let output = job.config.output.as_deref().expect("validated job output");
    let staged_output = staged_output_path(output, &job.id);
    if let Err(error) = job.config.validate_export_inputs() {
        finish_job(&shared, &job.id, Err(error), None);
        release_claim(&shared.store_path, &job.id);
        return;
    }
    if let Err(error) = prepare_stage(&staged_output, job.staging_path_owned) {
        finish_job(&shared, &job.id, Err(error), None);
        release_claim(&shared.store_path, &job.id);
        return;
    }
    if let Err(error) = set_stage_owned(&shared, &job.id, true) {
        let _ = fs::remove_file(&staged_output);
        finish_job(&shared, &job.id, Err(error), None);
        release_claim(&shared.store_path, &job.id);
        return;
    }
    let mut config = job.config.clone();
    config.output = Some(staged_output.clone());
    // The manager publishes job lifecycle events with the final output path;
    // suppress duplicate render-only notifications from the nested renderer.
    config.notifications.enabled = false;
    let log_shared = shared.clone();
    let log_id = job.id.clone();
    let control = ExportControl::with_log(move |message| record_log(&log_shared, &log_id, message))
        .with_phase_sink({
            let phase_shared = shared.clone();
            let phase_id = job.id.clone();
            move |phase| transition_phase(&phase_shared, &phase_id, phase)
        });
    if let Ok(mut controls) = shared.controls.lock() {
        controls.insert(job.id.clone(), control.clone());
    }
    let cancellation_was_requested = shared
        .state
        .lock()
        .ok()
        .and_then(|state| {
            state
                .jobs
                .iter()
                .find(|candidate| candidate.id == job.id)
                .map(|candidate| candidate.cancellation_requested)
        })
        .unwrap_or(false);
    if cancellation_was_requested {
        control.cancel();
    }
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor_shared = shared.clone();
    let monitor_id = job.id.clone();
    let monitor_control = control.clone();
    let monitor_flag = monitor_stop.clone();
    let cancel_monitor =
        thread::spawn(move || {
            while !monitor_flag.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(100));
                if monitor_flag.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(mut state) = monitor_shared.state.lock() {
                    let _ = refresh_queue_locked(&monitor_shared.store_path, &mut state);
                    if state.jobs.iter().any(|candidate| {
                        candidate.id == monitor_id && candidate.cancellation_requested
                    }) {
                        monitor_control.cancel();
                        break;
                    }
                }
            }
        });
    if job.config.execution_mode == WatchExecutionMode::SonoGcc
        && job.config.compile_sono_gcc_on_demand
    {
        transition_phase(&shared, &job.id, ExportPhase::Preparing);
    }
    let progress_shared = shared.clone();
    let progress_id = job.id.clone();
    let mut preserve_stage_for_recovery = false;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        config.render_video_controlled_unrecorded(
            Box::new(move |event| record_progress(&progress_shared, &progress_id, event)),
            control.clone(),
        )
    }))
    .map_err(|payload| {
        anyhow::anyhow!("renderer worker panicked: {}", panic_text(payload.as_ref()))
    })
    .and_then(|result| result)
    .and_then(|report| {
        control.check()?;
        if let Err(error) = publish_output(&staged_output, output) {
            preserve_stage_for_recovery = true;
            return Err(error);
        }
        let mut performance = report.performance;
        performance.job_id = Some(job.id.clone());
        if let Some(started) = job.started_unix_time_ms {
            performance.started_unix_time_ms = started;
        }
        performance.queue_wait_time_ms = queue_wait.map(|wait| wait.as_secs_f64() * 1000.0);
        performance.execution_time_ms = job_execution_started.elapsed().as_secs_f64() * 1000.0;
        performance.total_user_latency_ms = performance
            .queue_wait_time_ms
            .map(|queue_ms| queue_ms + performance.execution_time_ms);
        performance.completed_unix_time_ms = unix_time_ms();
        let phase_sum = performance
            .stage_timings
            .iter()
            .filter(|stage| {
                stage.semantics == crate::performance::StageSemantics::ExclusiveWallSpan
            })
            .map(|stage| stage.duration_ms)
            .sum::<f64>();
        let residual_ms = performance.execution_time_ms - phase_sum;
        if residual_ms > 0.0 {
            performance.stage_timings.push(crate::performance::stage(
                "Queue validation, staging, publication, and job finalization",
                Duration::from_secs_f64(residual_ms / 1000.0),
                1,
            ));
        }
        let diagnostic_report = crate::render_diagnostics::RenderDiagnosticRecord::from_export(
            &performance,
            &report.ffmpeg_source,
            &report.probe,
            report.deterministic_frame_hashes_match,
        );
        let performance_record_id = match crate::performance::PerformanceStore::open_default()
            .and_then(|store| store.save(&performance))
        {
            Ok(()) => Some(performance.id.clone()),
            Err(error) => {
                record_log(
                    &shared,
                    &job.id,
                    format!("Performance record was not saved: {error:#}"),
                );
                None
            }
        };
        if let Err(error) = crate::render_diagnostics::RenderDiagnosticStore::open_default()
            .and_then(|store| store.save(&diagnostic_report))
        {
            record_log(
                &shared,
                &job.id,
                format!("Render diagnostic was not saved separately: {error:#}"),
            );
        }
        Ok((
            RenderJobOutcome {
                submitted_frames: report.submitted_frames,
                timeline_start: report.timeline_start,
                timeline_end: report.timeline_end,
                rgb_sequence_sha1: report.second_pass_hash,
                sfx_event_sha1: report.sfx_event_sha1,
                sfx_pcm_sha1: report.sfx_pcm_sha1,
                watch_execution_status: report.watch_execution_status,
                ffmpeg_source: report.ffmpeg_source,
                performance_record_id,
            },
            diagnostic_report,
        ))
    });
    monitor_stop.store(true, Ordering::Release);
    let _ = cancel_monitor.join();
    if control.is_cancelled() || (result.is_err() && !preserve_stage_for_recovery) {
        let _ = fs::remove_file(&staged_output);
        let _ = set_stage_owned(&shared, &job.id, false);
    } else if result.is_ok() {
        let _ = set_stage_owned(&shared, &job.id, false);
    }
    if let Ok(mut controls) = shared.controls.lock() {
        controls.remove(&job.id);
    }
    finish_job(&shared, &job.id, result, None);
    release_claim(&shared.store_path, &job.id);
}

fn finish_job(
    shared: &Shared,
    id: &str,
    result: Result<(
        RenderJobOutcome,
        crate::render_diagnostics::RenderDiagnosticRecord,
    )>,
    _original: Option<&RenderJob>,
) {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    let _ = refresh_queue_locked(&shared.store_path, &mut state);
    let cancelled = state
        .jobs
        .iter()
        .find(|job| job.id == id)
        .is_some_and(|job| job.cancellation_requested);
    let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) else {
        return;
    };
    job.finished_unix_time_ms = Some(unix_time_ms());
    job.worker_process_id = None;
    match result {
        Ok((outcome, diagnostic_report)) if !cancelled => {
            job.state = JobState::Completed;
            job.failure_kind = None;
            job.diagnostic = None;
            job.diagnostic_report = Some(diagnostic_report);
            if let Some(output) = job.output_path().map(Path::to_path_buf) {
                push_log(job, outcome.summary(&output));
            }
            job.outcome = Some(outcome);
        }
        _ if cancelled => {
            job.state = JobState::Cancelled;
            job.diagnostic = None;
            job.failure_kind = None;
            job.outcome = None;
            let phase = job
                .progress
                .as_ref()
                .map(|progress| format!("{:?}", progress.phase));
            job.diagnostic_report = Some(
                crate::render_diagnostics::RenderDiagnosticRecord::from_failure(
                    format!("diagnostic-{id}-cancelled-attempt-{}", job.attempt),
                    Some(id.to_owned()),
                    &job.config,
                    phase,
                    Some("cancelled".into()),
                    &["Cancellation completed; no output was published.".into()],
                    crate::render_diagnostics::DiagnosticOutcome::Cancelled,
                ),
            );
            push_log(job, "Cancellation completed; no output was published.");
        }
        Err(error) => {
            let diagnostic = format!("{error:#}");
            job.state = JobState::Failed;
            let failure_kind = classify_failure(&diagnostic);
            let phase = job
                .progress
                .as_ref()
                .map(|progress| format!("{:?}", progress.phase));
            let error_chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
            job.failure_kind = Some(failure_kind);
            job.diagnostic = Some(diagnostic.clone());
            job.diagnostic_report = Some(
                crate::render_diagnostics::RenderDiagnosticRecord::from_failure(
                    format!("diagnostic-{id}-attempt-{}", job.attempt),
                    Some(id.to_owned()),
                    &job.config,
                    phase,
                    Some(format!("{failure_kind:?}").to_ascii_lowercase()),
                    &error_chain,
                    crate::render_diagnostics::DiagnosticOutcome::Failed,
                ),
            );
            job.outcome = None;
            push_log(job, format!("Job failed: {diagnostic}"));
        }
        Ok(_) => {}
    }
    let completed = job.clone();
    let _ = save_queue(&shared.store_path, &state);
    drop(state);
    record_terminal_history(shared, &completed);
    if let Ok(mut times) = shared.last_progress_persist.lock() {
        times.remove(id);
    }
    if let Ok(mut times) = shared.progress_notification_state.lock() {
        times.remove(id);
    }
    match completed.state {
        JobState::Completed => {
            emit_job_event(
                &completed,
                crate::notifications::RendererEventKind::JobCompleted,
                None,
            );
            emit_job_event(
                &completed,
                crate::notifications::RendererEventKind::RenderCompleted,
                None,
            );
        }
        JobState::Failed => {
            emit_job_event(
                &completed,
                crate::notifications::RendererEventKind::JobFailed,
                completed.diagnostic.clone(),
            );
            emit_job_event(
                &completed,
                crate::notifications::RendererEventKind::RenderFailed,
                completed.diagnostic.clone(),
            );
        }
        JobState::Cancelled => {
            emit_job_event(
                &completed,
                crate::notifications::RendererEventKind::JobCancelled,
                None,
            );
        }
        _ => {}
    }
    shared.changed.notify_all();
}

fn transition_phase(shared: &Shared, id: &str, phase: ExportPhase) {
    let target = match phase {
        ExportPhase::Compiling => JobState::Compiling,
        ExportPhase::Rendering => JobState::Rendering,
        ExportPhase::AudioMixing | ExportPhase::AudioEncoding | ExportPhase::Finalizing => {
            JobState::Encoding
        }
        ExportPhase::Preparing | ExportPhase::Validating => JobState::Preparing,
        ExportPhase::Complete => JobState::Encoding,
    };
    let mut changed_job = None;
    if let Ok(mut state) = shared.state.lock() {
        let _ = refresh_queue_locked(&shared.store_path, &mut state);
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            if job.state != target && job.state.is_active() {
                job.state = target;
                push_log(job, format!("State: {target:?}."));
                changed_job = Some(job.clone());
                let _ = save_queue(&shared.store_path, &state);
            }
        }
    }
    if let Some(job) = changed_job {
        match target {
            JobState::Compiling => emit_job_event(
                &job,
                crate::notifications::RendererEventKind::JobCompiling,
                None,
            ),
            JobState::Rendering => emit_job_event(
                &job,
                crate::notifications::RendererEventKind::JobRendering,
                None,
            ),
            _ => {}
        }
    }
    shared.changed.notify_all();
}

fn record_progress(shared: &Shared, id: &str, progress: ExportProgress) {
    let phase = progress.phase;
    let mut snapshot = None;
    if let Ok(mut state) = shared.state.lock() {
        let _ = refresh_queue_locked(&shared.store_path, &mut state);
        let mut job_copy = None;
        let mut state_changed = false;
        let mut should_persist = false;
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            job.progress = Some(progress.clone());
            touch(job);
            let target = match phase {
                ExportPhase::Compiling => JobState::Compiling,
                ExportPhase::Rendering => JobState::Rendering,
                ExportPhase::AudioMixing | ExportPhase::AudioEncoding | ExportPhase::Finalizing => {
                    JobState::Encoding
                }
                ExportPhase::Preparing | ExportPhase::Validating => JobState::Preparing,
                ExportPhase::Complete => JobState::Encoding,
            };
            if job.state.is_active() && job.state != target {
                job.state = target;
                push_log(job, format!("State: {target:?}."));
                state_changed = true;
            }
            should_persist = shared
                .last_progress_persist
                .lock()
                .map(|mut times| {
                    let due = times
                        .get(id)
                        .is_none_or(|last| last.elapsed() >= PROGRESS_PERSIST_INTERVAL);
                    if due {
                        times.insert(id.to_owned(), Instant::now());
                    }
                    due
                })
                .unwrap_or(true);
            job_copy = Some(job.clone());
        }
        if should_persist || state_changed {
            let _ = save_queue(&shared.store_path, &state);
        }
        if state_changed {
            snapshot = job_copy.clone();
        }
        drop(state);
        if phase == ExportPhase::Rendering {
            if let (Some(job), Some(percent)) = (job_copy, progress.percentage) {
                if job
                    .config
                    .notifications
                    .routes_event(crate::notifications::RendererEventKind::RenderProgress)
                {
                    if let Ok(mut notification_state) = shared.progress_notification_state.lock() {
                        let elapsed = Duration::from_secs_f64(progress.elapsed_seconds.max(0.0));
                        let last = notification_state
                            .entry(id.to_owned())
                            .or_insert((0, Duration::ZERO));
                        let rounded = percent.round().clamp(0.0, 100.0) as u8;
                        if job
                            .config
                            .notifications
                            .progress_due(rounded, elapsed, last.0, last.1)
                        {
                            *last = (rounded, elapsed);
                            let event = job_event(
                                &job,
                                crate::notifications::RendererEventKind::RenderProgress,
                                None,
                            )
                            .with("progress", format!("{percent:.0}"))
                            .with("completed_frames", progress.completed_frames)
                            .with(
                                "frame_count",
                                progress
                                    .total_frames
                                    .map_or_else(String::new, |n| n.to_string()),
                            );
                            crate::notifications::emit(&job.config.notifications, event);
                        }
                    }
                }
            }
        }
    }
    if let Some(job) = snapshot {
        if job.state == JobState::Rendering {
            emit_job_event(
                &job,
                crate::notifications::RendererEventKind::JobRendering,
                None,
            );
        } else if job.state == JobState::Compiling {
            emit_job_event(
                &job,
                crate::notifications::RendererEventKind::JobCompiling,
                None,
            );
        }
    }
    shared.changed.notify_all();
}

fn record_log(shared: &Shared, id: &str, message: String) {
    if let Ok(mut state) = shared.state.lock() {
        let _ = refresh_queue_locked(&shared.store_path, &mut state);
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            push_log(job, message);
            let _ = save_queue(&shared.store_path, &state);
        }
    }
    shared.changed.notify_all();
}

fn record_terminal_history(shared: &Shared, job: &RenderJob) {
    match persist_terminal_history(shared, job) {
        Ok(()) => {
            prune_terminal_queue(shared);
        }
        Err(error) => record_log(
            shared,
            &job.id,
            format!("History entry was not saved: {error:#}"),
        ),
    }
}

fn persist_terminal_history(shared: &Shared, job: &RenderJob) -> Result<()> {
    crate::render_history::RenderHistoryStore::open_at(&shared.history_path)?.save_attempt(job)?;
    let mut state = shared
        .state
        .lock()
        .map_err(|_| anyhow::anyhow!("render queue state lock poisoned"))?;
    refresh_queue_locked(&shared.store_path, &mut state)?;
    if let Some(current) = state.jobs.iter_mut().find(|current| current.id == job.id) {
        if current.attempt == job.attempt && current.state.is_terminal() {
            current.history_persisted = true;
            touch(current);
            save_queue(&shared.store_path, &state)?;
        }
    }
    Ok(())
}

fn prune_terminal_queue(shared: &Shared) {
    if let Ok(mut state) = shared.state.lock() {
        if refresh_queue_locked(&shared.store_path, &mut state).is_err() {
            return;
        }
        let mut persisted = state
            .jobs
            .iter()
            .filter(|job| job.state.is_terminal() && job.history_persisted)
            .map(|job| {
                (
                    job.finished_unix_time_ms
                        .unwrap_or(job.submitted_unix_time_ms),
                    job.id.clone(),
                )
            })
            .collect::<Vec<_>>();
        persisted.sort_by(|left, right| right.cmp(left));
        let removed = persisted
            .iter()
            .skip(TERMINAL_QUEUE_LIMIT)
            .map(|(_, id)| id.clone())
            .collect::<Vec<_>>();
        if removed.is_empty() {
            return;
        }
        state.jobs.retain(|job| !removed.contains(&job.id));
        state.removed_job_ids.extend(removed);
        let _ = save_queue(&shared.store_path, &state);
    }
}

fn history_path_for_queue(queue_path: &Path) -> PathBuf {
    crate::store_paths::history_store_path_for_queue(queue_path)
}

fn emit_job_event(
    job: &RenderJob,
    kind: crate::notifications::RendererEventKind,
    error: Option<String>,
) {
    crate::notifications::emit(&job.config.notifications, job_event(job, kind, error));
}

fn job_event(
    job: &RenderJob,
    kind: crate::notifications::RendererEventKind,
    error: Option<String>,
) -> crate::notifications::RendererEvent {
    let Some(output) = job.output_path() else {
        return crate::notifications::RendererEvent::new(kind);
    };
    let mut event = crate::notifications::RendererEvent::new(kind)
        .with("job_id", &job.id)
        .with("job_state", format!("{:?}", job.state).to_lowercase())
        .with("attempt", job.attempt)
        .with("execution_backend", &job.execution_backend)
        .with("engine", file_label(&job.config.engine))
        .with("chart", file_label(&job.config.level))
        .with("output", output.display())
        .with("output_path", output.display())
        .with(
            "execution_mode",
            match job.config.execution_mode {
                WatchExecutionMode::Interpreter => "Sono VM / Interpreter",
                WatchExecutionMode::SonoGcc => "Sono-GCC",
            },
        )
        .with("width", job.config.width)
        .with("height", job.config.height)
        .with("FPS", job.config.fps)
        .with(
            "resolution",
            format!("{}x{}", job.config.width, job.config.height),
        )
        .with("duration", format!("{:.3}", job.config.duration))
        .with("error", error.unwrap_or_default());
    if let Some(outcome) = &job.outcome {
        event = event
            .with(
                "duration",
                format!("{:.3}", outcome.timeline_end - outcome.timeline_start),
            )
            .with("frame_count", outcome.submitted_frames)
            .with("completed_frames", outcome.submitted_frames)
            .with("cache_status", &outcome.watch_execution_status);
    }
    if let Some(progress) = &job.progress {
        event = event
            .with("completed_frames", progress.completed_frames)
            .with(
                "frame_count",
                progress
                    .total_frames
                    .map_or_else(String::new, |n| n.to_string()),
            )
            .with(
                "progress",
                progress
                    .percentage
                    .map_or_else(String::new, |p| format!("{p:.0}")),
            );
    }
    if let (Some(started), Some(finished)) = (job.started_unix_time_ms, job.finished_unix_time_ms) {
        event = event.with(
            "render_time",
            format!("{:.3}", finished.saturating_sub(started) as f64 / 1_000.0),
        );
        event = event.with(
            "total_time",
            format!("{:.3}", finished.saturating_sub(started) as f64 / 1_000.0),
        );
    }
    event
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_str().unwrap_or_default())
        .to_owned()
}

fn execution_backend(config: &RenderConfig) -> String {
    let backend = match config.backend {
        RenderBackend::Cpu => "CPU",
        RenderBackend::Wgpu => "WGPU",
    };
    let vm = match config.execution_mode {
        WatchExecutionMode::Interpreter => "Sono VM",
        WatchExecutionMode::SonoGcc => "Sono-GCC with VM fallback",
    };
    format!("{backend}; {vm}")
}

fn classify_failure(message: &str) -> JobFailureKind {
    let lower = message.to_ascii_lowercase();
    if [
        "temporarily unavailable",
        "temporary network",
        "resource busy",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        JobFailureKind::Transient
    } else if [
        "permission denied",
        "disk full",
        "ffmpeg",
        "ffprobe",
        "device lost",
        "device removed",
        "output appeared while render was running",
        "publishing completed render",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        JobFailureKind::ExternalResource
    } else if [
        "missing or not a file",
        "must include",
        "invalid",
        "must be",
        "configuration",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        JobFailureKind::Configuration
    } else {
        JobFailureKind::Renderer
    }
}

fn find_job_mut<'a>(state: &'a mut PersistedQueue, id: &str) -> Result<&'a mut RenderJob> {
    state
        .jobs
        .iter_mut()
        .find(|job| job.id == id)
        .with_context(|| format!("render job {id:?} was not found"))
}

fn push_log(job: &mut RenderJob, message: impl Into<String>) {
    if job.logs.len() == LOG_LIMIT {
        job.logs.remove(0);
    }
    job.logs.push(JobLogEntry {
        unix_time_ms: unix_time_ms(),
        message: message.into(),
    });
    touch(job);
}

fn touch(job: &mut RenderJob) {
    job.updated_unix_time_ns = unix_time_ns();
}

fn unique_job_id(jobs: &[RenderJob]) -> String {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    loop {
        let candidate = format!(
            "job-{}-{}-{}",
            unix_time_ms(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        if jobs.iter().all(|job| job.id != candidate) {
            return candidate;
        }
    }
}

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn unix_time_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn load_queue(path: &Path) -> Result<PersistedQueue> {
    if !path.exists() {
        return Ok(PersistedQueue::default());
    }
    let bytes =
        fs::read(path).with_context(|| format!("reading render-job store {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing render-job store {}", path.display()))
}

fn save_queue(path: &Path, queue: &PersistedQueue) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating render-job store directory {}", parent.display()))?;
    let _lock = StoreLock::acquire(path)?;
    let persisted = load_queue(path)?;
    let merged = merge_queues(queue, &persisted);
    validate_reserved_outputs(&merged)?;
    let bytes = serde_json::to_vec_pretty(&merged).context("serializing render-job queue")?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).context("creating temporary render-job store")?;
    temp.write_all(&bytes).context("writing render-job store")?;
    temp.as_file()
        .sync_all()
        .context("flushing render-job store")?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("replacing render-job store {}", path.display()))?;
    Ok(())
}

fn validate_reserved_outputs(queue: &PersistedQueue) -> Result<()> {
    let mut outputs = HashMap::<String, String>::new();
    for job in queue.jobs.iter().filter(|job| !job.state.is_terminal()) {
        let Some(path) = job.output_path() else {
            continue;
        };
        let key = path_collision_key(path)?;
        if let Some(previous) = outputs.insert(key, job.id.clone()) {
            bail!(
                "jobs {previous} and {} reserve the same output path {}",
                job.id,
                path.display()
            );
        }
    }
    Ok(())
}

struct StoreLock(PathBuf);

impl StoreLock {
    fn acquire(store: &Path) -> Result<Self> {
        let lock_path = companion_path(store, ".lock");
        let started = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(Self(lock_path));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock_path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = fs::remove_file(&lock_path);
                        continue;
                    }
                    if started.elapsed() > Duration::from_secs(5) {
                        bail!(
                            "timed out waiting for render-job store lock {}",
                            lock_path.display()
                        );
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("locking render-job store {}", store.display()))
                }
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn companion_path(store: &Path, suffix: &str) -> PathBuf {
    let mut name = store.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn refresh_queue_locked(store: &Path, current: &mut PersistedQueue) -> Result<()> {
    let disk = load_queue(store)?;
    *current = merge_queues(current, &disk);
    Ok(())
}

fn merge_queues(local: &PersistedQueue, disk: &PersistedQueue) -> PersistedQueue {
    let mut merged = if local.updated_unix_time_ns > disk.updated_unix_time_ns {
        local
    } else {
        disk
    }
    .clone();
    merged.next_order = local.next_order.max(disk.next_order);
    merged.version = STORE_VERSION;
    merged
        .removed_job_ids
        .extend(local.removed_job_ids.iter().cloned());
    merged.removed_job_ids.sort();
    merged.removed_job_ids.dedup();
    for incoming in &local.jobs {
        if merged.removed_job_ids.contains(&incoming.id) {
            continue;
        }
        if let Some(existing) = merged.jobs.iter_mut().find(|job| job.id == incoming.id) {
            let mut selected = if incoming.updated_unix_time_ns > existing.updated_unix_time_ns {
                incoming.clone()
            } else {
                existing.clone()
            };
            selected.cancellation_requested |=
                existing.cancellation_requested || incoming.cancellation_requested;
            selected
                .previous_attempt_diagnostics
                .extend(existing.previous_attempt_diagnostics.iter().cloned());
            selected
                .previous_attempt_diagnostics
                .extend(incoming.previous_attempt_diagnostics.iter().cloned());
            selected.previous_attempt_diagnostics.sort();
            selected.previous_attempt_diagnostics.dedup();
            let mut logs = existing
                .logs
                .iter()
                .chain(incoming.logs.iter())
                .cloned()
                .collect::<Vec<_>>();
            logs.sort_by_key(|entry| entry.unix_time_ms);
            logs.dedup_by(|a, b| a.unix_time_ms == b.unix_time_ms && a.message == b.message);
            if logs.len() > LOG_LIMIT {
                logs.drain(..logs.len() - LOG_LIMIT);
            }
            selected.logs = logs;
            *existing = selected;
        } else {
            merged.jobs.push(incoming.clone());
        }
    }
    merged
        .jobs
        .retain(|job| !merged.removed_job_ids.contains(&job.id));
    if local.updated_unix_time_ns > disk.updated_unix_time_ns {
        merged.paused = local.paused;
        merged.updated_unix_time_ns = local.updated_unix_time_ns;
    }
    merged
}

fn claim_path(store: &Path, id: &str) -> PathBuf {
    companion_path(store, &format!(".{id}.claim"))
}

fn acquire_claim(store: &Path, id: &str) -> bool {
    let path = claim_path(store, id);
    for _ in 0..2 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                let _ = file.sync_all();
                return true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                release_stale_claim(store, id);
            }
            Err(_) => return false,
        }
    }
    false
}

fn release_claim(store: &Path, id: &str) {
    let _ = fs::remove_file(claim_path(store, id));
}

fn release_stale_claim(store: &Path, id: &str) {
    let path = claim_path(store, id);
    if let Ok(text) = fs::read_to_string(&path) {
        let owner = text.trim().parse::<u32>().ok();
        if !owner.is_some_and(process_is_running) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    if pid == std::process::id() {
        return true;
    }
    // PROCESS_QUERY_LIMITED_INFORMATION; failed access is treated as dead and
    // will surface as an interrupted job rather than being claimed as done.
    let handle = unsafe { OpenProcess(0x1000, 0, pid) };
    if handle.is_null() {
        false
    } else {
        unsafe { CloseHandle(handle) };
        true
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if pid == std::process::id() {
        return true;
    }
    let result = unsafe { kill(pid as i32, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1)
}

#[cfg(not(any(windows, unix)))]
fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

pub fn default_store_path() -> PathBuf {
    crate::store_paths::default_job_store_path()
}

fn ensure_output_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        bail!("output directory does not exist: {}", parent.display());
    }
    Ok(())
}

fn path_collision_key(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let value = normalized.to_string_lossy().into_owned();
    #[cfg(windows)]
    return Ok(value.to_ascii_lowercase());
    #[cfg(not(windows))]
    Ok(value)
}

fn staged_output_path(output: &Path, id: &str) -> PathBuf {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stem = output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("render");
    parent.join(format!(".{stem}.{id}.partial.mp4"))
}

fn prepare_stage(path: &Path, remove_previous_owned: bool) -> Result<()> {
    if path.exists() {
        if !remove_previous_owned {
            bail!(
                "job staging path already exists and is not owned by this job: {}",
                path.display()
            );
        }
        fs::remove_file(path)
            .with_context(|| format!("removing interrupted job staging file {}", path.display()))?;
    }
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("reserving render staging path {}", path.display()))?;
    Ok(())
}

fn set_stage_owned(shared: &Shared, id: &str, owned: bool) -> Result<()> {
    let mut state = shared
        .state
        .lock()
        .map_err(|_| anyhow::anyhow!("render queue state lock poisoned"))?;
    refresh_queue_locked(&shared.store_path, &mut state)?;
    let job = find_job_mut(&mut state, id)?;
    job.staging_path_owned = owned;
    touch(job);
    save_queue(&shared.store_path, &state)
}

fn ensure_output_not_input(output: &Path, config: &RenderConfig) -> Result<()> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = output.file_name().context("output path must name a file")?;
    let candidate = fs::canonicalize(parent)?.join(file_name);
    let inputs = [
        Some(config.engine.as_path()),
        Some(config.resources.as_path()),
        Some(config.level.as_path()),
        config.music.as_deref(),
        config.mv.as_deref(),
        config.resource_overrides.particles.as_deref(),
        config.resource_overrides.effects.as_deref(),
        config.resource_overrides.background.as_deref(),
    ];
    for input in inputs.into_iter().flatten() {
        if fs::canonicalize(input).ok().as_ref() == Some(&candidate) {
            bail!(
                "render output must not overwrite an input file: {}",
                output.display()
            );
        }
    }
    Ok(())
}

fn publish_output(staged: &Path, output: &Path) -> Result<()> {
    if output.exists() {
        bail!(
            "output appeared while render was running; refusing to replace {}",
            output.display()
        );
    }
    fs::hard_link(staged, output).with_context(|| {
        format!(
            "publishing completed render without overwriting output (staging file remains at {}): {}",
            staged.display(),
            output.display()
        )
    })?;
    // The final hard link is already durably available. Failure to remove the
    // redundant staging name must not turn a confirmed output into a failed job.
    let _ = fs::remove_file(staged);
    Ok(())
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn config(root: &Path, output_name: &str) -> RenderConfig {
        let engine = root.join("engine.zip");
        let resources = root.join("resources.scp");
        let level = root.join("level.data");
        fs::write(&engine, b"engine").unwrap();
        fs::write(&resources, b"resources").unwrap();
        fs::write(&level, b"level").unwrap();
        let mut config = RenderConfig::default();
        config.engine = engine;
        config.resources = resources;
        config.level = level;
        config.layers.bgm = false;
        config.output = Some(root.join(output_name));
        config
    }

    fn manager(dir: &TempDir, config: JobManagerConfig) -> RenderJobManager {
        RenderJobManager::open_at(dir.path().join("jobs.json"), config).unwrap()
    }

    #[test]
    fn submission_uses_unique_ids_snapshots_config_and_reserves_output() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(&dir, JobManagerConfig::default());
        let mut submitted = config(dir.path(), "result.mp4");
        submitted.width = 854;
        let first = manager.enqueue(submitted.clone(), None).unwrap();
        submitted.width = 640;
        assert_eq!(manager.inspect(&first).unwrap().config.width, 854);
        assert!(!first.contains("result.mp4"));
        assert!(manager
            .enqueue(config(dir.path(), "./result.mp4"), None)
            .is_err());
        let second = manager
            .enqueue(config(dir.path(), "another.mp4"), None)
            .unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn output_collision_is_checked_against_existing_files_and_queued_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(&dir, JobManagerConfig::default());
        let output = dir.path().join("present.mp4");
        fs::write(&output, b"keep me").unwrap();
        assert!(manager
            .enqueue(config(dir.path(), "present.mp4"), None)
            .is_err());
        assert_eq!(fs::read(output).unwrap(), b"keep me");
    }

    #[test]
    fn separate_gui_and_cli_managers_merge_queue_updates_and_remote_cancel_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let gui = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let cli = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let first = gui.enqueue(config(dir.path(), "gui.mp4"), None).unwrap();
        let second = cli.enqueue(config(dir.path(), "cli.mp4"), None).unwrap();
        assert_eq!(gui.jobs().unwrap().len(), 2);
        assert_eq!(cli.jobs().unwrap().len(), 2);

        {
            let mut state = gui.shared.state.lock().unwrap();
            let active = find_job_mut(&mut state, &first).unwrap();
            active.state = JobState::Rendering;
            active.worker_process_id = Some(std::process::id());
            touch(active);
            save_queue(&path, &state).unwrap();
        }
        cli.cancel(&first).unwrap();
        assert!(gui.inspect(&first).unwrap().cancellation_requested);
        cli.cancel(&second).unwrap();
        assert_eq!(gui.inspect(&second).unwrap().state, JobState::Cancelled);
    }

    #[test]
    fn queue_controls_cancel_clear_reorder_and_reject_invalid_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(&dir, JobManagerConfig::default());
        let first = manager
            .enqueue(config(dir.path(), "one.mp4"), None)
            .unwrap();
        let second = manager
            .enqueue(config(dir.path(), "two.mp4"), None)
            .unwrap();
        let third = manager
            .enqueue(config(dir.path(), "three.mp4"), None)
            .unwrap();
        manager.reorder(&third, 0).unwrap();
        assert_eq!(manager.jobs().unwrap()[0].id, third);
        manager.cancel(&second).unwrap();
        assert_eq!(manager.inspect(&second).unwrap().state, JobState::Cancelled);
        assert!(manager.retry(&second).is_err());
        assert!(manager.remove(&first).is_err());
        assert_eq!(manager.cancel_queued().unwrap(), 2);
        assert_eq!(manager.clear_terminal().unwrap(), 3);
        assert!(manager.jobs().unwrap().is_empty());
    }

    #[test]
    fn cancelled_attempt_is_durable_and_independent_of_terminal_queue_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let manager = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let mut submitted = config(dir.path(), "cancelled.mp4");
        submitted.notifications.discord.bot_token_env = "eyJhbGciOi.secret.value".into();
        let id = manager.enqueue(submitted, None).unwrap();
        manager.cancel(&id).unwrap();

        let history = manager.history_store().unwrap();
        let page = history.list(&crate::render_history::HistoryQuery::default());
        assert_eq!(page.entries.len(), 1);
        let record = &page.entries[0];
        assert_eq!(record.job_id, id);
        assert_eq!(
            record.status,
            crate::render_history::HistoryStatus::Cancelled
        );
        assert_eq!(
            record.config_snapshot.notifications.discord.bot_token_env,
            crate::notifications::DiscordNotificationConfig::default().bot_token_env
        );
        assert_eq!(manager.clear_terminal().unwrap(), 1);
        assert!(manager.jobs().unwrap().is_empty());
        assert_eq!(
            history
                .list(&crate::render_history::HistoryQuery::default())
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn startup_backfills_a_terminal_attempt_without_creating_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let manager = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let id = manager
            .enqueue(config(dir.path(), "cancel-before-history.mp4"), None)
            .unwrap();
        {
            let mut state = manager.shared.state.lock().unwrap();
            let job = find_job_mut(&mut state, &id).unwrap();
            job.state = JobState::Failed;
            job.attempt = 1;
            job.finished_unix_time_ms = Some(unix_time_ms());
            job.diagnostic = Some("simulated process result".into());
            touch(job);
            save_queue(&path, &state).unwrap();
        }
        let store = manager.history_store().unwrap();
        assert!(store
            .list(&crate::render_history::HistoryQuery::default())
            .entries
            .is_empty());
        drop(manager);

        let recovered = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let entries = recovered
            .history_store()
            .unwrap()
            .list(&crate::render_history::HistoryQuery::default())
            .entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].job_id, id);
        assert_eq!(
            entries[0].status,
            crate::render_history::HistoryStatus::Failed
        );
        drop(recovered);
        let reopened = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        assert_eq!(
            reopened
                .history_store()
                .unwrap()
                .list(&crate::render_history::HistoryQuery::default())
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn history_rerender_submission_uses_a_new_job_and_preserves_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let manager = manager(&dir, JobManagerConfig::default());
        let original = manager
            .enqueue(config(dir.path(), "original.mp4"), None)
            .unwrap();
        {
            let mut state = manager.shared.state.lock().unwrap();
            let job = find_job_mut(&mut state, &original).unwrap();
            job.state = JobState::Completed;
            job.attempt = 1;
            job.started_unix_time_ms = Some(unix_time_ms().saturating_sub(100));
            job.finished_unix_time_ms = Some(unix_time_ms());
            touch(job);
            save_queue(manager.store_path(), &state).unwrap();
        }
        let history = manager.history_store().unwrap();
        history
            .save_attempt(&manager.inspect(&original).unwrap())
            .unwrap();
        let entry = history
            .list(&crate::render_history::HistoryQuery::default())
            .entries
            .remove(0);
        let mut rerender = entry.config_snapshot.clone();
        rerender.output = Some(dir.path().join("rerendered.mp4"));
        let new_id = manager
            .enqueue_with_metadata(
                rerender.clone(),
                None,
                None,
                RenderJobMetadata {
                    project_path: entry.project_path.clone(),
                    original_job_id: Some(entry.job_id.clone()),
                    rerendered_from_history_id: Some(entry.id.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(new_id, original);
        let queued = manager.inspect(&new_id).unwrap();
        assert_eq!(queued.config, rerender);
        assert_eq!(
            queued.rerendered_from_history_id.as_deref(),
            Some(entry.id.as_str())
        );
        assert_eq!(queued.original_job_id.as_deref(), Some(original.as_str()));
    }

    #[test]
    fn retries_retain_diagnostics_and_recovery_never_marks_unconfirmed_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let manager = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let id = manager
            .enqueue(config(dir.path(), "retry.mp4"), None)
            .unwrap();
        {
            let mut state = manager.shared.state.lock().unwrap();
            let job = find_job_mut(&mut state, &id).unwrap();
            job.state = JobState::Failed;
            job.failure_kind = Some(JobFailureKind::Renderer);
            job.diagnostic = Some("test failure".into());
            touch(job);
            save_queue(&path, &state).unwrap();
        }
        manager.retry(&id).unwrap();
        let retry = manager.inspect(&id).unwrap();
        assert_eq!(retry.state, JobState::Queued);
        assert_eq!(retry.previous_attempt_diagnostics, ["test failure"]);
        {
            let mut state = manager.shared.state.lock().unwrap();
            let job = find_job_mut(&mut state, &id).unwrap();
            job.state = JobState::Rendering;
            job.worker_process_id = None;
            job.progress = Some(ExportProgress {
                phase: ExportPhase::Rendering,
                completed_frames: 2,
                total_frames: Some(10),
                percentage: Some(20.0),
                elapsed_seconds: 1.0,
                frames_per_second: Some(2.0),
                eta_seconds: Some(4.0),
            });
            touch(job);
            save_queue(&path, &state).unwrap();
        }
        drop(manager);
        let recovered = RenderJobManager::open_at(&path, JobManagerConfig::default()).unwrap();
        let job = recovered.inspect(&id).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.failure_kind, Some(JobFailureKind::Interrupted));
        assert!(job
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("completion was confirmed"));
        assert_eq!(job.progress.unwrap().completed_frames, 2);
    }

    #[test]
    fn scheduler_selects_priority_but_ages_old_low_priority_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = PersistedQueue::default();
        for (priority, name, order) in [
            (JobPriority::High, "high.mp4", 1),
            (JobPriority::Normal, "normal.mp4", 2),
            (JobPriority::Low, "low.mp4", 3),
        ] {
            let mut job = RenderJob {
                id: name.into(),
                profile_id: None,
                profile_name: None,
                project_path: None,
                original_job_id: None,
                retried_from_history_id: None,
                rerendered_from_history_id: None,
                reproduced_from_manifest_sha256: None,
                state: JobState::Queued,
                priority,
                queue_order: order,
                updated_unix_time_ns: unix_time_ns(),
                submitted_unix_time_ms: unix_time_ms(),
                started_unix_time_ms: None,
                worker_process_id: None,
                finished_unix_time_ms: None,
                attempt: 0,
                previous_attempt_diagnostics: Vec::new(),
                renderer_version: env!("CARGO_PKG_VERSION").into(),
                config: config(dir.path(), name),
                execution_backend: "CPU; Sono VM".into(),
                progress: None,
                outcome: None,
                diagnostic_report: None,
                logs: Vec::new(),
                diagnostic: None,
                failure_kind: None,
                cancellation_requested: false,
                staging_path_owned: false,
                history_persisted: false,
            };
            push_log(&mut job, "queued");
            state.jobs.push(job);
        }
        let mut legacy_job = serde_json::to_value(&state.jobs[0]).unwrap();
        legacy_job
            .as_object_mut()
            .unwrap()
            .remove("diagnostic_report");
        let migrated: RenderJob = serde_json::from_value(legacy_job).unwrap();
        assert!(migrated.diagnostic_report.is_none());
        assert_eq!(state.jobs[next_queued_job(&state).unwrap()].id, "high.mp4");
        state.jobs[2].submitted_unix_time_ms = unix_time_ms().saturating_sub(8_000_000);
        assert_eq!(state.jobs[next_queued_job(&state).unwrap()].id, "low.mp4");
    }

    #[test]
    fn manager_configuration_is_validated_and_defaults_conservatively() {
        assert_eq!(JobManagerConfig::default().max_concurrent_jobs, 1);
        assert!(JobManagerConfig {
            max_concurrent_jobs: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(JobManagerConfig {
            max_concurrent_jobs: crate::job_config::MAX_WORKERS + 1,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
