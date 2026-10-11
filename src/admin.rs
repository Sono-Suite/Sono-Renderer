//! Hosted operations API. This module is a client of the authoritative queue,
//! history, performance, diagnostics, cache, and profile owners; its DTOs are
//! deliberately separate from those stores' persistence representations.

use crate::{
    config_profiles::ConfigProfileManager,
    job_manager::{JobState, RenderJob, RenderJobManager},
    performance::{PerformanceStore, RenderPerformanceRecord},
    render_diagnostics::{RenderDiagnosticRecord, RenderDiagnosticStore},
    render_history::{HistoryQuery, HistoryStatus, RenderHistoryRecord, RenderHistoryStore},
};
use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive},
        Html, IntoResponse, Response, Sse,
    },
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, SocketAddr, TcpListener},
    path::{Path as FsPath, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;

pub const ADMIN_CONFIG_VERSION: u32 = 1;
const ADMIN_CONFIG_FILE: &str = "admin-server.json";
static PROCESS_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdminServerConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub bind_address: String,
    /// Zero asks the operating system for an available port.
    pub port: u16,
}

impl Default for AdminServerConfig {
    fn default() -> Self {
        Self {
            schema_version: ADMIN_CONFIG_VERSION,
            enabled: false,
            bind_address: "127.0.0.1".into(),
            port: 0,
        }
    }
}

impl AdminServerConfig {
    pub fn load_default() -> Result<Self> {
        Self::load_at(default_config_path())
    }

