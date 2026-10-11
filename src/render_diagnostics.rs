//! Structured explanations for completed and failed render jobs.
//!
//! Diagnostics consume the authoritative performance record. They deliberately
//! do not start clocks or create a second timing model.

use crate::{
    performance::{RenderPerformanceRecord, StageSemantics},
    render::RenderConfig,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const DEFAULT_DIAGNOSTIC_RETENTION: usize = 500;

pub const DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOutcome {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    MeasuredFact,
    ObservedEvent,
    DerivedConclusion,
    Recommendation,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticFinding {
    pub kind: FindingKind,
    pub code: String,
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticEvent {
    pub name: String,
    pub phase: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticWorkload {
    pub requested_backend: String,
    pub requested_execution_mode: String,
    pub resolution: [u32; 2],
    pub fps: u32,
    pub start_time_micros: Option<i64>,
    pub duration_micros: Option<i64>,
    pub effective_config_fingerprint: Option<String>,
    pub engine_fingerprint: Option<String>,
    pub level_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticError {
    pub subsystem: String,
    pub phase: Option<String>,
    pub failure_kind: Option<String>,
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticOutputValidation {
    pub ffprobe_succeeded: bool,
    pub submitted_frames: u64,
    pub video_streams: usize,
    pub audio_streams: usize,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<String>,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticMediaFacts {
    pub ffmpeg_source: String,
    pub output_validation: DiagnosticOutputValidation,
    pub deterministic_frame_hashes_match: Option<bool>,
    pub gpu_identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CriticalPathAssessment {
    /// Largest disjoint wall-span phase, not a dependency-graph critical path.
    pub largest_exclusive_phase: Option<String>,
    pub phase_wall_ms: Option<f64>,
    pub execution_share_percent: Option<f64>,
    pub exact_dependency_path_available: bool,
    pub limitation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderDiagnosticRecord {
    pub schema_version: u32,
    pub id: String,
    pub generated_unix_time_ms: u128,
    pub outcome: DiagnosticOutcome,
    pub job_id: Option<String>,
    pub performance_record_id: Option<String>,
    pub workload: Option<DiagnosticWorkload>,
    pub effective_backend: String,
    pub cache_state: Option<String>,
    #[serde(default)]
    pub media: Option<DiagnosticMediaFacts>,
    pub lifecycle_events: Vec<DiagnosticEvent>,
    pub critical_path: CriticalPathAssessment,
    pub findings: Vec<DiagnosticFinding>,
    pub error: Option<DiagnosticError>,
    /// A sanitized snapshot of the single authoritative performance record.
    pub performance: Option<RenderPerformanceRecord>,
}

impl RenderDiagnosticRecord {
    /// Attach compatibility warnings emitted by the authoritative WatchVm for
    /// this render attempt. These are observed events, not analyzer guesses.
    pub(crate) fn append_watch_compatibility_warnings(
        &mut self,
        warnings: impl IntoIterator<Item = String>,
    ) {
        for summary in warnings {
            if self.findings.iter().any(|finding| {
                finding.code == "watch_compatibility_approximation" && finding.summary == summary
            }) {
                continue;
            }
            self.findings.push(finding(
                FindingKind::ObservedEvent,
                "watch_compatibility_approximation",
                summary,
                vec![
                    "Observed during authoritative WatchVm execution".into(),
                    "Exact Sonolus client parity is unverified".into(),
                ],
            ));
        }
    }

    /// Attach recoverable particle-resource issues observed while loading the
    /// render inputs. These findings are separate from Watch compatibility:
    /// an invalid effect was omitted, while other effects remain renderable.
    pub(crate) fn append_particle_resource_warnings(
        &mut self,
        warnings: impl IntoIterator<Item = String>,
    ) {
        for summary in warnings {
            if self.findings.iter().any(|finding| {
                finding.code == "particle_effect_skipped" && finding.summary == summary
            }) {
                continue;
            }
            self.findings.push(finding(
                FindingKind::ObservedEvent,
                "particle_effect_skipped",
                summary,
                vec![
                    "The invalid effect was skipped while loading the particle resource".into(),
                    "Other validated effects from the same resource remain available".into(),
                ],
            ));
        }
    }

    pub fn validate_schema_version(&self) -> Result<()> {
        if self.schema_version != DIAGNOSTIC_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported render diagnostic schema {} (supported {})",
                self.schema_version,
                DIAGNOSTIC_SCHEMA_VERSION
            );
        }
        Ok(())
    }

    pub fn from_performance(record: &RenderPerformanceRecord) -> Self {
        let mut performance = record.clone();
        performance.effective_backend_status = sanitize_text(&performance.effective_backend_status);
        let workload = workload_from_record(record);
        let effective_backend = classify_effective_backend(record);
        let mut findings = Vec::new();
        let mut lifecycle_events = Vec::new();

        findings.push(finding(
            FindingKind::MeasuredFact,
            "execution_duration",
            format!(
                "Renderer execution took {:.2} ms.",
                record.execution_time_ms
            ),
            vec!["RenderPerformanceRecord.execution_time_ms".into()],
        ));
        if let Some(queue_wait) = record.queue_wait_time_ms {
            findings.push(finding(
                FindingKind::MeasuredFact,
                "queue_wait",
                format!(
                    "The job waited {:.2} ms in the queue before execution.",
                    queue_wait
                ),
                vec!["RenderPerformanceRecord.queue_wait_time_ms".into()],
            ));
        }
        findings.push(finding(
            FindingKind::ObservedEvent,
            "effective_backend",
            format!(
                "Observed effective backend: {effective_backend} ({}).",
                sanitize_text(&record.effective_backend_status)
            ),
            vec!["RenderPerformanceRecord.effective_backend_status".into()],
        ));
        if record.gcc_cache_state.as_deref() == Some("fallback")
            || record
                .effective_backend_status
                .to_ascii_lowercase()
                .contains("falling back")
            || record
                .effective_backend_status
                .to_ascii_lowercase()
                .contains("vm fallback")
        {
            findings.push(finding(
                FindingKind::ObservedEvent,
                "backend_fallback",
                "Sono-GCC did not provide the effective Watch backend for this render; Watch execution continued through Sono VM fallback, so this run is not a native GCC performance sample.".into(),
                vec![sanitize_text(&record.effective_backend_status)],
            ));
        }
        if record
            .effective_backend_status
            .to_ascii_lowercase()
            .contains("cache persistence unavailable")
        {
            findings.push(finding(
                FindingKind::ObservedEvent,
                "cache_persistence_warning",
                "The renderer reported that persistent Sono-GCC cache storage was unavailable for this run.".into(),
                vec![sanitize_text(&record.effective_backend_status)],
            ));
        }

        let mut exclusive: Vec<_> = record
            .stage_timings
            .iter()
            .filter(|stage| stage.semantics == StageSemantics::ExclusiveWallSpan)
            .collect();
        exclusive.sort_by(|left, right| right.duration_ms.total_cmp(&left.duration_ms));
        let largest = exclusive.first();
        let (largest_name, largest_ms, share) = if let Some(stage) = largest {
            let percent = (record.execution_time_ms > 0.0)
                .then(|| stage.duration_ms / record.execution_time_ms * 100.0);
            findings.push(finding(
                FindingKind::MeasuredFact,
                "largest_exclusive_phase",
                format!(
                    "{} was the largest disjoint phase at {:.2} ms.",
                    stage.name, stage.duration_ms
                ),
                vec!["StageSemantics::ExclusiveWallSpan".into()],
            ));
            findings.push(finding(
                FindingKind::DerivedConclusion,
                "phase_dominance",
                format!(
                    "{} is the leading measured phase for this render.",
                    stage.name
                ),
                vec![format!(
                    "{:.1}% of execution wall time",
                    percent.unwrap_or(0.0)
                )],
            ));
            lifecycle_events.push(DiagnosticEvent {
                name: "phase_completed".into(),
                phase: Some(sanitize_text(&stage.name)),
                detail: Some(format!("{:.2} ms exclusive wall span", stage.duration_ms)),
            });
            (
                Some(sanitize_text(&stage.name)),
                Some(stage.duration_ms),
                percent,
            )
        } else {
            findings.push(finding(
                FindingKind::Unavailable,
                "phase_timings",
                "No disjoint phase timings were recorded for this render.".into(),
                vec![],
            ));
            (None, None, None)
        };

        for stage in exclusive.iter().skip(1) {
            lifecycle_events.push(DiagnosticEvent {
                name: "phase_completed".into(),
                phase: Some(sanitize_text(&stage.name)),
                detail: Some(format!("{:.2} ms exclusive wall span", stage.duration_ms)),
            });
        }
        if !record
            .stage_timings
            .iter()
            .any(|stage| stage.semantics == StageSemantics::OverlappingAttribution)
        {
            findings.push(finding(
                FindingKind::Unavailable,
                "detailed_attribution",
                "Overlapping per-frame attribution was not collected.".into(),
                vec![],
            ));
        }
        let detailed_substages: Vec<_> = record
            .stage_timings
            .iter()
            .filter(|stage| {
                stage.semantics == StageSemantics::OverlappingAttribution
                    && [
                        "gpu", "readback", "wait/map", "ffmpeg", "encode", "mux", "audio",
                    ]
                    .iter()
                    .any(|needle| stage.name.to_ascii_lowercase().contains(needle))
            })
            .collect();
        if detailed_substages.is_empty() {
            findings.push(finding(
                FindingKind::Unavailable,
                "gpu_encoder_substages",
                "No detailed GPU, readback, encoder, or audio substage attribution was recorded."
                    .into(),
                vec![],
            ));
        } else {
            for stage in detailed_substages {
                findings.push(finding(
                    FindingKind::MeasuredFact,
                    "gpu_encoder_substage",
                    format!(
                        "{} attribution totaled {:.2} ms across {} sample(s).",
                        stage.name, stage.duration_ms, stage.samples
                    ),
                    vec!["Overlapping attribution; not additive to exclusive phase spans.".into()],
                ));
            }
        }

        if let Some(pipeline) = &record.pipeline_overlap {
            findings.push(finding(
                FindingKind::MeasuredFact,
                "pipeline_overlap",
                format!(
                    "Pipeline wall span was {:.2} ms; attributed Watch, render, output, producer-blocked, and consumer-waiting values are available.",
                    pipeline.wall_span_ms
                ),
                vec![
                    format!("producer blocked: {:.2} ms", pipeline.producer_blocked_ms),
                    format!("consumer waiting: {:.2} ms", pipeline.consumer_waiting_ms),
                    format!("render lane: {:.2} ms", pipeline.render_lane_ms),
                    format!("output lane: {:.2} ms", pipeline.output_lane_ms),
                    "These attribution spans overlap and must not be summed.".into(),
                ],
            ));
            findings.push(finding(
                FindingKind::Unavailable,
                "pipeline_cause",
                "The dependency-aware critical path and causal source of pipeline waiting are unavailable; lane spans show overlap but do not establish why a lane waited.".into(),
                vec![],
            ));
        } else {
            findings.push(finding(
                FindingKind::Unavailable,
                "pipeline_overlap",
                "Pipeline overlap telemetry was not recorded for this render.".into(),
                vec![],
            ));
        }

        if let Some(gcc) = &record.sono_gcc_compilation {
            findings.push(finding(
                FindingKind::MeasuredFact,
                "compiler_cache_observation",
                format!("Sono-GCC observed cache state: {}.", gcc.cache_state),
                vec![
                    format!("cache lookup {:.2} ms", gcc.cache_lookup_ms),
                    format!("compilation {:.2} ms", gcc.compilation_ms),
                    format!("native load {:.2} ms", gcc.native_load_ms),
                ],
            ));
            if matches!(gcc.cache_state.as_str(), "persistent_hit" | "process_hit") {
                findings.push(finding(
                    FindingKind::ObservedEvent,
                    "compiler_cache_hit",
                    format!("A Sono-GCC {} was observed.", gcc.cache_state),
                    vec![],
                ));
                findings.push(finding(
                    FindingKind::Unavailable,
                    "compiler_time_saved",
                    "Time saved by the cache cannot be calculated without a comparable cold compilation for this workload.".into(),
                    vec![],
                ));
            }
        } else if record.workload.execution_mode == "sono_gcc" {
            findings.push(finding(
                FindingKind::Unavailable,
                "compiler_metrics",
                "Sono-GCC was requested, but native compilation metrics were not available; inspect the effective backend observation for fallback.".into(),
                vec![],
            ));
        }
        findings.push(finding(
            FindingKind::Unavailable,
            "backend_comparison",
            "This single render cannot establish whether VM or Sono-GCC is faster. Use matched repeated benchmark records for that comparison.".into(),
            vec![],
        ));
        findings.push(finding(
            FindingKind::Unavailable,
            "recommendation",
            "No optimization recommendation is emitted without a measured causal bottleneck or a comparable before/after workload.".into(),
            vec![],
        ));
        findings.push(finding(
            FindingKind::Unavailable,
            "output_validation",
            "Container and stream validation details are not part of the performance record snapshot.".into(),
            vec![],
        ));
        lifecycle_events.push(DiagnosticEvent {
            name: "render_completed".into(),
            phase: None,
            detail: Some(format!(
                "{} frames; output validation completed",
                record.frames
            )),
        });

        Self {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            id: format!("diagnostic-{}", record.id),
            generated_unix_time_ms: record.completed_unix_time_ms,
            outcome: DiagnosticOutcome::Completed,
            job_id: record.job_id.clone(),
            performance_record_id: Some(record.id.clone()),
            workload: Some(workload),
            effective_backend,
            cache_state: record.gcc_cache_state.clone(),
            media: None,
            lifecycle_events,
            critical_path: CriticalPathAssessment {
                largest_exclusive_phase: largest_name,
                phase_wall_ms: largest_ms,
                execution_share_percent: share,
                exact_dependency_path_available: false,
                limitation: "The phase spans partition execution wall time. Concurrent lane attribution is reported separately and does not provide a dependency graph.".into(),
            },
            findings,
            error: None,
            performance: Some(performance),
        }
    }

    /// Add successful export facts that are not timings: FFprobe validation,
    /// codec metadata, encoder source, and optional full-frame determinism.
    pub fn from_export(
        record: &RenderPerformanceRecord,
        ffmpeg_source: &str,
        probe: &serde_json::Value,
        deterministic_frame_hashes_match: Option<bool>,
    ) -> Self {
        let mut report = Self::from_performance(record);
        report
            .findings
            .retain(|finding| finding.code != "output_validation");
        let streams = probe
            .get("streams")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let video_streams: Vec<_> = streams
            .iter()
            .filter(|stream| {
                stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("video")
            })
            .collect();
        let audio_streams: Vec<_> = streams
            .iter()
            .filter(|stream| {
                stream.get("codec_type").and_then(serde_json::Value::as_str) == Some("audio")
            })
            .collect();
        let video = video_streams.first().copied();
        let audio = audio_streams.first().copied();
        let validation = DiagnosticOutputValidation {
            ffprobe_succeeded: true,
            submitted_frames: record.frames,
            video_streams: video_streams.len(),
            audio_streams: audio_streams.len(),
            video_codec: video
                .and_then(|stream| stream.get("codec_name"))
                .and_then(serde_json::Value::as_str)
                .map(sanitize_text),
            audio_codec: audio
                .and_then(|stream| stream.get("codec_name"))
                .and_then(serde_json::Value::as_str)
                .map(sanitize_text),
            width: video
                .and_then(|stream| stream.get("width"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            height: video
                .and_then(|stream| stream.get("height"))
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok()),
            frame_rate: video
                .and_then(|stream| stream.get("avg_frame_rate"))
                .and_then(serde_json::Value::as_str)
                .map(sanitize_text),
            duration_seconds: probe
                .get("format")
                .and_then(|format| format.get("duration"))
                .and_then(|value| {
                    value
                        .as_str()
                        .and_then(|value| value.parse().ok())
                        .or_else(|| value.as_f64())
                }),
        };
        report.findings.push(finding(
            FindingKind::MeasuredFact,
            "output_validation",
            "FFprobe completed and the export passed the renderer's output validation.".into(),
            vec![
                format!(
                    "{} video stream(s), {} audio stream(s)",
                    validation.video_streams, validation.audio_streams
                ),
                format!(
                    "video codec {}",
                    validation.video_codec.as_deref().unwrap_or("not reported")
                ),
                format!(
                    "audio codec {}",
                    validation.audio_codec.as_deref().unwrap_or("not reported")
                ),
            ],
        ));
        report.findings.push(finding(
            FindingKind::ObservedEvent,
            "encoder_source",
            format!("Output was produced using the {ffmpeg_source} FFmpeg installation."),
            vec![format!(
                "FFprobe validated {} frame(s)",
                validation.submitted_frames
            )],
        ));
        match deterministic_frame_hashes_match {
            Some(matches) => report.findings.push(finding(
                FindingKind::MeasuredFact,
                "frame_determinism_validation",
                format!(
                    "Full-frame deterministic validation {}.",
                    if matches { "matched" } else { "did not match" }
                ),
                vec![],
            )),
            None => report.findings.push(finding(
                FindingKind::Unavailable,
                "frame_determinism_validation",
                "Full-frame deterministic validation was not requested for this export.".into(),
                vec![],
            )),
        }
        if record.profiling_detail_enabled {
            report.findings.push(finding(
                FindingKind::MeasuredFact,
                "gpu_encoder_attribution",
                "Opt-in detailed per-frame GPU and output attribution was collected; inspect overlapping performance stages for measured substage values.".into(),
                vec!["Per-frame attribution can overlap and is not additive to exclusive phase spans.".into()],
            ));
        } else {
            report.findings.push(finding(
                FindingKind::Unavailable,
                "gpu_encoder_attribution",
                "Detailed GPU submit/wait/readback and encoder substage timings were not collected; enabling deep profiling adds measurement overhead.".into(),
                vec![],
            ));
        }
        report.media = Some(DiagnosticMediaFacts {
            ffmpeg_source: sanitize_text(ffmpeg_source),
            output_validation: validation,
            deterministic_frame_hashes_match,
            gpu_identity: record
                .environment
                .gpu_identity
                .as_deref()
                .map(sanitize_text),
        });
        report
    }

    pub fn from_failure(
        id: impl Into<String>,
        job_id: Option<String>,
        config: &RenderConfig,
        phase: Option<String>,
        failure_kind: Option<String>,
        error_chain: &[String],
        outcome: DiagnosticOutcome,
    ) -> Self {
        let error_chain: Vec<_> = error_chain.iter().map(|item| sanitize_text(item)).collect();
        let mut findings = vec![finding(
            FindingKind::ObservedEvent,
            "render_failure",
            format!(
                "Render ended as {outcome:?} during {}.",
                phase.as_deref().unwrap_or("an unknown phase")
            ),
            error_chain.clone(),
        )];
        findings.push(finding(
            FindingKind::Unavailable,
            "performance_record",
            "No completed-render performance record exists for this attempt; stage durations and backend cost are unavailable.".into(),
            vec![],
        ));
        findings.push(finding(
            FindingKind::Recommendation,
            "next_action",
            "Use the error subsystem, phase, and cause chain above to choose the next diagnostic step; no performance optimization is inferred from a failed attempt.".into(),
            vec![],
        ));
        let chain = if error_chain.is_empty() {
            vec!["Failure detail unavailable.".into()]
        } else {
            error_chain
        };
        let identity = crate::performance::workload_identity(config);
        Self {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            id: id.into(),
            generated_unix_time_ms: crate::performance::unix_time_ms(),
            outcome,
            job_id,
            performance_record_id: None,
            workload: Some(DiagnosticWorkload {
                requested_backend: identity.backend,
                requested_execution_mode: identity.execution_mode,
                resolution: identity.resolution,
                fps: identity.fps,
                start_time_micros: Some(identity.start_time_micros),
                duration_micros: Some(identity.duration_micros),
                effective_config_fingerprint: Some(identity.effective_config_fingerprint),
                engine_fingerprint: Some(identity.engine_fingerprint),
                level_fingerprint: Some(identity.level_fingerprint),
            }),
            effective_backend: "unavailable".into(),
            cache_state: None,
            media: None,
            lifecycle_events: vec![DiagnosticEvent {
                name: format!("render_{:?}", outcome).to_ascii_lowercase(),
                phase: phase.clone(),
                detail: None,
            }],
            critical_path: CriticalPathAssessment {
                largest_exclusive_phase: None,
                phase_wall_ms: None,
                execution_share_percent: None,
                exact_dependency_path_available: false,
                limitation: "The render did not complete, so no authoritative execution phase record is available.".into(),
            },
            findings,
            error: Some(DiagnosticError {
                subsystem: classify_subsystem(&chain),
                phase,
                failure_kind,
                chain,
            }),
            performance: None,
        }
    }
}

fn workload_from_record(record: &RenderPerformanceRecord) -> DiagnosticWorkload {
    let identity = &record.workload;
    DiagnosticWorkload {
        requested_backend: identity.backend.clone(),
        requested_execution_mode: identity.execution_mode.clone(),
        resolution: identity.resolution,
        fps: identity.fps,
        start_time_micros: Some(identity.start_time_micros),
        duration_micros: Some(identity.duration_micros),
        effective_config_fingerprint: Some(identity.effective_config_fingerprint.clone()),
        engine_fingerprint: Some(identity.engine_fingerprint.clone()),
        level_fingerprint: Some(identity.level_fingerprint.clone()),
    }
}

fn classify_effective_backend(record: &RenderPerformanceRecord) -> String {
    if record.gcc_cache_state.as_deref() == Some("fallback")
        || record
            .effective_backend_status
            .to_ascii_lowercase()
            .contains("falling back")
        || record
            .effective_backend_status
            .to_ascii_lowercase()
            .contains("vm fallback")
    {
        "sono_vm_fallback".into()
    } else if record.sono_gcc_compilation.is_some() {
        "sono_gcc_native".into()
    } else if record.workload.execution_mode == "interpreter" {
        "sono_vm".into()
    } else {
        "unknown".into()
    }
}

fn classify_subsystem(chain: &[String]) -> String {
    let text = chain.join(" ").to_ascii_lowercase();
    if text.contains("ffmpeg") || text.contains("ffprobe") || text.contains("mux") {
        "encoder_or_media".into()
    } else if text.contains("watch") || text.contains("sono-gcc") || text.contains("compiler") {
        "watch_or_compiler".into()
    } else if text.contains("gpu") || text.contains("wgpu") || text.contains("adapter") {
        "gpu".into()
    } else if text.contains("audio") || text.contains("sfx") || text.contains("music") {
        "audio".into()
    } else if text.contains("engine") || text.contains("level") || text.contains("resource") {
        "input_or_resource".into()
    } else {
        "renderer_or_external_resource".into()
    }
}

fn finding(
    kind: FindingKind,
    code: &str,
    summary: String,
    evidence: Vec<String>,
) -> DiagnosticFinding {
    DiagnosticFinding {
        kind,
        code: code.into(),
        summary: sanitize_text(&summary),
        evidence: evidence.iter().map(|item| sanitize_text(item)).collect(),
    }
}

/// Remove machine-specific paths and credential-like values before exporting
/// diagnostics. Performance records otherwise contain workload fingerprints,
/// not source paths.
pub fn sanitize_text(input: &str) -> String {
    let path_scrubbed = redact_absolute_paths(input);
    let mut text = redact_credential_values(&path_scrubbed);
    if let Some(index) = text.to_ascii_lowercase().find("bearer ") {
        let value_start = index + "bearer ".len();
        let value_end = text[value_start..]
            .find(char::is_whitespace)
            .map_or(text.len(), |offset| value_start + offset);
        text.replace_range(value_start..value_end, "<redacted>");
    }
    text
}

fn redact_absolute_paths(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < bytes.len() {
        let windows_drive = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/');
        let unc = index + 1 < bytes.len() && bytes[index] == b'\\' && bytes[index + 1] == b'\\';
        let unix_root = bytes[index] == b'/'
            && (index == 0
                || bytes[index - 1].is_ascii_whitespace()
                || b"([=\"'".contains(&bytes[index - 1]));
        if windows_drive || unc || unix_root {
            output.push_str("<path>");
            index += if windows_drive {
                3
            } else if unc {
                2
            } else {
                1
            };
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b'\n' | b'\r' | b'"' | b'\'' | b')' | b']' | b'}' | b',' | b';'
                )
            {
                if bytes[index] == b':' && bytes.get(index + 1).is_some_and(u8::is_ascii_whitespace)
                {
                    break;
                }
                index += 1;
            }
            continue;
        }
        let character = input[index..].chars().next().expect("valid UTF-8 boundary");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

fn redact_credential_values(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let keys = [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "authorization=",
        "api_key:",
        "apikey:",
        "token:",
        "secret:",
        "password:",
        "authorization:",
    ];
    let mut output = input.to_owned();
    let mut spans = Vec::new();
    for key in keys {
        let mut from = 0;
        while let Some(relative) = lower[from..].find(key) {
            let value_start = from + relative + key.len();
            let start = value_start
                + input[value_start..]
                    .len()
                    .saturating_sub(input[value_start..].trim_start().len());
            let end = if key == "authorization:" {
                input[start..]
                    .find(['\n', '\r'])
                    .map_or(input.len(), |offset| start + offset)
            } else {
                input[start..]
                    .find(char::is_whitespace)
                    .map_or(input.len(), |offset| start + offset)
            };
            if end > start {
                spans.push((start, end));
            }
            from = start;
            if from >= lower.len() {
                break;
            }
        }
    }
    spans.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    for (start, end) in spans {
        output.replace_range(start..end, "<redacted>");
    }
    output
}

pub fn save_diagnostic_atomic(
    path: impl AsRef<Path>,
    record: &RenderDiagnosticRecord,
) -> Result<()> {
    let path = path.as_ref();
    if path.exists() {
        anyhow::bail!(
            "refusing to overwrite diagnostic report: {}",
            path.display()
        );
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating diagnostic report directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(record)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".renderer-diagnostic-")
        .suffix(".json.tmp")
        .tempfile_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)
        .map_err(|error| error.error)
        .with_context(|| format!("publishing diagnostic report {}", path.display()))?;
    Ok(())
}

/// Bounded sidecar storage for complete diagnostic reports. Timing values
/// remain owned by PerformanceStore; this store only persists their analysis
/// and non-timing observations such as FFprobe output validation.
#[derive(Debug, Clone)]
pub struct RenderDiagnosticStore {
    root: PathBuf,
    max_records: usize,
}

impl RenderDiagnosticStore {
    pub fn open_default() -> Result<Self> {
        Self::open_at(
            crate::performance::default_store_path().join("diagnostics"),
            DEFAULT_DIAGNOSTIC_RETENTION,
        )
    }

    pub fn open_at(path: impl Into<PathBuf>, max_records: usize) -> Result<Self> {
        let root = path.into();
        fs::create_dir_all(&root)
            .with_context(|| format!("creating render diagnostic store {}", root.display()))?;
        let store = Self {
            root,
            max_records: max_records.clamp(1, DEFAULT_DIAGNOSTIC_RETENTION),
        };
        store.prune()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self, report: &RenderDiagnosticRecord) -> Result<()> {
        report.validate_schema_version()?;
        let path = self.record_path(&report.id)?;
        save_diagnostic_atomic(&path, report)?;
        self.prune()
    }

    pub fn get(&self, id: &str) -> Result<RenderDiagnosticRecord> {
        let path = self.record_path(id)?;
        let bytes = fs::read(&path)
            .with_context(|| format!("reading render diagnostic {}", path.display()))?;
        let report: RenderDiagnosticRecord = serde_json::from_slice(&bytes)
            .with_context(|| format!("decoding render diagnostic {}", path.display()))?;
        report.validate_schema_version()?;
        Ok(report)
    }

    pub fn get_for_performance(&self, record_id: &str) -> Result<RenderDiagnosticRecord> {
        self.get(&format!("diagnostic-{record_id}"))
    }

    fn record_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            anyhow::bail!("invalid render diagnostic identifier");
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    fn prune(&self) -> Result<()> {
        let mut files = fs::read_dir(&self.root)
            .with_context(|| format!("listing render diagnostic store {}", self.root.display()))?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| {
                let modified = entry.metadata().ok()?.modified().ok()?;
                Some((modified, entry.path()))
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, path) in files.into_iter().skip(self.max_records) {
            fs::remove_file(&path)
                .with_context(|| format!("pruning render diagnostic {}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::{
        PerformanceEnvironment, PipelineOverlap, StageTiming, WorkloadIdentity,
    };

    fn performance_record() -> RenderPerformanceRecord {
        RenderPerformanceRecord {
            schema_version: 1,
            id: "perf-test".into(),
            created_unix_time_ms: 1,
            started_unix_time_ms: 1,
            completed_unix_time_ms: 2,
            execution_time_ms: 100.0,
            queue_wait_time_ms: Some(25.0),
            total_user_latency_ms: Some(125.0),
            job_id: Some("job-test".into()),
            benchmark_id: None,
            benchmark_role: None,
            workload: WorkloadIdentity {
                engine_fingerprint: "engine-hash".into(),
                resources_fingerprint: "resources-hash".into(),
                level_fingerprint: "level-hash".into(),
                music_fingerprint: "music-hash".into(),
                effective_config_fingerprint: "config-hash".into(),
                resolution: [1280, 720],
                fps: 60,
                start_time_micros: 0,
                duration_micros: 1_000_000,
                backend: "wgpu".into(),
                execution_mode: "interpreter".into(),
            },
            effective_backend_status: "Sono VM / Interpreter".into(),
            gcc_cache_state: None,
            gcc_codegen_version: None,
            gcc_runtime_abi: None,
            sono_gcc_compilation: None,
            stage_timings: vec![
                StageTiming {
                    name: "Render frames".into(),
                    duration_ms: 75.0,
                    samples: 1,
                    semantics: StageSemantics::ExclusiveWallSpan,
                },
                StageTiming {
                    name: "Mux output".into(),
                    duration_ms: 25.0,
                    samples: 1,
                    semantics: StageSemantics::ExclusiveWallSpan,
                },
                StageTiming {
                    name: "GPU lane".into(),
                    duration_ms: 90.0,
                    samples: 60,
                    semantics: StageSemantics::OverlappingAttribution,
                },
            ],
            pipeline_overlap: Some(PipelineOverlap {
                wall_span_ms: 90.0,
                producer_watch_ms: 40.0,
                producer_snapshot_ms: 10.0,
                producer_blocked_ms: 5.0,
                queue_residence_ms: 10.0,
                consumer_waiting_ms: 8.0,
                render_lane_ms: 90.0,
                output_lane_ms: 20.0,
                queue_capacity: 4,
                queue_high_water: 3,
                render_workers: 2,
            }),
            frames: 60,
            watch_runtime_frames: 60,
            watch_callbacks: 60,
            watch_evaluations: 90,
            sono_gcc_regions_executed: 0,
            profiling_detail_enabled: false,
            environment: PerformanceEnvironment {
                renderer_version: "0.1".into(),
                renderer_commit: None,
                operating_system: "windows".into(),
                architecture: "x86_64".into(),
                cpu_identity: None,
                gpu_identity: None,
            },
        }
    }

    #[test]
    fn report_separates_facts_from_inferences_and_preserves_queue_and_overlap_semantics() {
        let report = RenderDiagnosticRecord::from_performance(&performance_record());
        assert_eq!(
            report.critical_path.largest_exclusive_phase.as_deref(),
            Some("Render frames")
        );
        assert!(!report.critical_path.exact_dependency_path_available);
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == FindingKind::MeasuredFact && f.code == "queue_wait"));
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == FindingKind::Unavailable && f.code == "pipeline_cause"));
        assert!(report
            .performance
            .as_ref()
            .unwrap()
            .stage_timings
            .iter()
            .any(|s| s.semantics == StageSemantics::OverlappingAttribution));
        let json = serde_json::to_string(&report).unwrap();
        let decoded: RenderDiagnosticRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.schema_version, DIAGNOSTIC_SCHEMA_VERSION);
    }

    #[test]
    fn exported_text_redacts_paths_and_credentials() {
        let sanitized = sanitize_text(
            "failed at C:\\Users\\Admin\\secret file.zip: token=abc123 Authorization: Bot supersecret\nnext",
        );
        assert!(!sanitized.contains("Users"));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("supersecret"));
        assert!(sanitized.contains("<path>"));
        assert!(sanitized.contains("token=<redacted>"));
        assert!(sanitized.contains("Authorization: <redacted>"));
    }

    #[test]
    fn failed_attempt_reports_subsystem_phase_and_missing_timings() {
        let mut config = RenderConfig::default();
        config.width = 1280;
        config.height = 720;
        let report = RenderDiagnosticRecord::from_failure(
            "diagnostic-job-1",
            Some("job-1".into()),
            &config,
            Some("Rendering".into()),
            Some("renderer".into()),
            &["GPU readback failed".into()],
            DiagnosticOutcome::Failed,
        );
        assert_eq!(report.error.as_ref().unwrap().subsystem, "gpu");
        assert_eq!(
            report.error.as_ref().unwrap().phase.as_deref(),
            Some("Rendering")
        );
        assert!(report.performance.is_none());
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == "performance_record" && f.kind == FindingKind::Unavailable));
    }

    #[test]
    fn successful_export_summary_contains_only_selected_ffprobe_facts() {
        let record = performance_record();
        let probe = serde_json::json!({
            "streams": [
                {"codec_type":"video", "codec_name":"h264", "width":1280, "height":720, "avg_frame_rate":"60/1"},
                {"codec_type":"audio", "codec_name":"aac"}
            ],
            "format": {"duration":"1.000", "filename":"C:/private/output.mp4"}
        });
        let report = RenderDiagnosticRecord::from_export(&record, "managed", &probe, Some(true));
        let media = report.media.as_ref().unwrap();
        assert!(media.output_validation.ffprobe_succeeded);
        assert_eq!(media.output_validation.video_codec.as_deref(), Some("h264"));
        assert_eq!(media.output_validation.audio_codec.as_deref(), Some("aac"));
        assert_eq!(media.output_validation.width, Some(1280));
        assert_eq!(media.output_validation.duration_seconds, Some(1.0));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("private"));
        assert!(report.validate_schema_version().is_ok());
    }

    #[test]
    fn watch_compatibility_warnings_are_observed_diagnostics_with_parity_limits() {
        let mut report = RenderDiagnosticRecord::from_export(
            &performance_record(),
            "managed",
            &serde_json::json!({"streams":[],"format":{"duration":"1"}}),
            None,
        );
        let warning = "Stack operations use bounded Temporary Memory compatibility semantics.";
        report.append_watch_compatibility_warnings([warning.to_owned(), warning.to_owned()]);
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.code == "watch_compatibility_approximation")
            .collect::<Vec<_>>();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::ObservedEvent);
        assert_eq!(findings[0].summary, warning);
        assert!(findings[0]
            .evidence
            .iter()
            .any(|evidence| evidence.contains("parity is unverified")));
    }

    #[test]
    fn cache_hits_and_vm_fallback_are_reported_without_claiming_saved_time() {
        let mut cached = performance_record();
        cached.workload.execution_mode = "sono_gcc".into();
        cached.gcc_cache_state = Some("persistent_hit".into());
        cached.sono_gcc_compilation = Some(crate::performance::SonoGccCompilationMetrics {
            cache_state: "persistent_hit".into(),
            cache_lookup_ms: 3.0,
            compilation_ms: 0.0,
            native_load_ms: 1.0,
            graph_identity_ms: 0.5,
            disk_validation_ms: 0.5,
            metadata_reconstruction_ms: 0.25,
            cache_write_ms: 0.0,
        });
        let report = RenderDiagnosticRecord::from_performance(&cached);
        assert_eq!(report.effective_backend, "sono_gcc_native");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "compiler_cache_hit"));
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "compiler_time_saved"
                && finding.kind == FindingKind::Unavailable));

        let mut fallback = performance_record();
        fallback.workload.execution_mode = "sono_gcc".into();
        fallback.gcc_cache_state = Some("fallback".into());
        fallback.effective_backend_status =
            "Sono-GCC unavailable; falling back to Sono VM: compiler error".into();
        let report = RenderDiagnosticRecord::from_performance(&fallback);
        assert_eq!(report.effective_backend, "sono_vm_fallback");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "backend_fallback"));
    }

    #[test]
    fn unsupported_diagnostic_schema_is_rejected_explicitly() {
        let mut report = RenderDiagnosticRecord::from_performance(&performance_record());
        report.schema_version += 1;
        assert!(report.validate_schema_version().is_err());
    }

    #[test]
    fn diagnostic_store_round_trips_and_rejects_path_identifiers() {
        let directory = tempfile::tempdir().unwrap();
        let store = RenderDiagnosticStore::open_at(directory.path().join("reports"), 4).unwrap();
        let report = RenderDiagnosticRecord::from_performance(&performance_record());
        store.save(&report).unwrap();
        let loaded = store.get_for_performance("perf-test").unwrap();
        assert_eq!(loaded.id, report.id);
        assert!(store.get("../escape").is_err());
        assert!(store.save(&report).is_err());
    }
}