    pub fn load_at(path: impl AsRef<FsPath>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let config: Self = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("decoding admin-server settings {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save_default(&self) -> Result<()> {
        self.save_at(default_config_path())
    }

    pub fn save_at(&self, path: impl AsRef<FsPath>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(FsPath::new("."));
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".admin-settings-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        use std::io::Write;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(path)
            .map_err(|error| error.error)
            .with_context(|| {
                format!("atomically saving admin-server settings {}", path.display())
            })?;
        Ok(())
    }

    pub fn validate(&self) -> Result<SocketAddr> {
        if self.schema_version != ADMIN_CONFIG_VERSION {
            bail!(
                "unsupported admin-server settings schema version {}",
                self.schema_version
            );
        }
        let ip: IpAddr = self
            .bind_address
            .parse()
            .context("admin bind_address must be a literal IPv4 or IPv6 address")?;
        Ok(SocketAddr::new(ip, self.port))
    }

    pub fn is_loopback(&self) -> Result<bool> {
        Ok(self.validate()?.ip().is_loopback())
    }
}

pub fn default_config_path() -> PathBuf {
    crate::store_paths::config_root().join(ADMIN_CONFIG_FILE)
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminConfigView {
    pub schema_version: u32,
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub authentication: &'static str,
    pub restart_required: bool,
}

impl AdminConfigView {
    pub fn from_config(config: &AdminServerConfig) -> Self {
        Self {
            schema_version: config.schema_version,
            enabled: config.enabled,
            bind_address: config.bind_address.clone(),
            port: config.port,
            authentication: if config.is_loopback().unwrap_or(false) {
                "local operating-system boundary"
            } else {
                "HTTPS bearer token"
            },
            restart_required: true,
        }
    }
}

#[derive(Clone)]
struct AdminState {
    jobs: Arc<RenderJobManager>,
    history: RenderHistoryStore,
    performance: PerformanceStore,
    diagnostics: RenderDiagnosticStore,
    profiles: ConfigProfileManager,
    config: Arc<std::sync::RwLock<AdminServerConfig>>,
    started: Instant,
    instance_id: String,
    resource_sampler: ResourceSampler,
    process_logs: crate::app_log::ProcessLogStore,
    process_log_capture_active: bool,
}

impl AdminState {
    fn open(jobs: Arc<RenderJobManager>, config: AdminServerConfig) -> Result<Self> {
        let history = jobs.history_store()?;
        #[cfg(test)]
        let (performance, diagnostics, profiles) = {
            let root = jobs.store_path().parent().unwrap_or(FsPath::new("."));
            let performance = PerformanceStore::open_at(
                root.join("admin-test-performance"),
                crate::performance::DEFAULT_RETENTION,
            )?;
            let diagnostics = RenderDiagnosticStore::open_at(
                root.join("admin-test-diagnostics"),
                crate::render_diagnostics::DEFAULT_DIAGNOSTIC_RETENTION,
            )?;
            let profiles = ConfigProfileManager::open_at(root.join("admin-test-profiles.json"))?;
            (performance, diagnostics, profiles)
        };
        #[cfg(not(test))]
        let (performance, diagnostics, profiles) = (
            PerformanceStore::open_default()?,
            RenderDiagnosticStore::open_default()?,
            ConfigProfileManager::open_default()?,
        );
        #[cfg(test)]
        let process_logs = crate::app_log::ProcessLogStore::default();
        #[cfg(test)]
        let process_log_capture_active = false;
        #[cfg(not(test))]
        let process_logs = crate::app_log::global_store();
        #[cfg(not(test))]
        let process_log_capture_active = crate::app_log::initialize();
        let instance_id = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        Ok(Self {
            jobs,
            history,
            performance,
            diagnostics,
            profiles,
            config: Arc::new(std::sync::RwLock::new(config)),
            started: *PROCESS_START.get_or_init(Instant::now),
            instance_id,
            resource_sampler: ResourceSampler::default(),
            process_logs,
            process_log_capture_active,
        })
    }

    fn overview(&self) -> Result<OverviewDto> {
        let jobs = self.jobs.jobs()?;
        let history = self.history.list(&HistoryQuery {
            limit: Some(100),
            ..Default::default()
        });
        let paused = self.jobs.is_paused()?;
        let summary = JobCountsDto {
            queued: jobs
                .iter()
                .filter(|job| job.state == JobState::Queued)
                .count(),
            active: jobs.iter().filter(|job| job.state.is_active()).count(),
            completed: jobs
                .iter()
                .filter(|job| job.state == JobState::Completed)
                .count(),
            failed: jobs
                .iter()
                .filter(|job| job.state == JobState::Failed)
                .count(),
            cancelled: jobs
                .iter()
                .filter(|job| job.state == JobState::Cancelled)
                .count(),
            paused,
        };
        let recent_failures = history
            .entries
            .iter()
            .filter(|record| record.status == HistoryStatus::Failed)
            .take(5)
            .map(HistoryDto::from)
            .collect();
        let latest_performance = self.performance.list(1)?.first().map(PerformanceDto::from);
        Ok(OverviewDto {
            jobs: summary,
            recent_failures,
            latest_performance,
            resources: self.resource_sampler.sample(),
        })
    }
}

#[derive(Clone)]
struct AuthState(Option<String>);

pub struct AdminServerHandle {
    pub local_addr: SocketAddr,
    pub https: bool,
    running: Arc<AtomicBool>,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

/// Checks only operator-supplied TLS/token prerequisites; it performs no bind
/// and is safe for a CLI validation command.
pub fn validate_startup_security(config: &AdminServerConfig) -> Result<()> {
    let address = config.validate()?;
    if !config.enabled || address.ip().is_loopback() {
        return Ok(());
    }
    let token = std::env::var("SONO_RENDERER_ADMIN_TOKEN").ok();
    let cert = std::env::var_os("SONO_RENDERER_ADMIN_TLS_CERT").map(PathBuf::from);
    let key = std::env::var_os("SONO_RENDERER_ADMIN_TLS_KEY").map(PathBuf::from);
    validate_remote_prerequisites(token.as_deref(), cert.as_deref(), key.as_deref())
}

fn validate_remote_prerequisites(
    token: Option<&str>,
    cert: Option<&FsPath>,
    key: Option<&FsPath>,
) -> Result<()> {
    let token = token.context("remote admin binding requires SONO_RENDERER_ADMIN_TOKEN")?;
    if token.as_bytes().len() < 32 {
        bail!("SONO_RENDERER_ADMIN_TOKEN must contain at least 32 bytes");
    }
    for (path, variable, description) in [
        (cert, "SONO_RENDERER_ADMIN_TLS_CERT", "TLS certificate"),
        (key, "SONO_RENDERER_ADMIN_TLS_KEY", "TLS private key"),
    ] {
        let path = path.with_context(|| format!("remote admin binding requires {variable}"))?;
        if !path.is_file() {
            bail!("admin {description} path from {variable} is not a regular file");
        }
    }
    Ok(())
}

impl AdminServerHandle {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn url(&self) -> String {
        format!(
            "{}://{}{}",
            if self.https { "https" } else { "http" },
            self.local_addr,
            "/"
        )
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AdminServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the hosted service. The caller must provide the same live job-manager
/// instance used by its UI if web cancellation should affect active work.
pub fn start(
    config: &AdminServerConfig,
    jobs: Arc<RenderJobManager>,
) -> Result<Option<AdminServerHandle>> {
    if !config.enabled {
        return Ok(None);
    }
    let configured_addr = config.validate()?;
    let remote = !configured_addr.ip().is_loopback();
    validate_startup_security(config)?;
    let token = if remote {
        let token = std::env::var("SONO_RENDERER_ADMIN_TOKEN")
            .context("remote admin binding requires SONO_RENDERER_ADMIN_TOKEN")?;
        if token.as_bytes().len() < 32 {
            bail!("SONO_RENDERER_ADMIN_TOKEN must contain at least 32 bytes");
        }
        Some(token)
    } else {
        None
    };
    let tls_cert = if remote {
        Some(PathBuf::from(
            std::env::var_os("SONO_RENDERER_ADMIN_TLS_CERT")
                .context("remote admin binding requires SONO_RENDERER_ADMIN_TLS_CERT")?,
        ))
    } else {
        None
    };
    let tls_key = if remote {
        Some(PathBuf::from(
            std::env::var_os("SONO_RENDERER_ADMIN_TLS_KEY")
                .context("remote admin binding requires SONO_RENDERER_ADMIN_TLS_KEY")?,
        ))
    } else {
        None
    };
    let listener = TcpListener::bind(configured_addr).with_context(|| {
        format!("binding admin server at {configured_addr}; no alternate port was selected")
    })?;
    listener.set_nonblocking(true)?;
    let local_addr = listener.local_addr()?;
    let state = AdminState::open(jobs, config.clone())?;
    log::info!(target: "renderer::admin", "admin service bound to {local_addr}");
    let app = build_router(
        state,
        AuthState(token),
        LoopbackHostGuard {
            enabled: local_addr.ip().is_loopback(),
            port: local_addr.port(),
        },
    );
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
    let running = Arc::new(AtomicBool::new(false));
    let worker_running = running.clone();
    let worker = thread::Builder::new()
        .name("admin-web-server".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(anyhow::anyhow!(
                        "starting admin server runtime: {error}"
                    )));
                    return;
                }
            };
            runtime.block_on(async move {
                if let (Some(cert), Some(key)) = (tls_cert, tls_key) {
                    let tls =
                        match axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert, &key)
                            .await
                        {
                            Ok(tls) => tls,
                            Err(error) => {
                                let _ = ready_tx.send(Err(anyhow::Error::new(error)
                                    .context("loading admin HTTPS certificate/key")));
                                return;
                            }
                        };
                    let handle = axum_server::Handle::new();
                    let shutdown_handle = handle.clone();
                    tokio::spawn(async move {
                        if shutdown_rx.await.is_ok() {
                            shutdown_handle.graceful_shutdown(Some(Duration::from_secs(5)));
                        }
                    });
                    worker_running.store(true, Ordering::Release);
                    let _ = ready_tx.send(Ok(()));
                    let _ = axum_server::from_tcp_rustls(listener, tls)
                        .handle(handle)
                        .serve(app.into_make_service())
                        .await;
                } else {
                    let listener = match tokio::net::TcpListener::from_std(listener) {
                        Ok(listener) => listener,
                        Err(error) => {
                            let _ =
                                ready_tx
                                    .send(Err(anyhow::Error::new(error)
                                        .context("adopting admin listener")));
                            return;
                        }
                    };
                    worker_running.store(true, Ordering::Release);
                    let _ = ready_tx.send(Ok(()));
                    let _ = axum::serve(listener, app)
                        .with_graceful_shutdown(async {
                            let _ = shutdown_rx.await;
                        })
                        .await;
                }
                worker_running.store(false, Ordering::Release);
            });
        })
        .context("spawning admin server thread")?;
    match ready_rx.recv_timeout(Duration::from_secs(15)) {
        Ok(Ok(())) => Ok(Some(AdminServerHandle {
            local_addr,
            https: remote,
            running,
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
        })),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        }
        Err(error) => {
            let _ = shutdown_tx.send(());
            let _ = worker.join();
            Err(anyhow::anyhow!(
                "admin server did not become ready: {error}"
            ))
        }
    }
}

#[derive(Clone, Copy)]
struct LoopbackHostGuard {
    enabled: bool,
    port: u16,
}

fn build_router(state: AdminState, auth: AuthState, host_guard: LoopbackHostGuard) -> Router<()> {
    let protected: Router<AdminState> = Router::new()
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/events", get(events))
        .route("/api/v1/jobs", get(list_jobs))
        .route("/api/v1/jobs/{id}", get(get_job))
        .route("/api/v1/jobs/{id}/cancel", post(cancel_job))
        .route("/api/v1/jobs/{id}/retry", post(retry_job))
        .route("/api/v1/queue/pause", post(pause_queue))
        .route("/api/v1/queue/resume", post(resume_queue))
        .route("/api/v1/history", get(history))
        .route("/api/v1/performance", get(performance))
        .route("/api/v1/performance/compare", get(compare_performance))
        .route("/api/v1/cache", get(cache_inventory))
        .route(
            "/api/v1/cache/{id}",
            get(inspect_cache).delete(remove_cache),
        )
        .route("/api/v1/cache/validate", post(validate_cache))
        .route("/api/v1/cache/clear", post(clear_cache))
        .route("/api/v1/profiles", get(profiles))
        .route("/api/v1/profiles/select", post(select_profile))
        .route(
            "/api/v1/profiles/{id}",
            patch(rename_profile).delete(delete_profile),
        )
        .route("/api/v1/diagnostics", get(diagnostics))
        .route("/api/v1/logs", get(logs))
        .route(
            "/api/v1/admin-config",
            get(admin_config).put(update_admin_config),
        )
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024))
        .route_layer(middleware::from_fn_with_state(auth, authorize));
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/", get(index))
        .route("/admin.js", get(javascript))
        .route("/admin.css", get(stylesheet))
        .merge(protected)
        .layer(middleware::from_fn_with_state(
            host_guard,
            enforce_loopback_host,
        ))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn enforce_loopback_host(
    axum::extract::State(guard): axum::extract::State<LoopbackHostGuard>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !guard.enabled {
        return next.run(request).await;
    }
    let valid = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| valid_loopback_host(value, guard.port));
    if valid {
        next.run(request).await
    } else {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorDto {
                error: "invalid Host header for loopback administration".into(),
            }),
        )
            .into_response()
    }
}

fn valid_loopback_host(value: &str, expected_port: u16) -> bool {
    let Ok(authority) = value.parse::<axum::http::uri::Authority>() else {
        return false;
    };
    let hostname = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let loopback = hostname.eq_ignore_ascii_case("localhost")
        || hostname
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let port_matches = authority.port_u16().unwrap_or(80) == expected_port;
    loopback && port_matches
}

async fn authorize(State(auth): State<AuthState>, request: Request<Body>, next: Next) -> Response {
    let Some(expected) = auth.0 else {
        return next.run(request).await;
    };
    let supplied = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");
    if constant_time_eq(expected.as_bytes(), supplied.as_bytes()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorDto {
                error: "operator token required".into(),
            }),
        )
            .into_response()
    }
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = (left.len() ^ right.len()) as u64;
    for index in 0..left.len().max(right.len()) {
        difference |= u64::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

async fn index() -> impl IntoResponse {
    Html(include_str!("../web/dist/index.html"))
}
async fn javascript() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_bytes!("../web/dist/admin.js").as_slice(),
    )
}
async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_bytes!("../web/dist/admin.css").as_slice(),
    )
}

#[derive(Serialize)]
struct HealthDto {
    status: &'static str,
    version: &'static str,
    uptime_seconds: u64,
    instance_id: String,
}
async fn health(State(state): State<AdminState>) -> Json<HealthDto> {
    Json(HealthDto {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: state.started.elapsed().as_secs(),
        instance_id: state.instance_id.clone(),
    })
}

#[derive(Serialize)]
struct ErrorDto {
    error: String,
}
type ApiResult<T> = std::result::Result<Json<T>, (StatusCode, Json<ErrorDto>)>;
fn api_error(error: impl std::fmt::Display) -> (StatusCode, Json<ErrorDto>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorDto {
            error: redact(&error.to_string()),
        }),
    )
}

#[derive(Serialize)]
struct JobCountsDto {
    queued: usize,
    active: usize,
    completed: usize,
    failed: usize,
    cancelled: usize,
    paused: bool,
}
#[derive(Clone, Serialize)]
struct ResourceDto {
    process_cpu_percent: Option<f64>,
    system_cpu_percent: Option<f64>,
    process_memory_bytes: Option<u64>,
    process_io_bytes_per_second: Option<f64>,
    gpu_utilization_percent: Option<f64>,
    vram_bytes: Option<u64>,
    disk_io_bytes_per_second: Option<f64>,
    availability: String,
}

/// Low-rate, on-demand sampling for the optional admin console. There is no
/// worker while the service is disabled; overview/SSE requests sample at most
/// once per second and reuse the last view between samples.
#[derive(Clone, Default)]
struct ResourceSampler {
    state: Arc<Mutex<ResourceSamplerState>>,
}

#[derive(Default)]
struct ResourceSamplerState {
    sampled_at: Option<Instant>,
    previous: Option<RawResourceSample>,
    last_view: Option<ResourceDto>,
}

#[derive(Clone)]
struct RawResourceSample {
    sampled_at: Instant,
    process_cpu_100ns: Option<u64>,
    system_cpu_100ns: Option<(u64, u64, u64)>, // idle, kernel, user
    process_io_bytes: Option<u64>,
    process_memory_bytes: Option<u64>,
}

impl ResourceSampler {
    fn sample(&self) -> ResourceDto {
        let Ok(mut state) = self.state.lock() else {
            return unavailable_resource_view("resource sampler lock is unavailable");
        };
        if state
            .sampled_at
            .is_some_and(|sampled_at| sampled_at.elapsed() < Duration::from_secs(1))
        {
            if let Some(last_view) = &state.last_view {
                return last_view.clone();
            }
        }

        let current = platform_resource_sample();
        let previous = state.previous.as_ref();
        let elapsed = previous.map(|sample| current.sampled_at.duration_since(sample.sampled_at));
        let process_cpu_percent = previous.and_then(|previous| {
            percent_delta(
                previous.process_cpu_100ns?,
                current.process_cpu_100ns?,
                elapsed?,
                std::thread::available_parallelism()
                    .map(|count| count.get())
                    .unwrap_or(1),
            )
        });
        let system_cpu_percent = previous.and_then(|previous| {
            let (old_idle, old_kernel, old_user) = previous.system_cpu_100ns?;
            let (idle, kernel, user) = current.system_cpu_100ns?;
            system_cpu_percent(
                idle.saturating_sub(old_idle),
                kernel.saturating_sub(old_kernel),
                user.saturating_sub(old_user),
            )
        });
        let process_io_bytes_per_second = previous.and_then(|previous| {
            let duration = elapsed?.as_secs_f64();
            if duration <= 0.0 {
                return None;
            }
            let delta = current
                .process_io_bytes?
                .saturating_sub(previous.process_io_bytes?);
            Some(delta as f64 / duration)
        });
        let has_rate_sample = process_cpu_percent.is_some()
            || system_cpu_percent.is_some()
            || process_io_bytes_per_second.is_some();
        let availability = if cfg!(windows) {
            let memory_status = if current.process_memory_bytes.is_some() {
                "process working-set memory available"
            } else {
                "process memory counter unavailable"
            };
            let rate_status = if has_rate_sample {
                "one or more CPU/process I/O rates available from two samples"
            } else {
                "CPU/process I/O rates unavailable or waiting for a second sample"
            };
            format!("Windows {memory_status}; {rate_status}. Process I/O is OS-reported read/write transfer volume, not physical disk throughput. GPU/VRAM and physical disk metrics are unavailable.")
        } else {
            "This platform has no host resource sampler; metrics are unavailable, not zero."
                .to_owned()
        };
        let view = ResourceDto {
            process_cpu_percent,
            system_cpu_percent,
            process_memory_bytes: current.process_memory_bytes,
            process_io_bytes_per_second,
            gpu_utilization_percent: None,
            vram_bytes: None,
            // Windows process transfer counters are not physical disk I/O.
            disk_io_bytes_per_second: None,
            availability,
        };
        state.sampled_at = Some(current.sampled_at);
        state.previous = Some(current);
        state.last_view = Some(view.clone());
        view
    }
}

fn unavailable_resource_view(reason: &str) -> ResourceDto {
    ResourceDto {
        process_cpu_percent: None,
        system_cpu_percent: None,
        process_memory_bytes: None,
        process_io_bytes_per_second: None,
        gpu_utilization_percent: None,
        vram_bytes: None,
        disk_io_bytes_per_second: None,
        availability: reason.to_owned(),
    }
}

fn percent_delta(previous: u64, current: u64, elapsed: Duration, processors: usize) -> Option<f64> {
    if elapsed.is_zero() || processors == 0 {
        return None;
    }
    let wall_100ns = elapsed.as_nanos() as f64 / 100.0;
    if wall_100ns <= 0.0 {
        return None;
    }
    Some(
        ((current.saturating_sub(previous) as f64 / (wall_100ns * processors as f64)) * 100.0)
            .clamp(0.0, 100.0),
    )
}

fn percentage(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| (numerator as f64 / denominator as f64 * 100.0).clamp(0.0, 100.0))
}

fn system_cpu_percent(idle_delta: u64, kernel_delta: u64, user_delta: u64) -> Option<f64> {
    let total_delta = kernel_delta.saturating_add(user_delta);
    percentage(idle_delta, total_delta).map(|idle_percent| 100.0 - idle_percent)
}

#[cfg(windows)]
fn platform_resource_sample() -> RawResourceSample {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{
                GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, GetSystemTimes,
                IO_COUNTERS,
            },
        },
    };

    fn filetime(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    // GetCurrentProcess returns a pseudo-handle owned by the OS; it is not
    // closed. Every output pointer below references a live, correctly sized
    // local structure for the duration of the native call.
    let process = unsafe { GetCurrentProcess() };
    let (process_cpu_100ns, process_memory_bytes, process_io_bytes) = unsafe {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let cpu = (GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) != 0)
            .then(|| filetime(kernel).saturating_add(filetime(user)));

        let mut memory = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let memory_ok = K32GetProcessMemoryInfo(
            process,
            (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            memory.cb,
        ) != 0;

        let mut io = IO_COUNTERS::default();
        let io_ok = GetProcessIoCounters(process, &mut io) != 0;
        (
            cpu,
            memory_ok.then_some(memory.WorkingSetSize as u64),
            io_ok.then(|| io.ReadTransferCount.saturating_add(io.WriteTransferCount)),
        )
    };
    let system_cpu_100ns = unsafe {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        (GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0)
            .then(|| (filetime(idle), filetime(kernel), filetime(user)))
    };
    RawResourceSample {
        sampled_at: Instant::now(),
        process_cpu_100ns,
        system_cpu_100ns,
        process_io_bytes,
        process_memory_bytes,
    }
}

#[cfg(not(windows))]
fn platform_resource_sample() -> RawResourceSample {
    RawResourceSample {
        sampled_at: Instant::now(),
        process_cpu_100ns: None,
        system_cpu_100ns: None,
        process_io_bytes: None,
        process_memory_bytes: None,
    }
}
#[derive(Serialize)]
struct OverviewDto {
    jobs: JobCountsDto,
    recent_failures: Vec<HistoryDto>,
    latest_performance: Option<PerformanceDto>,
    resources: ResourceDto,
}
async fn overview(State(state): State<AdminState>) -> ApiResult<OverviewDto> {
    state.overview().map(Json).map_err(api_error)
}

fn job_state_name(state: JobState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}
#[derive(Serialize)]
struct JobDto {
    id: String,
    state: String,
    priority: String,
    attempt: u32,
    submitted_unix_time_ms: u128,
    started_unix_time_ms: Option<u128>,
    finished_unix_time_ms: Option<u128>,
    profile_name: Option<String>,
    renderer_version: String,
    execution_backend: String,
    phase: Option<String>,
    completed_frames: u64,
    total_frames: Option<u64>,
    percentage: Option<f64>,
    output_name: Option<String>,
    output_exists: bool,
    cancellation_requested: bool,
    diagnostic: Option<String>,
    failure_kind: Option<String>,
    logs: Vec<JobLogDto>,
    configuration_snapshot: Option<crate::render::RenderConfig>,
}
#[derive(Serialize)]
struct JobLogDto {
    unix_time_ms: u128,
    message: String,
}
impl From<RenderJob> for JobDto {
    fn from(job: RenderJob) -> Self {
        let progress = job.progress.as_ref();
        let configuration_snapshot = crate::render_history::sanitize_config_snapshot(&job.config);
        let (output_name, output_exists) = job
            .output_path()
            .map(|output| {
                (
                    output
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                    output.exists(),
                )
            })
            .unwrap_or((None, false));
        Self {
            id: job.id,
            state: job_state_name(job.state),
            priority: format!("{:?}", job.priority).to_lowercase(),
            attempt: job.attempt,
            submitted_unix_time_ms: job.submitted_unix_time_ms,
            started_unix_time_ms: job.started_unix_time_ms,
            finished_unix_time_ms: job.finished_unix_time_ms,
            profile_name: job.profile_name,
            renderer_version: job.renderer_version,
            execution_backend: job.execution_backend,
            phase: progress.map(|progress| format!("{:?}", progress.phase).to_lowercase()),
            completed_frames: progress
                .map(|progress| progress.completed_frames)
                .unwrap_or(0),
            total_frames: progress.and_then(|progress| progress.total_frames),
            percentage: progress.and_then(|progress| progress.percentage),
            output_name,
            output_exists,
            cancellation_requested: job.cancellation_requested,
            diagnostic: job.diagnostic.map(|message| redact(&message)),
            failure_kind: job
                .failure_kind
                .map(|kind| format!("{:?}", kind).to_lowercase()),
            logs: job
                .logs
                .into_iter()
                .map(|log| JobLogDto {
                    unix_time_ms: log.unix_time_ms,
                    message: redact(&log.message),
                })
                .collect(),
            configuration_snapshot: Some(configuration_snapshot),
        }
    }
}
async fn list_jobs(
    State(state): State<AdminState>,
    Query(params): Query<LimitParams>,
) -> ApiResult<Vec<JobDto>> {
    state
        .jobs
        .jobs()
        .map(|mut jobs| {
            jobs.sort_by(
                |left, right| match (left.state.is_active(), right.state.is_active()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    (true, true) => left.queue_order.cmp(&right.queue_order),
                    (false, false) => right.finished_unix_time_ms.cmp(&left.finished_unix_time_ms),
                },
            );
            Json(
                jobs.into_iter()
                    .take(params.limit.unwrap_or(100).clamp(1, 500))
                    .map(|job| {
                        let mut dto = JobDto::from(job);
                        dto.configuration_snapshot = None;
                        dto.logs.clear();
                        dto
                    })
                    .collect(),
            )
        })
        .map_err(api_error)
}
async fn get_job(State(state): State<AdminState>, Path(id): Path<String>) -> ApiResult<JobDto> {
    state
        .jobs
        .inspect(&id)
        .map(JobDto::from)
        .map(Json)
        .map_err(api_error)
}
async fn cancel_job(State(state): State<AdminState>, Path(id): Path<String>) -> ApiResult<JobDto> {
    state
        .jobs
        .cancel(&id)
        .and_then(|_| state.jobs.inspect(&id))
        .map(JobDto::from)
        .map(Json)
        .map_err(api_error)
}
async fn retry_job(State(state): State<AdminState>, Path(id): Path<String>) -> ApiResult<JobDto> {
    state
        .jobs
        .retry(&id)
        .and_then(|_| state.jobs.inspect(&id))
        .map(JobDto::from)
        .map(Json)
        .map_err(api_error)
}
async fn pause_queue(State(state): State<AdminState>) -> ApiResult<JobCountsDto> {
    state.jobs.pause().map_err(api_error)?;
    state
        .overview()
        .map(|value| Json(value.jobs))
        .map_err(api_error)
}
async fn resume_queue(State(state): State<AdminState>) -> ApiResult<JobCountsDto> {
    state.jobs.resume().map_err(api_error)?;
    state
        .overview()
        .map(|value| Json(value.jobs))
        .map_err(api_error)
}

#[derive(Debug, Default, Deserialize)]
struct HistoryParams {
    text: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
}
#[derive(Serialize)]
struct HistoryDto {
    id: String,
    job_id: String,
    status: String,
    renderer_version: String,
    execution_backend: String,
    profile_name: Option<String>,
    completed_unix_time_ms: u128,
    performance_record_id: Option<String>,
    diagnostic_record_id: Option<String>,
    output_name: Option<String>,
    error: Option<String>,
}
impl From<&RenderHistoryRecord> for HistoryDto {
    fn from(record: &RenderHistoryRecord) -> Self {
        Self {
            id: record.id.clone(),
            job_id: record.job_id.clone(),
            status: format!("{:?}", record.status).to_lowercase(),
            renderer_version: record.renderer_version.clone(),
            execution_backend: record.execution_backend.clone(),
            profile_name: record.profile_name.clone(),
            completed_unix_time_ms: record.completed_unix_time_ms,
            performance_record_id: record.performance_record_id.clone(),
            diagnostic_record_id: record.diagnostic_record_id.clone(),
            output_name: record
                .output
                .path
                .as_deref()
                .and_then(FsPath::file_name)
                .map(|name| name.to_string_lossy().into_owned()),
            error: record.error.as_deref().map(redact),
        }
    }
}
async fn history(
    State(state): State<AdminState>,
    Query(params): Query<HistoryParams>,
) -> ApiResult<Vec<HistoryDto>> {
    let status = match params.status.as_deref() {
        Some("completed") => Some(HistoryStatus::Completed),
        Some("failed") => Some(HistoryStatus::Failed),
        Some("cancelled") => Some(HistoryStatus::Cancelled),
        Some(_) => return Err(api_error("status must be completed, failed, or cancelled")),
        None => None,
    };
    let page = state.history.list(&HistoryQuery {
        text: params.text,
        status,
        limit: Some(params.limit.unwrap_or(100).clamp(1, 500)),
        ..Default::default()
    });
    Ok(Json(page.entries.iter().map(HistoryDto::from).collect()))
}

#[derive(Serialize)]
struct PerformanceDto {
    id: String,
    execution_time_ms: f64,
    queue_wait_time_ms: Option<f64>,
    total_user_latency_ms: Option<f64>,
    effective_backend_status: String,
    execution_mode: String,
    backend: String,
    renderer_version: String,
    resolution: [u32; 2],
    fps: u32,
    frames: u64,
    cache_state: Option<String>,
    stages: Vec<StageDto>,
    environment: PerformanceEnvironmentDto,
}
#[derive(Serialize)]
struct StageDto {
    name: String,
    duration_ms: f64,
    samples: u64,
    semantics: String,
}
#[derive(Serialize)]
struct PerformanceEnvironmentDto {
    operating_system: String,
    architecture: String,
    cpu_identity: Option<String>,
    gpu_identity: Option<String>,
}
impl From<&RenderPerformanceRecord> for PerformanceDto {
    fn from(record: &RenderPerformanceRecord) -> Self {
        Self {
            id: record.id.clone(),
            execution_time_ms: record.execution_time_ms,
            queue_wait_time_ms: record.queue_wait_time_ms,
            total_user_latency_ms: record.total_user_latency_ms,
            effective_backend_status: record.effective_backend_status.clone(),
            execution_mode: record.workload.execution_mode.clone(),
            backend: record.workload.backend.clone(),
            renderer_version: record.environment.renderer_version.clone(),
            resolution: record.workload.resolution,
            fps: record.workload.fps,
            frames: record.frames,
            cache_state: record.gcc_cache_state.clone(),
            stages: record
                .stage_timings
                .iter()
                .map(|stage| StageDto {
                    name: stage.name.clone(),
                    duration_ms: stage.duration_ms,
                    samples: stage.samples,
                    semantics: format!("{:?}", stage.semantics).to_lowercase(),
                })
                .collect(),
            environment: PerformanceEnvironmentDto {
                operating_system: record.environment.operating_system.clone(),
                architecture: record.environment.architecture.clone(),
                cpu_identity: record.environment.cpu_identity.clone(),
                gpu_identity: record.environment.gpu_identity.clone(),
            },
        }
    }
}
async fn performance(
    State(state): State<AdminState>,
    Query(params): Query<LimitParams>,
) -> ApiResult<Vec<PerformanceDto>> {
    state
        .performance
        .list(params.limit.unwrap_or(100).clamp(1, 500))
        .map(|items| Json(items.iter().map(PerformanceDto::from).collect()))
        .map_err(api_error)
}
#[derive(Debug, Default, Deserialize)]
struct LimitParams {
    limit: Option<usize>,
}
#[derive(Debug, Deserialize)]
struct CompareParams {
    left: String,
    right: String,
}
async fn compare_performance(
    State(state): State<AdminState>,
    Query(params): Query<CompareParams>,
) -> ApiResult<crate::performance::PerformanceComparison> {
    let left = state.performance.get(&params.left).map_err(api_error)?;
    let right = state.performance.get(&params.right).map_err(api_error)?;
    Ok(Json(crate::performance::compare_records(&left, &right)))
}

async fn cache_inventory() -> ApiResult<crate::cache_manager::CacheInventory> {
    crate::cache_manager::inventory()
        .map(Json)
        .map_err(api_error)
}
async fn inspect_cache(Path(id): Path<String>) -> ApiResult<crate::cache_manager::CacheEntry> {
    crate::cache_manager::inspect(&id)
        .map(Json)
        .map_err(api_error)
}
#[derive(Deserialize)]
struct ConfirmCacheRemoval {
    confirmation: String,
}
async fn remove_cache(
    Path(id): Path<String>,
    Json(body): Json<ConfirmCacheRemoval>,
) -> ApiResult<crate::cache_manager::CacheOperationReport> {
    if body.confirmation != "remove-cache-entry" {
        return Err(api_error(
            "explicit cache-entry removal confirmation is required",
        ));
    }
    crate::cache_manager::remove(&id)
        .map(Json)
        .map_err(api_error)
}
#[derive(Deserialize)]
struct ConfirmCache {
    confirmation: String,
    #[serde(default)]
    include_memory: bool,
}
async fn clear_cache(
    Json(body): Json<ConfirmCache>,
) -> ApiResult<crate::cache_manager::CacheOperationReport> {
    if body.confirmation != "clear-generated-cache" {
        return Err(api_error("explicit cache-clear confirmation is required"));
    }
    crate::cache_manager::clear(body.include_memory)
        .map(Json)
        .map_err(api_error)
}
async fn validate_cache() -> ApiResult<Vec<crate::cache_manager::CacheEntry>> {
    crate::cache_manager::validate_all()
        .map(Json)
        .map_err(api_error)
}

#[derive(Serialize)]
struct ProfileDto {
    id: String,
    name: String,
    active: bool,
    updated_unix_time_ms: u128,
    resolution: [u32; 2],
    fps: u32,
    execution_mode: String,
}
async fn profiles(State(state): State<AdminState>) -> ApiResult<Vec<ProfileDto>> {
    let active = state
        .profiles
        .active()
        .map_err(api_error)?
        .map(|profile| profile.id);
    state
        .profiles
        .list()
        .map(|profiles| {
            Json(
                profiles
                    .into_iter()
                    .map(|profile| ProfileDto {
                        active: active.as_deref() == Some(profile.id.as_str()),
                        id: profile.id,
                        name: profile.name,
                        updated_unix_time_ms: profile.updated_unix_time_ms,
                        resolution: [profile.config.width, profile.config.height],
                        fps: profile.config.fps,
                        execution_mode: format!("{:?}", profile.config.execution_mode)
                            .to_lowercase(),
                    })
                    .collect(),
            )
        })
        .map_err(api_error)
}
#[derive(Deserialize)]
struct SelectProfile {
    selector: String,
}
async fn select_profile(
    State(state): State<AdminState>,
    Json(body): Json<SelectProfile>,
) -> ApiResult<Vec<ProfileDto>> {
    if body.selector == "current" {
        state.profiles.select_current().map_err(api_error)?;
    } else {
        state.profiles.select(&body.selector).map_err(api_error)?;
    }
    profiles(State(state)).await
}
#[derive(Deserialize)]
struct RenameProfile {
    name: String,
}
async fn rename_profile(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<RenameProfile>,
) -> ApiResult<ProfileDto> {
    let active = state
        .profiles
        .active()
        .map_err(api_error)?
        .is_some_and(|profile| profile.id == id);
    state
        .profiles
        .rename(&id, &body.name)
        .map(|profile| {
            Json(ProfileDto {
                id: profile.id,
                name: profile.name,
                active,
                updated_unix_time_ms: profile.updated_unix_time_ms,
                resolution: [profile.config.width, profile.config.height],
                fps: profile.config.fps,
                execution_mode: format!("{:?}", profile.config.execution_mode).to_lowercase(),
            })
        })
        .map_err(api_error)
}
#[derive(Deserialize)]
struct ConfirmDelete {
    confirmation: String,
}
async fn delete_profile(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(body): Json<ConfirmDelete>,
) -> ApiResult<serde_json::Value> {
    if body.confirmation != "delete-profile" {
        return Err(api_error(
            "explicit profile deletion confirmation is required",
        ));
    }
    state
        .profiles
        .delete(&id)
        .map(|_| Json(serde_json::json!({"deleted": true})))
        .map_err(api_error)
}

#[derive(Serialize)]
struct DiagnosticDto {
    id: String,
    generated_unix_time_ms: u128,
    outcome: String,
    effective_backend: String,
    findings: Vec<FindingDto>,
    performance_record_id: Option<String>,
}
#[derive(Serialize)]
struct FindingDto {
    kind: String,
    code: String,
    summary: String,
}
impl From<RenderDiagnosticRecord> for DiagnosticDto {
    fn from(record: RenderDiagnosticRecord) -> Self {
        Self {
            id: record.id,
            generated_unix_time_ms: record.generated_unix_time_ms,
            outcome: format!("{:?}", record.outcome).to_lowercase(),
            effective_backend: record.effective_backend,
            findings: record
                .findings
                .into_iter()
                .map(|finding| FindingDto {
                    kind: format!("{:?}", finding.kind).to_lowercase(),
                    code: finding.code,
                    summary: redact(&finding.summary),
                })
                .collect(),
            performance_record_id: record.performance_record_id,
        }
    }
}
async fn diagnostics(
    State(state): State<AdminState>,
    Query(params): Query<LimitParams>,
) -> ApiResult<Vec<DiagnosticDto>> {
    let history = state.history.list(&HistoryQuery {
        limit: Some(params.limit.unwrap_or(50).clamp(1, 500)),
        ..Default::default()
    });
    let mut reports = Vec::new();
    for record in history
        .entries
        .into_iter()
        .filter_map(|entry| entry.diagnostic_record_id)
    {
        if let Ok(report) = state.diagnostics.get(&record) {
            reports.push(DiagnosticDto::from(report));
        }
        if reports.len() >= params.limit.unwrap_or(50).clamp(1, 500) {
            break;
        }
    }
    Ok(Json(reports))
}
#[derive(Debug, Default, Deserialize)]
struct LogParams {
    text: Option<String>,
    level: Option<String>,
    limit: Option<usize>,
}
#[derive(Serialize)]
struct LogDto {
    source: &'static str,
    job_id: Option<String>,
    state: Option<String>,
    level: Option<String>,
    target: Option<String>,
    unix_time_ms: u128,
    message: String,
}

#[derive(Serialize)]
struct LogsResponse {
    application_capture_active: bool,
    application_retention_limit: usize,
    entries: Vec<LogDto>,
}

async fn logs(
    State(state): State<AdminState>,
    Query(params): Query<LogParams>,
) -> ApiResult<LogsResponse> {
    let text = params.text.unwrap_or_default().to_lowercase();
    let requested_level = params.level.as_deref().map(str::to_ascii_lowercase);
    if requested_level
        .as_deref()
        .is_some_and(|level| !matches!(level, "error" | "warn" | "info"))
    {
        return Err(api_error("level must be error, warn, or info"));
    }
    let mut logs = Vec::new();
    for entry in state.process_logs.recent(2_000) {
        let matches_level = requested_level
            .as_deref()
            .is_none_or(|level| entry.level == level);
        let matches_text = text.is_empty()
            || entry.message.to_lowercase().contains(&text)
            || entry.target.to_lowercase().contains(&text);
        if matches_level && matches_text {
            logs.push(LogDto {
                source: "application",
                job_id: None,
                state: None,
                level: Some(entry.level),
                target: Some(entry.target),
                unix_time_ms: entry.unix_time_ms,
                message: redact(&entry.message),
            });
        }
    }
    let all_jobs = state.jobs.jobs().map_err(api_error)?;
    let active = all_jobs.iter().filter(|job| job.state.is_active()).cloned();
    let recent_finished = all_jobs
        .iter()
        .filter(|job| !job.state.is_active())
        .rev()
        .take(100)
        .cloned();
    for job in active.chain(recent_finished) {
        let job_id = job.id.clone();
        let status = job_state_name(job.state);
        for entry in job.logs {
            if requested_level.is_none()
                && (text.is_empty() || entry.message.to_lowercase().contains(&text))
            {
                logs.push(LogDto {
                    source: "job",
                    job_id: Some(job_id.clone()),
                    state: Some(status.clone()),
                    level: None,
                    target: None,
                    unix_time_ms: entry.unix_time_ms,
                    message: redact(&entry.message),
                });
            }
        }
    }
    logs.sort_by_key(|entry| entry.unix_time_ms);
    let limit = params.limit.unwrap_or(500).clamp(1, 500);
    if logs.len() > limit {
        logs.drain(..logs.len() - limit);
    }
    Ok(Json(LogsResponse {
        application_capture_active: state.process_log_capture_active,
        application_retention_limit: crate::app_log::PROCESS_LOG_CAPACITY,
        entries: logs,
    }))
}

async fn admin_config(State(state): State<AdminState>) -> Json<AdminConfigView> {
    let config = state
        .config
        .read()
        .expect("admin configuration lock poisoned");
    Json(AdminConfigView::from_config(&config))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAdminConfig {
    enabled: bool,
    bind_address: String,
    port: u16,
}
async fn update_admin_config(
    State(state): State<AdminState>,
    Json(body): Json<UpdateAdminConfig>,
) -> ApiResult<AdminConfigView> {
    let next = AdminServerConfig {
        schema_version: ADMIN_CONFIG_VERSION,
        enabled: body.enabled,
        bind_address: body.bind_address,
        port: body.port,
    };
    next.validate().map_err(api_error)?;
    validate_startup_security(&next).map_err(api_error)?;
    next.save_default().map_err(api_error)?;
    *state
        .config
        .write()
        .map_err(|_| api_error("admin configuration lock poisoned"))? = next.clone();
    Ok(Json(AdminConfigView::from_config(&next)))
}

async fn events(
    State(state): State<AdminState>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, std::convert::Infallible>>> {
    let stream = tokio_stream::iter(std::iter::repeat(())).then(move |_| {
        let state = state.clone();
        async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let data = state
                .overview()
                .and_then(|value| serde_json::to_string(&value).map_err(Into::into))
                .unwrap_or_else(|error| {
                    serde_json::json!({"error": redact(&error.to_string())}).to_string()
                });
            Ok::<Event, std::convert::Infallible>(Event::default().data(data))
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn redact(value: &str) -> String {
    crate::app_log::redact(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_cpu_rate_is_normalized_and_bounded() {
        assert_eq!(
            percent_delta(0, 5_000_000, Duration::from_secs(1), 1),
            Some(50.0)
        );
        assert_eq!(
            percent_delta(0, 5_000_000, Duration::from_secs(1), 2),
            Some(25.0)
        );
        assert_eq!(
            percent_delta(0, 50_000_000, Duration::from_secs(1), 1),
            Some(100.0)
        );
        assert_eq!(percent_delta(10, 20, Duration::ZERO, 1), None);
        assert_eq!(percent_delta(10, 20, Duration::from_secs(1), 0), None);
    }

    #[test]
    fn system_busy_percentage_uses_idle_kernel_user_deltas() {
        assert_eq!(system_cpu_percent(25, 80, 20), Some(75.0));
        assert_eq!(system_cpu_percent(150, 80, 20), Some(0.0));
        assert_eq!(system_cpu_percent(0, 0, 0), None);
    }

    #[test]
    fn resource_sampler_reuses_the_recent_snapshot() {
        let sampler = ResourceSampler::default();
        let cached = ResourceDto {
            process_cpu_percent: Some(12.5),
            system_cpu_percent: Some(25.0),
            process_memory_bytes: Some(u64::MAX),
            process_io_bytes_per_second: Some(10.0),
            gpu_utilization_percent: None,
            vram_bytes: None,
            disk_io_bytes_per_second: None,
            availability: "cached sample".into(),
        };
        {
            let mut state = sampler.state.lock().unwrap();
            state.sampled_at = Some(Instant::now());
            state.last_view = Some(cached.clone());
        }

        let view = sampler.sample();
        assert_eq!(view.availability, cached.availability);
        assert_eq!(view.process_memory_bytes, cached.process_memory_bytes);
        assert_eq!(view.system_cpu_percent, cached.system_cpu_percent);
    }

    #[cfg(windows)]
    #[test]
    fn windows_resource_sampler_reads_own_process_memory() {
        let sample = platform_resource_sample();
        assert!(sample.process_memory_bytes.is_some());
        assert!(sample.sampled_at.elapsed() < Duration::from_secs(1));
    }
    use axum::http::Request;
    use std::io::{Read, Write};
    use tower::ServiceExt;

    #[test]
    fn admin_config_defaults_to_disabled_loopback_and_accepts_ephemeral_port() {
        let config = AdminServerConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.validate().unwrap(), "127.0.0.1:0".parse().unwrap());
        assert!(config.is_loopback().unwrap());
    }

    #[test]
    fn admin_config_rejects_hostnames_and_unknown_fields() {
        let mut config = AdminServerConfig::default();
        config.bind_address = "example.com".into();
        assert!(config.validate().is_err());
        assert!(serde_json::from_str::<AdminServerConfig>(r#"{"schema_version":1,"enabled":false,"bind_address":"127.0.0.1","port":0,"token":"secret"}"#).is_err());
    }

    #[test]
    fn config_save_and_reload_is_versioned() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admin.json");
        let mut config = AdminServerConfig::default();
        config.enabled = true;
        config.port = 0;
        config.save_at(&path).unwrap();
        assert_eq!(AdminServerConfig::load_at(path).unwrap(), config);
    }

    #[test]
    fn rejected_admin_config_save_preserves_previous_valid_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admin.json");
        let original = AdminServerConfig::default();
        original.save_at(&path).unwrap();
        let mut invalid = original.clone();
        invalid.schema_version = ADMIN_CONFIG_VERSION + 1;
        assert!(invalid.save_at(&path).is_err());
        assert_eq!(AdminServerConfig::load_at(path).unwrap(), original);
    }

    #[test]
    fn bind_failure_does_not_choose_an_alternate_port() {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let jobs = Arc::new(
            RenderJobManager::open_at(
                directory.path().join("jobs.json"),
                crate::job_manager::JobManagerConfig::default(),
            )
            .unwrap(),
        );
        let config = AdminServerConfig {
            enabled: true,
            bind_address: address.ip().to_string(),
            port: address.port(),
            ..Default::default()
        };
        assert!(start(&config, jobs).is_err());
        assert!(listener.local_addr().is_ok());
    }

    #[test]
    fn constant_time_token_compare_handles_equal_and_different_lengths() {
        assert!(constant_time_eq(b"operator-token", b"operator-token"));
        assert!(!constant_time_eq(b"operator-token", b"operator-toke"));
        assert!(!constant_time_eq(b"operator-token", b"other-longer-token"));
    }

    #[test]
    fn loopback_host_guard_rejects_dns_rebinding_hosts_and_wrong_ports() {
        assert!(valid_loopback_host("localhost:8080", 8080));
        assert!(valid_loopback_host("127.0.0.1:8080", 8080));
        assert!(valid_loopback_host("[::1]:8080", 8080));
        assert!(!valid_loopback_host("renderer.attacker.test:8080", 8080));
        assert!(!valid_loopback_host("localhost:8081", 8080));
        assert!(!valid_loopback_host("localhost", 8080));
    }

    #[test]
    fn redactor_hides_short_nonempty_secret_values() {
        assert_eq!(
            crate::app_log::redact_with_secrets("authorization value: abc", &["abc".into()]),
            "authorization value: [redacted]"
        );
    }

    #[tokio::test]
    async fn logs_api_exposes_filtered_process_events_without_losing_job_event_contract() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = Arc::new(
            RenderJobManager::open_at(
                directory.path().join("jobs.json"),
                crate::job_manager::JobManagerConfig::default(),
            )
            .unwrap(),
        );
        let state = AdminState::open(jobs, AdminServerConfig::default()).unwrap();
        state
            .process_logs
            .record("warn", "renderer::fixture", "media decoder warning");
        state
            .process_logs
            .record("info", "renderer::fixture", "queue ready");
        let app = build_router(
            state,
            AuthState(Some("a-long-test-operator-token".into())),
            LoopbackHostGuard {
                enabled: true,
                port: 80,
            },
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/logs?level=warn&text=decoder")
                    .header(header::HOST, "localhost")
                    .header(header::AUTHORIZATION, "Bearer a-long-test-operator-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let response: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let records = response["entries"].as_array().unwrap();
        assert_eq!(response["application_capture_active"], false);
        assert_eq!(response["application_retention_limit"], 2_000);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["source"], "application");
        assert_eq!(records[0]["level"], "warn");
        assert_eq!(records[0]["target"], "renderer::fixture");
        assert_eq!(records[0]["message"], "media decoder warning");

        let invalid_level = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/logs?level=verbose")
                    .header(header::HOST, "localhost")
                    .header(header::AUTHORIZATION, "Bearer a-long-test-operator-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_level.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn remote_binding_requires_strong_token_and_both_tls_files() {
        let directory = tempfile::tempdir().unwrap();
        let cert = directory.path().join("cert.pem");
        let key = directory.path().join("key.pem");
        std::fs::write(&cert, "certificate bytes").unwrap();
        std::fs::write(&key, "private key bytes").unwrap();
        assert!(validate_remote_prerequisites(None, Some(&cert), Some(&key)).is_err());
        assert!(validate_remote_prerequisites(Some("short"), Some(&cert), Some(&key)).is_err());
        assert!(validate_remote_prerequisites(Some(&"x".repeat(32)), None, Some(&key)).is_err());
        assert!(
            validate_remote_prerequisites(Some(&"x".repeat(32)), Some(&cert), Some(&key)).is_ok()
        );
    }

    #[test]
    fn local_server_exposes_health_overview_and_embedded_frontend() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = Arc::new(
            RenderJobManager::open_at(
                directory.path().join("jobs.json"),
                crate::job_manager::JobManagerConfig::default(),
            )
            .unwrap(),
        );
        let config = AdminServerConfig {
            enabled: true,
            ..Default::default()
        };
        let server = start(&config, jobs).unwrap().unwrap();
        assert!(server.local_addr.ip().is_loopback());
        assert!(server.is_running());

        let local_host = format!("localhost:{}", server.local_addr.port());
        let get = |path: &str, host: &str| {
            let mut socket = std::net::TcpStream::connect(server.local_addr).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                socket,
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            socket.read_to_string(&mut response).unwrap();
            response
        };

        let health = get("/api/v1/health", &local_host);
        assert!(health.starts_with("HTTP/1.1 200"));
        assert!(health.contains("\"status\":\"ok\""));
        let overview = get("/api/v1/overview", &local_host);
        assert!(overview.starts_with("HTTP/1.1 200"));
        assert!(overview.contains("\"process_cpu_percent\":null"));
        assert!(overview.contains("\"system_cpu_percent\":"));
        assert!(overview.contains("\"process_io_bytes_per_second\":"));
        let frontend = get("/", &local_host);
        assert!(frontend.starts_with("HTTP/1.1 200"));
        assert!(frontend.contains("/admin.js"));
        let javascript = get("/admin.js", &local_host);
        assert!(javascript.starts_with("HTTP/1.1 200"));
        assert!(javascript.contains("Sono-Renderer"));
        assert!(javascript.contains("Process I/O"));
        let rebinding = get(
            "/api/v1/overview",
            &format!("renderer.attacker.test:{}", server.local_addr.port()),
        );
        assert!(rebinding.starts_with("HTTP/1.1 400"));
    }

    #[tokio::test]
    async fn protected_routes_require_token_and_apply_browser_security_headers() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = Arc::new(
            RenderJobManager::open_at(
                directory.path().join("jobs.json"),
                crate::job_manager::JobManagerConfig::default(),
            )
            .unwrap(),
        );
        let state = AdminState::open(jobs.clone(), AdminServerConfig::default()).unwrap();
        let app = build_router(
            state,
            AuthState(Some("a-long-test-operator-token".into())),
            LoopbackHostGuard {
                enabled: true,
                port: 80,
            },
        );
        let anonymous = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin-config")
                    .header(header::HOST, "localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
        let authorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin-config")
                    .header(header::HOST, "localhost")
                    .header(header::AUTHORIZATION, "Bearer a-long-test-operator-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authorized.status(), StatusCode::OK);
        assert_eq!(authorized.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert!(authorized
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY));

        for path in [
            "/api/v1/events",
            "/api/v1/jobs?limit=10",
            "/api/v1/history?limit=10",
            "/api/v1/performance?limit=10",
            "/api/v1/cache",
            "/api/v1/profiles",
            "/api/v1/diagnostics?limit=10",
            "/api/v1/logs",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header::HOST, "localhost")
                        .header(header::AUTHORIZATION, "Bearer a-long-test-operator-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }

        for (path, expected_paused) in [
            ("/api/v1/queue/pause", true),
            ("/api/v1/queue/resume", false),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header::HOST, "localhost")
                        .header(header::AUTHORIZATION, "Bearer a-long-test-operator-token")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(jobs.is_paused().unwrap(), expected_paused);
        }
    }
}
