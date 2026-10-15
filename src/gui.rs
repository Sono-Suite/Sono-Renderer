//! Native desktop frontend. Widgets edit RenderConfig; the export worker calls the backend.
use crate::{
    export_control::{Cancelled, ExportControl},
    export_progress::ExportProgress,
    render::{LevelOptionValue, RenderBackend, RenderConfig},
};
use anyhow::{bail, Context, Result};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

type MediaThumbnailMessage = (String, String, std::result::Result<Vec<u8>, String>);

fn gameplay_ui_asset_texture(
    ctx: &egui::Context,
    cache: &mut HashMap<String, egui::TextureHandle>,
    asset_id: &str,
    path: &Path,
) -> Option<egui::TextureId> {
    if !cache.contains_key(asset_id) {
        let image = image::open(path).ok()?.thumbnail(72, 72).to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        cache.insert(
            asset_id.to_owned(),
            ctx.load_texture(
                format!("gameplay-ui-asset-{asset_id}"),
                color_image,
                egui::TextureOptions::LINEAR,
            ),
        );
    }
    cache.get(asset_id).map(egui::TextureHandle::id)
}

// The hook captures the origin before unwinding. A catch-site backtrace would
// only show the worker recovery code. Other threads keep their existing hook.
thread_local! {
    static WORKER_PANIC: std::cell::RefCell<Option<Option<String>>> = const {
        std::cell::RefCell::new(None)
    };
}
fn catch_export_panic<T>(export: impl FnOnce() -> Result<T>) -> Result<T> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let handled = WORKER_PANIC.with(|slot| {
                let mut slot = slot.borrow_mut();
                if let Some(diagnostic) = slot.as_mut() {
                    let message = panic_message(info.payload());
                    let location = info.location().map(|l| l.to_string()).unwrap_or_default();
                    let trace = std::backtrace::Backtrace::capture();
                    let mut text =
                        format!("Export worker panicked: {message}\nPanic origin: {location}");
                    if trace.status() == std::backtrace::BacktraceStatus::Captured {
                        text.push_str(&format!("\n{trace}"));
                    }
                    *diagnostic = Some(text);
                    true
                } else {
                    false
                }
            });
            if !handled {
                previous(info);
            }
        }));
    });
    WORKER_PANIC.with(|slot| *slot.borrow_mut() = Some(None));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(export));
    let diagnostic = WORKER_PANIC.with(|slot| slot.borrow_mut().take().flatten());
    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow::anyhow!(diagnostic.unwrap_or_else(|| {
            format!(
                "Export worker panicked: {}",
                panic_message(payload.as_ref())
            )
        }))),
    }
}
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

pub fn launch() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn FreeConsole() -> i32;
        }
        FreeConsole();
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 790.0])
            .with_min_inner_size([820.0, 650.0])
            .with_position([40.0, 40.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Sono-Renderer",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = Desktop::default();
            app.parent = cc.winit_window().cloned();
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Native GUI startup failed: {error}"));
    if let Err(error) = &result {
        rfd::MessageDialog::new()
            .set_title("Sono-Renderer")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    result
}

#[derive(Default)]
struct LiveState {
    progress: Option<ExportProgress>,
    logs: VecDeque<String>,
}
impl LiveState {
    fn log(&mut self, message: String) {
        log::info!(target: "renderer::gui::progress", "{message}");
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(message);
    }
    fn progress(&mut self, event: ExportProgress) {
        if self
            .progress
            .as_ref()
            .is_none_or(|old| old.phase != event.phase)
        {
            self.log(format!("{:?}", event.phase));
        }
        self.progress = Some(event);
    }
}
struct Job {
    control: ExportControl,
    result: mpsc::Receiver<Result<(String, crate::render_diagnostics::RenderDiagnosticRecord)>>,
    worker: JoinHandle<()>,
    config_snapshot: RenderConfig,
}
struct EngineJob {
    result: mpsc::Receiver<EngineJobMessage>,
    worker: JoinHandle<()>,
}
struct PerformanceJob {
    result: mpsc::Receiver<std::result::Result<crate::performance::BenchmarkReport, String>>,
    worker: JoinHandle<()>,
}
struct RegressionJob {
    result: mpsc::Receiver<std::result::Result<GuiRegressionOutcome, String>>,
    worker: JoinHandle<()>,
}
struct ManifestJob {
    result: mpsc::Receiver<std::result::Result<ManifestJobOutcome, String>>,
    worker: JoinHandle<()>,
}
enum MediaAcquisitionOutcome {
    Inspected(crate::media_acquisition::MediaInfo),
    Downloaded {
        mode: crate::media_acquisition::AcquisitionMode,
        media: crate::media_acquisition::AcquiredMedia,
    },
}
struct MediaAcquisitionJob {
    result: mpsc::Receiver<std::result::Result<MediaAcquisitionOutcome, String>>,
    worker: JoinHandle<()>,
    control: crate::media_acquisition::AcquisitionControl,
}
struct ProgramPreviewFrame {
    time: f64,
    width: u32,
    height: u32,
    rgb: Vec<u8>,
    config: RenderConfig,
    ui_state: crate::render_ui::UiFrameState,
}
struct ProgramPreviewJob {
    generation: u64,
    time: f64,
    config: RenderConfig,
    result: mpsc::Receiver<std::result::Result<ProgramPreviewFrame, String>>,
    worker: JoinHandle<()>,
}
struct ChartEngineJob {
    result: mpsc::Receiver<std::result::Result<ChartEngineSummary, String>>,
    worker: JoinHandle<()>,
}
struct ChartEngineSummary {
    path: PathBuf,
    archetypes: Vec<String>,
    identity: crate::chart_studio::EngineIdentity,
}
struct ChartPreviewFrame {
    revision: u64,
    time: f64,
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}
struct ChartPreviewJob {
    revision: u64,
    result: mpsc::Receiver<std::result::Result<ChartPreviewFrame, String>>,
    worker: JoinHandle<()>,
}
struct ChartTempoJob {
    generation: u64,
    path: PathBuf,
    control: crate::tempo_analysis::AnalysisControl,
    progress: Arc<Mutex<crate::tempo_analysis::AnalysisProgress>>,
    result: mpsc::Receiver<
        std::result::Result<(crate::tempo_analysis::TempoAnalysis, f64, Vec<f32>), String>,
    >,
    worker: JoinHandle<()>,
}
struct ChartAudioPlaybackStart {
    generation: u64,
    path: PathBuf,
    result: mpsc::Receiver<std::result::Result<crate::timeline_playback::TimelinePlayback, String>>,
    worker: JoinHandle<()>,
}
struct QuickValidationJob {
    config: RenderConfig,
    result: mpsc::Receiver<std::result::Result<QuickValidationReport, String>>,
    worker: JoinHandle<()>,
}
struct QuickValidationReport {
    summary: String,
    details: String,
    ffmpeg_status: String,
}

enum PresentationJobOutcome {
    Preview {
        time: f64,
        png: Vec<u8>,
    },
    SourcePreview {
        time: f64,
        png: Vec<u8>,
    },
    Export {
        output: PathBuf,
    },
    ProxyGenerated {
        source_id: String,
        source_path: PathBuf,
        proxy_path: PathBuf,
    },
    SpeechDependencyInstalled {
        dependency: crate::subtitles::SpeechDependency,
        path: PathBuf,
    },
    SubtitlesGenerated {
        snapshot_digest: String,
        source_name: String,
        caption_ids: Vec<String>,
        project: crate::composition::CompositionProject,
    },
}

#[derive(Debug, Clone)]
struct PresentationProgress {
    message: String,
    fraction: Option<f32>,
}

#[derive(Clone, Copy)]
enum SourceInsertMode {
    Video,
    Audio,
    LinkedAv,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SourceEditOperation {
    #[default]
    PlaceAtPlayhead,
    InsertRipple,
    OverwriteTrack,
    Append,
}

struct PresentationJob {
    result: mpsc::Receiver<std::result::Result<PresentationJobOutcome, String>>,
    progress: Option<mpsc::Receiver<PresentationProgress>>,
    worker: JoinHandle<()>,
    control: ExportControl,
}

#[derive(Default)]
struct CompositionEditHistory {
    undo: Vec<crate::composition::CompositionProject>,
    redo: Vec<crate::composition::CompositionProject>,
}

impl CompositionEditHistory {
    const LIMIT: usize = 40;

    fn remember(&mut self, previous: crate::composition::CompositionProject) {
        if self.undo.len() == Self::LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(previous);
        self.redo.clear();
    }

    fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn undo(&mut self, current: &mut crate::composition::CompositionProject) -> bool {
        let Some(mut previous) = self.undo.pop() else {
            return false;
        };
        // Undoing a clip insertion must not remove its media-bin asset or WAV.
        for source in &current.sources {
            if !previous.sources.iter().any(|item| item.id == source.id) {
                previous.sources.push(source.clone());
            }
        }
        self.redo.push(std::mem::replace(current, previous));
        true
    }

    fn redo(&mut self, current: &mut crate::composition::CompositionProject) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        true
    }
}

struct PresentationPlaybackStart {
    result: mpsc::Receiver<std::result::Result<crate::timeline_playback::TimelinePlayback, String>>,
    worker: JoinHandle<()>,
}

/// A separate project and media worker. This state is not derived from the
/// Sonolus RenderConfig or the Watch render lifecycle.
struct PresentationStudioView {
    project: crate::composition::CompositionProject,
    project_path: Option<PathBuf>,
    selected_source: Option<String>,
    media_search: String,
    source_monitor_source: Option<String>,
    source_monitor_time: f64,
    source_in: f64,
    source_out: f64,
    source_edit_operation: SourceEditOperation,
    selected_clip: Option<String>,
    selected_clips: HashSet<String>,
    playhead: f64,
    output: PathBuf,
    preview_texture: Option<egui::TextureHandle>,
    preview_png: Option<(f64, Vec<u8>)>,
    source_preview_texture: Option<egui::TextureHandle>,
    thumbnail_cache: HashMap<String, egui::TextureHandle>,
    thumbnail_lru: VecDeque<String>,
    thumbnail_queue: VecDeque<(String, String, PathBuf)>,
    thumbnail_requested: HashSet<String>,
    thumbnail_failed: HashSet<String>,
    thumbnail_tx: mpsc::Sender<MediaThumbnailMessage>,
    thumbnail_rx: mpsc::Receiver<MediaThumbnailMessage>,
    thumbnail_worker_active: bool,
    job: Option<PresentationJob>,
    message: String,
    error: Option<String>,
    pending_keyframe_easing: crate::composition::Easing,
    selected_animated_property: crate::composition::AnimatedProperty,
    selected_title_preset: String,
    selected_caption_preset: String,
    presentation_template: Option<crate::composition::CompositionTemplate>,
    template_source_bindings: HashMap<String, String>,
    template_text_values: HashMap<String, String>,
    template_responsive: bool,
    transcription_language: String,
    job_progress: Option<PresentationProgress>,
    title_preset_name: String,
    selected_curve_keys: Option<(String, crate::composition::AnimatedProperty, HashSet<usize>)>,
    curve_drag_origin: Option<(
        String,
        crate::composition::AnimatedProperty,
        egui::Pos2,
        Vec<(usize, f64, f64)>,
    )>,
    keyframe_clipboard: Option<(
        crate::composition::AnimatedProperty,
        Vec<crate::composition::ScalarKeyframe>,
    )>,
    audio_preferences: crate::audio_input::AudioInputPreferences,
    audio_preferences_error: Option<String>,
    audio_devices: Vec<crate::audio_input::InputDeviceInfo>,
    audio_devices_loaded: bool,
    audio_device_check: Instant,
    audio_capture: Option<crate::audio_input::CaptureSession>,
    recoverable_takes: Vec<crate::audio_input::RecoveredTake>,
    playback: Option<crate::timeline_playback::TimelinePlayback>,
    playback_start: Option<PresentationPlaybackStart>,
    record_voiceover_after_playback: bool,
    timeline_zoom: f32,
    timeline_track_height: f32,
    timeline_snapping: bool,
    scrubbing: bool,
    timeline_drag_before: Option<crate::composition::CompositionProject>,
    clip_clipboard: Option<crate::composition::CompositionClip>,
    effect_clipboard: Option<crate::composition::VisualEffectInstance>,
    effect_stack_clipboard: Option<Vec<crate::composition::VisualEffectInstance>>,
    solid_color_input: String,
    dirty: bool,
    autosave_pending: bool,
    last_autosave: Instant,
    recovery_checked: bool,
    recovery_snapshots: Vec<(PathBuf, crate::composition::CompositionRecoverySnapshot)>,
    edit_history: CompositionEditHistory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimelineDragKind {
    Move,
    TrimStart,
    TrimEnd,
}

struct TimelineDragUpdate {
    clip_id: String,
    kind: TimelineDragKind,
    delta_x: f32,
    pointer: egui::Pos2,
    origin: crate::composition::CompositionClip,
}

#[derive(Clone)]
struct PresentationMediaDrag {
    source_id: String,
}

fn add_media_source_to_timeline_track(
    project: &mut crate::composition::CompositionProject,
    source_id: &str,
    track_id: &str,
    timeline_start: f64,
) -> Result<String> {
    let track = project
        .tracks
        .iter()
        .find(|track| track.id == track_id)
        .context("drop target track no longer exists")?;
    if track.locked {
        bail!("cannot drop media onto a locked track");
    }
    if !track.enabled {
        bail!("cannot drop media onto a disabled track");
    }
    let kind = track.kind;
    let source = project
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .context("dragged media source no longer exists")?;
    let has_video = source.has_video;
    let has_audio = source.has_audio;
    if kind == crate::composition::TrackKind::Audio {
        if !has_audio {
            bail!("this media source has no audio stream for the selected track");
        }
        return project.add_clip_to_track(
            source_id,
            crate::composition::ClipKind::Audio,
            timeline_start,
            Some(track_id),
        );
    }
    if !has_video {
        bail!("this media source has no video stream for the selected track");
    }
    if !has_audio {
        return project.add_clip_to_track(
            source_id,
            crate::composition::ClipKind::Video,
            timeline_start,
            Some(track_id),
        );
    }

    // A/V media dropped onto a visual row becomes one linked pair. Build both
    // instances on a candidate so a missing/locked audio destination cannot
    // leave half of the pair in the project.
    let mut candidate = project.clone();
    let video_id = candidate.add_clip_to_track(
        source_id,
        crate::composition::ClipKind::Video,
        timeline_start,
        Some(track_id),
    )?;
    let audio_id = candidate.add_clip(
        source_id,
        crate::composition::ClipKind::Audio,
        timeline_start,
    )?;
    let link_id = format!("av-drop-{video_id}");
    for clip in &mut candidate.clips {
        if clip.id == video_id || clip.id == audio_id {
            clip.link_group_id = Some(link_id.clone());
        }
    }
    candidate.validate()?;
    *project = candidate;
    Ok(video_id)
}

fn presentation_timeline(
    ui: &mut egui::Ui,
    project: &mut crate::composition::CompositionProject,
    selected_clip: &mut Option<String>,
    selected_clips: &mut HashSet<String>,
    playhead: &mut f64,
    pixels_per_second: &mut f32,
    track_height: &mut f32,
    snapping: &mut bool,
) -> Option<String> {
    let mut marker_error = None;
    let mut group_selection = false;
    let mut ungroup_selection = false;
    ui.horizontal(|ui| {
        ui.label("Timeline zoom");
        ui.add(egui::Slider::new(pixels_per_second, 8.0..=240.0).logarithmic(true));
        ui.label("Track height");
        ui.add(egui::Slider::new(track_height, 36.0..=100.0).suffix(" px"));
        ui.checkbox(snapping, "Snap to frames/playhead/edges");
        ui.weak("Ctrl/Command-click clips to multi-select");
        group_selection = ui
            .add_enabled(
                selected_clips.len() >= 2,
                egui::Button::new("Group selection"),
            )
            .clicked();
        let selected_group = selected_clip
            .as_deref()
            .and_then(|id| project.clips.iter().find(|clip| clip.id == id))
            .and_then(|clip| clip.group_id.as_deref());
        ungroup_selection = ui
            .add_enabled(selected_group.is_some(), egui::Button::new("Ungroup"))
            .clicked();
        if ui.small_button("+ Marker at playhead").clicked() {
            let label = format!("Marker {}", project.markers.len() + 1);
            if let Err(error) = project.add_marker(*playhead, label) {
                marker_error = Some(format!("{error:#}"));
            }
        }
    });
    if group_selection {
        let before = project.clone();
        let ids = selected_clips.iter().cloned().collect::<Vec<_>>();
        match project.group_clips(&ids) {
            Ok(group_id) => {
                selected_clips.clear();
                selected_clips.extend(
                    project
                        .clips
                        .iter()
                        .filter(|clip| clip.group_id.as_deref() == Some(group_id.as_str()))
                        .map(|clip| clip.id.clone()),
                );
            }
            Err(error) => {
                *project = before;
                marker_error = Some(format!("{error:#}"));
            }
        }
    }
    if ungroup_selection {
        if let Some(id) = selected_clip.clone() {
            if let Err(error) = project.ungroup_clip(&id) {
                marker_error = Some(format!("{error:#}"));
            } else {
                selected_clips
                    .retain(|selected| project.clips.iter().any(|clip| &clip.id == selected));
            }
        }
    }
    let mut add_track = None;
    let mut reorder_track = None;
    let mut remove_track = None;
    let mut track_updates = Vec::new();
    ui.horizontal(|ui| {
        if ui.small_button("+ Video track").clicked() {
            add_track = Some(crate::composition::TrackKind::Visual);
        }
        if ui.small_button("+ Audio track").clicked() {
            add_track = Some(crate::composition::TrackKind::Audio);
        }
    });
    ui.collapsing("Track controls", |ui| {
        let mut tracks = project.tracks.clone();
        tracks.sort_by_key(|track| (track.kind, track.order, track.id.clone()));
        for track in tracks {
            let mut enabled = track.enabled;
            let mut muted = track.muted;
            let mut solo = track.solo;
            let mut locked = track.locked;
            let mut gain = track.gain;
            ui.horizontal(|ui| {
                ui.label(format!("{} · {}", track.order + 1, track.name));
                ui.checkbox(
                    &mut enabled,
                    if track.kind == crate::composition::TrackKind::Visual {
                        "Visible"
                    } else {
                        "Enabled"
                    },
                );
                if track.kind == crate::composition::TrackKind::Audio {
                    ui.checkbox(&mut muted, "Mute");
                    ui.checkbox(&mut solo, "Solo");
                    ui.add_enabled_ui(!locked, |ui| {
                        ui.label("Gain");
                        ui.add(
                            egui::DragValue::new(&mut gain)
                                .range(0.0..=16.0)
                                .speed(0.02)
                                .suffix("×"),
                        );
                    });
                }
                ui.checkbox(&mut locked, "Lock");
                if ui.small_button("Up").clicked() {
                    reorder_track = Some((track.id.clone(), -1));
                }
                if ui.small_button("Down").clicked() {
                    reorder_track = Some((track.id.clone(), 1));
                }
                let has_clips = project
                    .clips
                    .iter()
                    .any(|clip| clip.track_id.as_deref() == Some(track.id.as_str()));
                if ui
                    .add_enabled(!has_clips, egui::Button::new("Remove"))
                    .clicked()
                {
                    remove_track = Some(track.id.clone());
                }
            });
            if enabled != track.enabled
                || muted != track.muted
                || solo != track.solo
                || gain != track.gain
                || locked != track.locked
            {
                track_updates.push((track.id, enabled, muted, solo, gain, locked));
            }
        }
    });
    if !project.markers.is_empty() {
        let mut remove_marker = None;
        ui.collapsing(format!("Markers ({})", project.markers.len()), |ui| {
            for marker in &mut project.markers {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut marker.time)
                            .range(0.0..=project.duration)
                            .speed(0.05),
                    );
                    ui.text_edit_singleline(&mut marker.label);
                    if ui.small_button("Go").clicked() {
                        *playhead = marker.time;
                    }
                    if ui.small_button("Remove").clicked() {
                        remove_marker = Some(marker.id.clone());
                    }
                });
            }
        });
        if let Some(id) = remove_marker {
            project.markers.retain(|marker| marker.id != id);
        }
    }
    let mut track_error = marker_error;
    if let Some(kind) = add_track {
        let name = match kind {
            crate::composition::TrackKind::Visual => format!(
                "Video {}",
                project
                    .tracks
                    .iter()
                    .filter(|track| track.kind == kind)
                    .count()
                    + 1
            ),
            crate::composition::TrackKind::Audio => format!(
                "Audio {}",
                project
                    .tracks
                    .iter()
                    .filter(|track| track.kind == kind)
                    .count()
                    + 1
            ),
        };
        if let Err(error) = project.add_track(kind, name) {
            track_error = Some(format!("{error:#}"));
        }
    }
    if let Some((track_id, direction)) = reorder_track {
        if let Err(error) = project.move_track(&track_id, direction) {
            track_error = Some(format!("{error:#}"));
        }
    }
    if let Some(track_id) = remove_track {
        if let Err(error) = project.remove_track(&track_id) {
            track_error = Some(format!("{error:#}"));
        }
    }
    for (track_id, enabled, muted, solo, gain, locked) in track_updates {
        if let Some(track) = project.tracks.iter_mut().find(|track| track.id == track_id) {
            track.enabled = enabled;
            track.muted = muted;
            track.solo = solo;
            track.gain = gain;
            track.locked = locked;
        }
    }
    let scale = f64::from((*pixels_per_second).max(1.0));
    let canvas_width = (project.duration * scale).max(640.0).min(20_000_000.0) as f32;
    let mut select_after = None;
    let mut delete_after = None;
    let mut edit_after: Option<TimelineDragUpdate> = None;
    let mut track_rows = Vec::<(egui::Rect, crate::composition::CompositionTrack)>::new();
    egui::ScrollArea::both()
        .id_salt("composition-multitrack-timeline")
        .max_height(300.0)
        .show(ui, |ui| {
            let (axis_rect, axis_response) =
                ui.allocate_exact_size(egui::vec2(canvas_width, 28.0), egui::Sense::click());
            let painter = ui.painter();
            painter.rect_filled(axis_rect, 2.0, egui::Color32::from_gray(42));
            let tick_seconds = if scale >= 120.0 {
                1.0
            } else if scale >= 28.0 {
                5.0
            } else if scale >= 8.0 {
                10.0
            } else {
                30.0
            };
            let viewport = ui.clip_rect();
            let visible_start =
                ((viewport.left() - axis_rect.left()).max(0.0) as f64 / scale).max(0.0);
            let visible_end = ((viewport.right() - axis_rect.left()).max(0.0) as f64 / scale)
                .min(project.duration);
            let mut tick = (visible_start / tick_seconds).floor() * tick_seconds;
            while tick <= visible_end {
                let x = axis_rect.left() + (tick * scale) as f32;
                painter.line_segment(
                    [
                        egui::pos2(x, axis_rect.top()),
                        egui::pos2(x, axis_rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                );
                painter.text(
                    egui::pos2(x + 3.0, axis_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format_timecode(tick, project.fps),
                    egui::FontId::monospace(11.0),
                    egui::Color32::LIGHT_GRAY,
                );
                tick += tick_seconds;
            }
            for marker in &project.markers {
                if marker.time >= visible_start && marker.time <= visible_end {
                    let x = axis_rect.left() + (marker.time * scale) as f32;
                    painter.line_segment(
                        [
                            egui::pos2(x, axis_rect.top()),
                            egui::pos2(x, axis_rect.bottom()),
                        ],
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 213, 79)),
                    );
                    painter.text(
                        egui::pos2(x + 3.0, axis_rect.top() + 2.0),
                        egui::Align2::LEFT_TOP,
                        marker.label.chars().take(18).collect::<String>(),
                        egui::FontId::proportional(9.0),
                        egui::Color32::from_rgb(255, 230, 150),
                    );
                }
            }
            if axis_response.clicked() {
                if let Some(pointer) = axis_response.interact_pointer_pos() {
                    let raw = ((pointer.x - axis_rect.left()) as f64 / scale)
                        .clamp(0.0, project.duration);
                    *playhead = snap_timeline_time(project, raw, None, *playhead, scale, *snapping);
                }
            }

            let mut tracks = project.tracks.clone();
            tracks.sort_by_key(|track| (track.kind, track.order, track.id.clone()));
            for track in tracks {
                let row_height = (*track_height).clamp(36.0, 100.0);
                let (row, row_response) = ui.allocate_exact_size(
                    egui::vec2(canvas_width, row_height),
                    egui::Sense::hover(),
                );
                track_rows.push((row, track.clone()));
                let painter = ui.painter();
                let media_drop_payload = row_response.dnd_hover_payload::<PresentationMediaDrag>();
                let media_drop_compatible = media_drop_payload
                    .as_ref()
                    .and_then(|payload| {
                        project
                            .sources
                            .iter()
                            .find(|source| source.id == payload.source_id)
                    })
                    .is_some_and(|source| {
                        track.enabled
                            && !track.locked
                            && match track.kind {
                                crate::composition::TrackKind::Audio => source.has_audio,
                                crate::composition::TrackKind::Visual => source.has_video,
                            }
                    });
                painter.rect_filled(
                    row,
                    2.0,
                    if media_drop_compatible {
                        egui::Color32::from_rgb(32, 76, 70)
                    } else if media_drop_payload.is_some() {
                        egui::Color32::from_rgb(74, 43, 43)
                    } else {
                        egui::Color32::from_gray(27)
                    },
                );
                painter.text(
                    egui::pos2(row.left() + 5.0, row.top() + 8.0),
                    egui::Align2::LEFT_CENTER,
                    format!(
                        "{}  |  {}{}{}",
                        track.name,
                        if track.enabled { "" } else { "DISABLED  " },
                        if track.muted { "MUTED  " } else { "" },
                        if track.locked { "LOCKED" } else { "" }
                    ),
                    egui::FontId::proportional(11.0),
                    egui::Color32::from_gray(190),
                );
                if let Some(payload) = row_response.dnd_release_payload::<PresentationMediaDrag>() {
                    let pointer = row_response
                        .interact_pointer_pos()
                        .unwrap_or_else(|| row.center());
                    let raw_time =
                        ((pointer.x - row.left()) as f64 / scale).clamp(0.0, project.duration);
                    let drop_time =
                        snap_timeline_time(project, raw_time, None, *playhead, scale, *snapping);
                    match add_media_source_to_timeline_track(
                        project,
                        &payload.source_id,
                        &track.id,
                        drop_time,
                    ) {
                        Ok(clip_id) => {
                            *selected_clip = Some(clip_id.clone());
                            selected_clips.clear();
                            selected_clips.insert(clip_id);
                            *playhead = drop_time;
                            track_error = None;
                        }
                        Err(error) => track_error = Some(format!("{error:#}")),
                    }
                }
                let visible_start =
                    ((viewport.left() - row.left()).max(0.0) as f64 / scale).max(0.0);
                let visible_end =
                    ((viewport.right() - row.left()).max(0.0) as f64 / scale).min(project.duration);
                let mut second = (visible_start / tick_seconds).floor() * tick_seconds;
                while second <= visible_end {
                    let x = row.left() + (second * scale) as f32;
                    painter.line_segment(
                        [egui::pos2(x, row.top() + 14.0), egui::pos2(x, row.bottom())],
                        egui::Stroke::new(0.5, egui::Color32::from_gray(45)),
                    );
                    second += tick_seconds;
                }
                let playhead_x = row.left() + (*playhead * scale) as f32;
                painter.line_segment(
                    [
                        egui::pos2(playhead_x, row.top()),
                        egui::pos2(playhead_x, row.bottom()),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::YELLOW),
                );

                let mut clips: Vec<_> = project
                    .clips
                    .iter()
                    .filter(|clip| {
                        project
                            .track_for_clip(clip)
                            .is_some_and(|item| item.id == track.id)
                            && clip.timeline_start + clip.duration >= visible_start
                            && clip.timeline_start <= visible_end
                    })
                    .collect();
                clips.sort_by(|a, b| {
                    a.timeline_start
                        .total_cmp(&b.timeline_start)
                        .then(a.z_order.cmp(&b.z_order))
                });
                for clip in clips {
                    let left = row.left() + (clip.timeline_start * scale) as f32;
                    let right = row.left() + ((clip.timeline_start + clip.duration) * scale) as f32;
                    let clip_rect = egui::Rect::from_min_max(
                        egui::pos2(left, row.top() + 17.0),
                        egui::pos2(right.max(left + 4.0), row.bottom() - 3.0),
                    );
                    let base_color = match clip.kind {
                        crate::composition::ClipKind::Video => {
                            egui::Color32::from_rgb(50, 115, 165)
                        }
                        crate::composition::ClipKind::Audio => egui::Color32::from_rgb(45, 120, 90),
                        crate::composition::ClipKind::Text => egui::Color32::from_rgb(145, 95, 55),
                        crate::composition::ClipKind::Adjustment => {
                            egui::Color32::from_rgb(145, 105, 180)
                        }
                    };
                    painter.rect_filled(
                        clip_rect,
                        3.0,
                        if clip.enabled {
                            base_color
                        } else {
                            egui::Color32::from_gray(65)
                        },
                    );
                    if selected_clips.contains(&clip.id) {
                        painter.line_segment(
                            [clip_rect.left_top(), clip_rect.right_top()],
                            egui::Stroke::new(2.0, egui::Color32::YELLOW),
                        );
                        painter.line_segment(
                            [clip_rect.left_bottom(), clip_rect.right_bottom()],
                            egui::Stroke::new(2.0, egui::Color32::YELLOW),
                        );
                    }
                    if clip.group_id.is_some() {
                        painter.circle_filled(
                            egui::pos2(clip_rect.left() + 7.0, clip_rect.top() + 7.0),
                            3.0,
                            egui::Color32::from_rgb(255, 170, 70),
                        );
                    }
                    painter.text(
                        egui::pos2(clip_rect.left() + 5.0, clip_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        clip.name.chars().take(36).collect::<String>(),
                        egui::FontId::proportional(11.0),
                        egui::Color32::WHITE,
                    );
                    if clip.effects.iter().any(|effect| effect.enabled) {
                        painter.text(
                            egui::pos2(clip_rect.right() - 4.0, clip_rect.top() + 5.0),
                            egui::Align2::RIGHT_TOP,
                            "FX",
                            egui::FontId::proportional(9.0),
                            egui::Color32::LIGHT_YELLOW,
                        );
                    }
                    for transition in project.transitions.iter().filter(|transition| {
                        transition.from_clip_id == clip.id || transition.to_clip_id == clip.id
                    }) {
                        let transition_rect = egui::Rect::from_min_max(
                            egui::pos2(
                                row.left() + (transition.start_time * scale) as f32,
                                clip_rect.top() + 2.0,
                            ),
                            egui::pos2(
                                row.left()
                                    + ((transition.start_time + transition.duration) * scale)
                                        as f32,
                                clip_rect.top() + 6.0,
                            ),
                        );
                        let transition_color = match transition.kind {
                            crate::composition::TransitionKind::CrossDissolve => {
                                egui::Color32::from_rgb(90, 220, 245)
                            }
                            crate::composition::TransitionKind::DipToBlack => {
                                egui::Color32::from_rgb(105, 105, 130)
                            }
                            crate::composition::TransitionKind::DipToWhite => egui::Color32::WHITE,
                        };
                        painter.rect_filled(transition_rect, 1.0, transition_color);
                    }
                    if clip.mask.is_some() {
                        painter.text(
                            egui::pos2(clip_rect.right() - 4.0, clip_rect.top() + 14.0),
                            egui::Align2::RIGHT_TOP,
                            "M",
                            egui::FontId::proportional(9.0),
                            egui::Color32::LIGHT_GREEN,
                        );
                    }
                    for key in clip
                        .animated_properties
                        .values()
                        .flat_map(|keys| keys.iter())
                        .chain(
                            clip.effects
                                .iter()
                                .flat_map(|effect| effect.animated_parameters.values())
                                .flat_map(|keys| keys.iter()),
                        )
                        .filter(|key| {
                            key.time >= clip.timeline_start
                                && key.time <= clip.timeline_start + clip.duration
                        })
                    {
                        let key_x = row.left() + (key.time * scale) as f32;
                        painter.circle_filled(
                            egui::pos2(key_x, clip_rect.top() + 4.0),
                            3.0,
                            egui::Color32::YELLOW,
                        );
                    }
                    if clip.kind == crate::composition::ClipKind::Audio {
                        if let Some(source) = clip
                            .source_id
                            .as_deref()
                            .and_then(|id| project.sources.iter().find(|source| source.id == id))
                        {
                            let peaks = &source.waveform_peaks;
                            if !peaks.is_empty() {
                                let width = clip_rect.width().max(1.0);
                                let step = (width / peaks.len() as f32).max(1.0);
                                let visible_count = (width / step).ceil() as usize;
                                for index in 0..visible_count {
                                    let clip_fraction = index as f64 / visible_count.max(1) as f64;
                                    let source_time =
                                        clip.source_start + clip_fraction * clip.duration;
                                    let peak_index = ((source_time / source.duration)
                                        * peaks.len() as f64)
                                        .floor()
                                        as usize;
                                    if let Some(peak) = peaks.get(peak_index.min(peaks.len() - 1)) {
                                        let x = clip_rect.left() + index as f32 * step;
                                        let half = peak.clamp(0.025, 1.0) * 8.0;
                                        painter.line_segment(
                                            [
                                                egui::pos2(x, clip_rect.center().y - half),
                                                egui::pos2(x, clip_rect.center().y + half),
                                            ],
                                            egui::Stroke::new(
                                                step.max(1.0),
                                                egui::Color32::from_rgb(135, 225, 190),
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    let track_locked = project
                        .track_for_clip(clip)
                        .is_some_and(|track| track.locked);
                    let response = ui.interact(
                        clip_rect,
                        ui.id().with(("timeline-clip", &clip.id)),
                        if track_locked {
                            egui::Sense::click()
                        } else {
                            egui::Sense::click_and_drag()
                        },
                    );
                    if response.clicked() {
                        let additive =
                            ui.input(|input| input.modifiers.ctrl || input.modifiers.command);
                        if additive {
                            if !selected_clips.insert(clip.id.clone()) {
                                selected_clips.remove(&clip.id);
                            }
                            select_after = Some(clip.id.clone());
                        } else {
                            selected_clips.clear();
                            selected_clips.insert(clip.id.clone());
                            select_after = Some(clip.id.clone());
                        }
                    }
                    if response.secondary_clicked() && !track_locked {
                        delete_after = Some(clip.id.clone());
                    }
                    if response.dragged() || response.drag_stopped() {
                        if let Some(pointer) = response.interact_pointer_pos() {
                            let edge = 8.0;
                            let kind = if pointer.x - clip_rect.left() <= edge {
                                TimelineDragKind::TrimStart
                            } else if clip_rect.right() - pointer.x <= edge {
                                TimelineDragKind::TrimEnd
                            } else {
                                TimelineDragKind::Move
                            };
                            let origin_id = ui.id().with(("timeline-drag-origin", &clip.id));
                            if response.drag_started() {
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(origin_id, clip.clone());
                                });
                            }
                            let origin = ui
                                .ctx()
                                .data(|data| {
                                    data.get_temp::<crate::composition::CompositionClip>(origin_id)
                                })
                                .unwrap_or_else(|| clip.clone());
                            edit_after = Some(TimelineDragUpdate {
                                clip_id: clip.id.clone(),
                                kind,
                                delta_x: response.drag_delta().x,
                                pointer,
                                origin,
                            });
                        }
                    }
                    if response.drag_stopped() {
                        let origin_id = ui.id().with(("timeline-drag-origin", &clip.id));
                        ui.ctx().data_mut(|data| {
                            data.remove_temp::<crate::composition::CompositionClip>(origin_id);
                        });
                    }
                }
            }
        });

    if let Some(id) = select_after {
        *selected_clip = Some(id);
    }
    if let Some(id) = delete_after {
        match project.remove_clip(&id) {
            Ok(()) => {
                selected_clips
                    .retain(|selected| project.clips.iter().any(|clip| &clip.id == selected));
                if selected_clip
                    .as_deref()
                    .is_some_and(|selected| !project.clips.iter().any(|clip| clip.id == selected))
                {
                    *selected_clip = None;
                }
            }
            Err(error) => track_error = Some(format!("{error:#}")),
        }
    }
    if let Some(drag) = edit_after {
        if let Some(index) = project
            .clips
            .iter()
            .position(|clip| clip.id == drag.clip_id)
        {
            let fps = f64::from(project.fps);
            let frame = 1.0 / fps;
            let delta = f64::from(drag.delta_x) / scale;
            let old = &drag.origin;
            let link_id = old.link_group_id.clone();
            let mut linked_ids = project
                .clips
                .iter()
                .filter(|clip| {
                    clip.id == old.id
                        || link_id
                            .as_deref()
                            .is_some_and(|link_id| clip.link_group_id.as_deref() == Some(link_id))
                })
                .map(|clip| clip.id.clone())
                .collect::<Vec<_>>();
            linked_ids.sort();
            let linked_locked = project
                .clips
                .iter()
                .filter(|clip| linked_ids.contains(&clip.id))
                .any(|clip| {
                    project
                        .track_for_clip(clip)
                        .is_some_and(|track| track.locked)
                });
            if linked_locked {
                track_error =
                    Some("A locked track prevents editing this clip or its linked media.".into());
                return track_error;
            }
            let mut updated = old.clone();
            match drag.kind {
                TimelineDragKind::Move => {
                    let proposed =
                        (old.timeline_start + delta).clamp(0.0, project.duration - old.duration);
                    updated.timeline_start = snap_timeline_time(
                        project,
                        proposed,
                        Some(&old.id),
                        *playhead,
                        scale,
                        *snapping,
                    )
                    .clamp(0.0, project.duration - old.duration);
                }
                TimelineDragKind::TrimStart => {
                    let proposed = old.timeline_start + delta;
                    let snapped = snap_timeline_time(
                        project,
                        proposed,
                        Some(&old.id),
                        *playhead,
                        scale,
                        *snapping,
                    );
                    let still = old
                        .source_id
                        .as_deref()
                        .and_then(|id| project.sources.iter().find(|source| source.id == id))
                        .is_some_and(|source| source.still_image || source.solid_color.is_some());
                    let applied = if still {
                        (snapped - old.timeline_start).clamp(0.0, old.duration - frame)
                    } else {
                        (snapped - old.timeline_start)
                            .clamp(-old.source_start, old.duration - frame)
                    };
                    updated.timeline_start = old.timeline_start + applied;
                    if !still {
                        updated.source_start = old.source_start + applied;
                    }
                    updated.duration = old.duration - applied;
                }
                TimelineDragKind::TrimEnd => {
                    let end = old.timeline_start + old.duration;
                    let proposed = snap_timeline_time(
                        project,
                        end + delta,
                        Some(&old.id),
                        *playhead,
                        scale,
                        *snapping,
                    );
                    let source = old
                        .source_id
                        .as_deref()
                        .and_then(|id| project.sources.iter().find(|source| source.id == id));
                    let max_duration = if source
                        .is_some_and(|source| source.still_image || source.solid_color.is_some())
                    {
                        project.duration - old.timeline_start
                    } else {
                        project.duration.min(
                            project
                                .sources
                                .iter()
                                .find(|source| Some(source.id.as_str()) == old.source_id.as_deref())
                                .map_or(project.duration, |source| source.duration)
                                - old.source_start
                                + old.timeline_start,
                        ) - old.timeline_start
                    };
                    updated.duration =
                        (proposed - old.timeline_start).clamp(frame, max_duration.max(frame));
                }
            }
            if let Some((row, track)) = track_rows
                .iter()
                .find(|(row, _)| row.contains(drag.pointer))
            {
                let _ = row;
                let expected = match updated.kind {
                    crate::composition::ClipKind::Audio => crate::composition::TrackKind::Audio,
                    crate::composition::ClipKind::Video
                    | crate::composition::ClipKind::Text
                    | crate::composition::ClipKind::Adjustment => {
                        crate::composition::TrackKind::Visual
                    }
                };
                if track.kind == expected {
                    if track.locked {
                        track_error =
                            Some("A locked destination track prevents moving this clip.".into());
                    } else {
                        updated.track_id = Some(track.id.clone());
                    }
                }
            }
            let timeline_delta = updated.timeline_start - old.timeline_start;
            let source_delta = updated.source_start - old.source_start;
            let duration_delta = updated.duration - old.duration;
            if track_error.is_some() {
                return track_error;
            }
            if drag.kind == TimelineDragKind::Move && old.group_id.is_some() {
                let mut candidate = project.clone();
                if let Err(error) = candidate.move_clip_to(&drag.clip_id, updated.timeline_start) {
                    track_error = Some(format!("{error:#}"));
                    return track_error;
                }
                if updated.track_id != old.track_id {
                    if let Some(moved) = candidate
                        .clips
                        .iter_mut()
                        .find(|clip| clip.id == drag.clip_id)
                    {
                        moved.track_id = updated.track_id;
                    }
                }
                if let Err(error) = candidate.validate() {
                    track_error = Some(format!("{error:#}"));
                    return track_error;
                }
                *project = candidate;
            } else {
                project.clips[index] = updated;
                if link_id.is_some() {
                    for sibling in project.clips.iter_mut().filter(|clip| {
                        clip.link_group_id.as_deref() == link_id.as_deref()
                            && clip.id != drag.clip_id
                    }) {
                        sibling.timeline_start += timeline_delta;
                        sibling.source_start += source_delta;
                        sibling.duration += duration_delta;
                    }
                }
            }
        }
    }
    track_error.or_else(|| project.validate().err().map(|error| format!("{error:#}")))
}

fn snap_timeline_time(
    project: &crate::composition::CompositionProject,
    time: f64,
    exclude_clip: Option<&str>,
    playhead: f64,
    pixels_per_second: f64,
    enabled: bool,
) -> f64 {
    if !enabled {
        return time.clamp(0.0, project.duration);
    }
    let frame = 1.0 / f64::from(project.fps);
    let framed = (time / frame).round() * frame;
    let mut best = framed;
    let mut distance = (framed - time).abs();
    let tolerance = 7.0 / pixels_per_second.max(1.0);
    let mut candidates = vec![0.0, project.duration, playhead];
    for clip in &project.clips {
        if Some(clip.id.as_str()) == exclude_clip {
            continue;
        }
        candidates.push(clip.timeline_start);
        candidates.push(clip.timeline_start + clip.duration);
    }
    for candidate in candidates {
        let candidate = (candidate / frame).round() * frame;
        let candidate_distance = (candidate - time).abs();
        if candidate_distance <= tolerance && candidate_distance < distance {
            best = candidate;
            distance = candidate_distance;
        }
    }
    best.clamp(0.0, project.duration)
}

fn format_timecode(time: f64, fps: u32) -> String {
    let total_frames = (time.max(0.0) * f64::from(fps)).round() as u64;
    let frames = total_frames % u64::from(fps.max(1));
    let seconds = total_frames / u64::from(fps.max(1));
    format!("{:02}:{:02}:{:02}", seconds / 60, seconds % 60, frames)
}

fn presentation_thumbnail_key(source: &crate::composition::MediaSource) -> Option<String> {
    if source.solid_color.is_some() || !source.has_video {
        return None;
    }
    let metadata = std::fs::metadata(&source.path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let path = source
        .path
        .canonicalize()
        .unwrap_or_else(|_| source.path.clone());
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.id.hash(&mut hasher);
    path.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    Some(format!("{}:{:016x}", source.id, hasher.finish()))
}

fn save_composition_preview_png(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;

    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .context("preview data is not a valid PNG image")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    if !parent.is_dir() {
        anyhow::bail!("snapshot directory does not exist: {}", parent.display());
    }
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .context("creating a staged composition snapshot")?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    staged
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("saving composition snapshot {}", path.display()))?;
    Ok(())
}

impl Default for PresentationStudioView {
    fn default() -> Self {
        let (audio_preferences, audio_preferences_error) =
            match crate::audio_input::load_preferences() {
                Ok(preferences) => (preferences, None),
                Err(error) => (
                    crate::audio_input::AudioInputPreferences::default(),
                    Some(format!("Could not load microphone preference: {error:#}")),
                ),
            };
        let (thumbnail_tx, thumbnail_rx) = mpsc::channel();
        Self {
            project: crate::composition::CompositionProject::new(),
            project_path: None,
            selected_source: None,
            media_search: String::new(),
            source_monitor_source: None,
            source_monitor_time: 0.0,
            source_in: 0.0,
            source_out: 5.0,
            source_edit_operation: SourceEditOperation::PlaceAtPlayhead,
            selected_clip: None,
            selected_clips: HashSet::new(),
            playhead: 0.0,
            output: PathBuf::from("output.mp4"),
            preview_texture: None,
            preview_png: None,
            source_preview_texture: None,
            thumbnail_cache: HashMap::new(),
            thumbnail_lru: VecDeque::new(),
            thumbnail_queue: VecDeque::new(),
            thumbnail_requested: HashSet::new(),
            thumbnail_failed: HashSet::new(),
            thumbnail_tx,
            thumbnail_rx,
            thumbnail_worker_active: false,
            job: None,
            message: "Create a composition or import ordinary media to begin.".into(),
            error: None,
            pending_keyframe_easing: crate::composition::Easing::Linear,
            selected_animated_property: crate::composition::AnimatedProperty::X,
            selected_title_preset: "builtin-centered-title".into(),
            selected_caption_preset: "builtin-short-form-caption".into(),
            presentation_template: None,
            template_source_bindings: HashMap::new(),
            template_text_values: HashMap::new(),
            template_responsive: true,
            transcription_language: "auto".into(),
            job_progress: None,
            title_preset_name: String::new(),
            selected_curve_keys: None,
            curve_drag_origin: None,
            keyframe_clipboard: None,
            audio_preferences,
            audio_preferences_error,
            audio_devices: Vec::new(),
            audio_devices_loaded: false,
            audio_device_check: Instant::now(),
            audio_capture: None,
            recoverable_takes: Vec::new(),
            playback: None,
            playback_start: None,
            record_voiceover_after_playback: false,
            timeline_zoom: 45.0,
            timeline_track_height: 52.0,
            timeline_snapping: true,
            scrubbing: false,
            timeline_drag_before: None,
            clip_clipboard: None,
            effect_clipboard: None,
            effect_stack_clipboard: None,
            solid_color_input: "20242C".into(),
            dirty: false,
            autosave_pending: false,
            last_autosave: Instant::now(),
            recovery_checked: false,
            recovery_snapshots: Vec::new(),
            edit_history: CompositionEditHistory::default(),
        }
    }
}

const SETTINGS_SEARCH_TARGETS: &[(&str, &str, usize)] = &[
    ("engine options chart options watch", "Engine options", 1),
    (
        "gcc interpreter fps resolution backend video",
        "Render / video settings",
        2,
    ),
    (
        "hud life score combo judgement asset profile",
        "Gameplay UI settings",
        3,
    ),
    ("watermark attribution moving mark", "Watermark settings", 4),
    ("notification discord desktop", "Notifications", 5),
    (
        "configuration config file profile",
        "Configuration & Profiles",
        0,
    ),
];

fn settings_search_results(query: &str) -> Vec<(&'static str, usize)> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    SETTINGS_SEARCH_TARGETS
        .iter()
        .filter(|(terms, title, _)| {
            title.to_ascii_lowercase().contains(&query)
                || terms
                    .split_ascii_whitespace()
                    .any(|term| term.contains(&query) || query.contains(term))
        })
        .map(|(_, title, tab)| (*title, *tab))
        .collect()
}

fn workflow_for_command(command: crate::workspace::CommandId) -> crate::workspace::WorkflowId {
    use crate::workspace::{CommandId as Command, WorkflowId as Workflow};
    match command {
        Command::GoHome => Workflow::Home,
        Command::GoQuickRender => Workflow::QuickRender,
        Command::GoChartStudio => Workflow::ChartStudio,
        Command::GoPresentationStudio | Command::GoVoiceoverWorkspace => {
            Workflow::PresentationStudio
        }
        Command::GoProduction => Workflow::Production,
        Command::GoRenderQueue | Command::GoRenderHistory => Workflow::Production,
        Command::GoTools => Workflow::Tools,
        Command::GoCacheManager
        | Command::GoPerformanceLab
        | Command::GoRegressionLab
        | Command::GoCompatibility
        | Command::GoProgramPreview => Workflow::Tools,
        Command::GoSettings => Workflow::Settings,
        Command::GoConfigurationSettings
        | Command::GoEngineOptionsSettings
        | Command::GoRenderSettings
        | Command::GoGameplayUiSettings
        | Command::GoWatermarkSettings
        | Command::GoNotificationSettings => Workflow::Settings,
        Command::GoAdvancedDiagnostics => Workflow::Diagnostics,
        _ => Workflow::Home,
    }
}

fn command_for_workflow_page(
    workflow: crate::workspace::WorkflowId,
    page: usize,
) -> Option<crate::workspace::CommandId> {
    use crate::workspace::{CommandId as Command, WorkflowId as Workflow};
    Some(match (workflow, page) {
        (Workflow::Production, 6) => Command::GoRenderQueue,
        (Workflow::Production, 7) => Command::GoRenderHistory,
        (Workflow::Tools, 8) => Command::GoCacheManager,
        (Workflow::Tools, 9) => Command::GoPerformanceLab,
        (Workflow::Tools, 10) => Command::GoRegressionLab,
        (Workflow::Tools, 11) => Command::GoCompatibility,
        (Workflow::Tools, 13) => Command::GoProgramPreview,
        (Workflow::Settings, 0) => Command::GoConfigurationSettings,
        (Workflow::Settings, 1) => Command::GoEngineOptionsSettings,
        (Workflow::Settings, 2) => Command::GoRenderSettings,
        (Workflow::Settings, 3) => Command::GoGameplayUiSettings,
        (Workflow::Settings, 4) => Command::GoWatermarkSettings,
        (Workflow::Settings, 5) => Command::GoNotificationSettings,
        _ => return None,
    })
}
enum ManifestJobOutcome {
    Loaded {
        path: PathBuf,
        manifest: Box<crate::render_manifest::RenderManifest>,
        report: Box<crate::render_manifest::ReproducibilityReport>,
    },
    Verified(crate::render_manifest::ReproducibilityReport),
    Exported {
        path: PathBuf,
        sha256: String,
    },
    ReproductionPrepared {
        config: Box<crate::render::RenderConfig>,
        manifest: Box<crate::render_manifest::RenderManifest>,
    },
    Compared(crate::render_manifest::ManifestComparison),
}
enum GuiRegressionOutcome {
    Updated(crate::regression::ReferenceUpdateReport),
    Compared(crate::regression::RegressionReport),
}
enum EngineJobMessage {
    CheckingCache {
        engine: PathBuf,
        graph_nodes: usize,
        eligible_operations: u64,
    },
    Compiling {
        engine: PathBuf,
        graph_nodes: usize,
        eligible_operations: u64,
    },
    Complete {
        engine: PathBuf,
        result: std::result::Result<EngineLoaded, String>,
    },
}
struct EngineLoaded {
    options: Option<Vec<serde_json::Value>>,
    nodes: Arc<Vec<crate::watch::EngineNode>>,
    gcc: Option<std::result::Result<crate::sono_gcc::SonoGccPrecompileReport, String>>,
}
#[derive(Clone, Default)]
enum GccUiStatus {
    #[default]
    Disabled,
    Loading,
    CheckingCache {
        graph_nodes: usize,
        eligible_operations: u64,
    },
    Compiling {
        graph_nodes: usize,
        eligible_operations: u64,
    },
    Ready(crate::sono_gcc::SonoGccPrecompileReport),
    Partial(crate::sono_gcc::SonoGccPrecompileReport),
    Failed {
        graph_nodes: usize,
        diagnostic: String,
    },
}

/// State owned by the Chart Studio workspace. Keeping chart editing state
/// together prevents its selections, drafts, workers and preview from being
/// mistaken for application-wide render state.
struct ChartStudioView {
    document: crate::chart_studio::ChartDocument,
    active: bool,
    pending_document: Option<crate::chart_studio::ChartDocument>,
    selected_entity: Option<usize>,
    selection: std::collections::BTreeSet<usize>,
    entity_draft: String,
    new_archetype: String,
    archetype_filter: String,
    engine: Option<ChartEngineSummary>,
    engine_job: Option<ChartEngineJob>,
    error: Option<String>,
    validation: Option<crate::chart_studio::LevelValidation>,
    schema_path: PathBuf,
    schema: Option<crate::chart_studio::AuthoringSchema>,
    schema_error: Option<String>,
    export_pending: Option<PathBuf>,
    export_error: Option<String>,
    last_export: Option<PathBuf>,
    preview_time: f64,
    preview_frame: Option<ChartPreviewFrame>,
    preview_texture: Option<egui::TextureHandle>,
    preview_error: Option<String>,
    preview_job: Option<ChartPreviewJob>,
    entity_clipboard: Vec<crate::formats::LevelEntity>,
    timeline_rows: Vec<(usize, f64, crate::chart_studio::TimingUnit)>,
    timeline_revision: u64,
    timeline_schema_identity: Option<String>,
    music_path: PathBuf,
    music_duration_seconds: f64,
    music_waveform: Vec<f32>,
    tempo_analysis: Option<crate::tempo_analysis::TempoAnalysis>,
    tempo_job: Option<ChartTempoJob>,
    tempo_generation: u64,
    tempo_error: Option<String>,
    selected_tempo_candidate: usize,
    tempo_bpm_edit: f64,
    tempo_phase_edit: f64,
    tap_times: VecDeque<Instant>,
    tap_tempo_bpm: Option<f64>,
    audio_playhead: f64,
    audio_generation: u64,
    audio_playback: Option<crate::timeline_playback::TimelinePlayback>,
    audio_playback_start: Option<ChartAudioPlaybackStart>,
}

impl Default for ChartStudioView {
    fn default() -> Self {
        Self {
            document: Default::default(),
            active: false,
            pending_document: None,
            selected_entity: None,
            selection: Default::default(),
            entity_draft: String::new(),
            new_archetype: String::new(),
            archetype_filter: String::new(),
            engine: None,
            engine_job: None,
            error: None,
            validation: None,
            schema_path: PathBuf::new(),
            schema: None,
            schema_error: None,
            export_pending: None,
            export_error: None,
            last_export: None,
            preview_time: 0.0,
            preview_frame: None,
            preview_texture: None,
            preview_error: None,
            preview_job: None,
            entity_clipboard: Vec::new(),
            timeline_rows: Vec::new(),
            timeline_revision: u64::MAX,
            timeline_schema_identity: None,
            music_path: PathBuf::new(),
            music_duration_seconds: 0.0,
            music_waveform: Vec::new(),
            tempo_analysis: None,
            tempo_job: None,
            tempo_generation: 0,
            tempo_error: None,
            selected_tempo_candidate: 0,
            tempo_bpm_edit: 120.0,
            tempo_phase_edit: 0.0,
            tap_times: VecDeque::new(),
            tap_tempo_bpm: None,
            audio_playhead: 0.0,
            audio_generation: 0,
            audio_playback: None,
            audio_playback_start: None,
        }
    }
}

struct Desktop {
    parent: Option<Arc<winit::window::Window>>,
    ui_font_locale: String,
    config: RenderConfig,
    tab: usize,
    live: Arc<Mutex<LiveState>>,
    job: Option<Job>,
    summary: String,
    error: bool,
    compatibility_report: Option<crate::compatibility::CompatibilityReport>,
    compatibility_error: Option<String>,
    compatibility_dynamic: bool,
    last_diagnostic: Option<crate::render_diagnostics::RenderDiagnosticRecord>,
    options: Vec<serde_json::Value>,
    engine_job: Option<EngineJob>,
    options_engine: PathBuf,
    loaded_watch: Option<(PathBuf, Arc<Vec<crate::watch::EngineNode>>)>,
    gcc_status: GccUiStatus,
    show_gcc_details: bool,
    arbitrary_index: usize,
    arbitrary_value: f64,
    close_pending: bool,
    chart_close_prompt: bool,
    presentation_close_prompt: bool,
    presentation_close_checkpointed: bool,
    job_manager: Option<Arc<crate::job_manager::RenderJobManager>>,
    job_manager_error: Option<String>,
    admin_server: Option<crate::admin::AdminServerHandle>,
    admin_server_error: Option<String>,
    admin_config: crate::admin::AdminServerConfig,
    admin_settings_message: Option<String>,
    selected_job: Option<String>,
    profile_manager: Option<crate::config_profiles::ConfigProfileManager>,
    profile_manager_error: Option<String>,
    active_profile_id: Option<String>,
    last_profile_sync: Instant,
    profile_name_input: String,
    confirm_profile_delete: bool,
    confirm_profile_reset: bool,
    gameplay_ui_asset_pack: Option<crate::presentation_assets::PresentationAssetPack>,
    gameplay_ui_import_report: Option<crate::render_ui::AssetPackImportReport>,
    gameplay_ui_selected_element: Option<String>,
    gameplay_ui_preview_texture: Option<egui::TextureHandle>,
    gameplay_ui_asset_thumbnail_cache: HashMap<String, egui::TextureHandle>,
    gameplay_ui_new_glyph_character: String,
    gameplay_ui_new_glyph_asset: Option<String>,
    cache_inventory: Option<crate::cache_manager::CacheInventory>,
    cache_selection: Option<String>,
    cache_limit_gib: u64,
    cache_manager_error: Option<String>,
    performance_store: Option<crate::performance::PerformanceStore>,
    diagnostic_store: Option<crate::render_diagnostics::RenderDiagnosticStore>,
    performance_records: Vec<crate::performance::RenderPerformanceRecord>,
    performance_latest_report: Option<crate::performance::BenchmarkReport>,
    performance_error: Option<String>,
    performance_job: Option<PerformanceJob>,
    performance_warmups: usize,
    performance_runs: usize,
    performance_compare_modes: bool,
    performance_measure_overhead: bool,
    performance_summary: String,
    regression_reference: PathBuf,
    regression_fixture_id: String,
    regression_checkpoint_time: f64,
    regression_compare_vm_gcc: bool,
    regression_include_media: bool,
    regression_timing_samples: usize,
    regression_job: Option<RegressionJob>,
    regression_summary: String,
    regression_error: Option<String>,
    regression_latest_report: Option<crate::regression::RegressionReport>,
    project: Option<crate::project::RenderProject>,
    project_path: Option<PathBuf>,
    project_error: Option<String>,
    project_recents: crate::project::RecentProjects,
    project_recents_path: PathBuf,
    input_discovery: Option<crate::project::InputDiscovery>,
    input_discovery_selection: std::collections::BTreeMap<crate::project::InputRole, PathBuf>,
    last_dropped_files: Option<Vec<PathBuf>>,
    history_store: Option<crate::render_history::RenderHistoryStore>,
    history_error: Option<String>,
    history_entries: Vec<crate::render_history::RenderHistoryRecord>,
    history_search: String,
    history_status: Option<crate::render_history::HistoryStatus>,
    history_engine_filter: String,
    history_project_filter: String,
    history_backend_filter: String,
    selected_history_id: Option<String>,
    history_rerender_config: Option<RenderConfig>,
    history_rerender_source: Option<String>,
    history_retention_max: usize,
    history_retention_age_days: u64,
    history_retention_use_age: bool,
    history_keep_failures: bool,
    history_keep_pinned: bool,
    manifest: Option<crate::render_manifest::RenderManifest>,
    manifest_path: Option<PathBuf>,
    manifest_asset_overrides: std::collections::BTreeMap<String, PathBuf>,
    manifest_report: Option<crate::render_manifest::ReproducibilityReport>,
    manifest_comparison: Option<crate::render_manifest::ManifestComparison>,
    manifest_error: Option<String>,
    manifest_job: Option<ManifestJob>,
    media_acquisition_requested: bool,
    media_acquisition_url: String,
    media_acquisition_inspected_url: Option<String>,
    media_acquisition_mode: crate::media_acquisition::AcquisitionMode,
    media_acquisition_info: Option<crate::media_acquisition::MediaInfo>,
    media_acquisition_error: Option<String>,
    media_acquisition_job: Option<MediaAcquisitionJob>,
    media_acquisition_progress: Arc<Mutex<crate::media_acquisition::AcquisitionProgress>>,
    preview_time: f64,
    preview_frame: Option<ProgramPreviewFrame>,
    preview_texture: Option<egui::TextureHandle>,
    preview_error: Option<String>,
    preview_job: Option<ProgramPreviewJob>,
    preview_queued: Option<(u64, f64, RenderConfig)>,
    preview_generation: u64,
    chart_studio: ChartStudioView,
    presentation_studio: PresentationStudioView,
    onboarding_preferences: crate::onboarding::OnboardingPreferences,
    onboarding_path: PathBuf,
    onboarding_error: Option<String>,
    onboarding_write_blocked: bool,
    welcome_visible: bool,
    quick_render_active: bool,
    quick_job_id: Option<String>,
    quick_validation_job: Option<QuickValidationJob>,
    quick_validation_result: Option<std::result::Result<QuickValidationReport, String>>,
    quick_validation_config: Option<RenderConfig>,
    help_open: bool,
    help_search: String,
    presentation_tour_open: bool,
    presentation_tour_step: usize,
    settings_search: String,
    workspace_preferences: crate::workspace::WorkspacePreferences,
    workspace_preferences_path: PathBuf,
    workspace_preferences_error: Option<String>,
    workspace_name_input: String,
    command_palette_open: bool,
    command_palette_query: String,
    command_palette_focus: bool,
    shortcut_editor_open: bool,
    shortcut_search: String,
    shortcut_capture: Option<crate::workspace::CommandId>,
    shortcut_conflict: Option<(String, crate::workspace::ShortcutBinding, Vec<String>)>,
    pending_keyframe_value: Option<f64>,
}
impl Default for Desktop {
    fn default() -> Self {
        let mut config = RenderConfig::default();
        // Preserve the desktop application's existing first-run defaults.
        config.width = 1920;
        config.height = 1080;
        config.fps = 60;
        config.duration = 20.0;
        // The GUI prepares Sono-GCC in the background when selected, so its
        // export worker does not synchronously compile on a cache miss.
        config.compile_sono_gcc_on_demand = false;
        #[cfg(test)]
        let test_id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        #[cfg(test)]
        let manager_path =
            std::env::temp_dir().join(format!("sono-renderer-gui-test-{test_id}.json"));
        #[cfg(not(test))]
        let manager_path = crate::job_manager::default_store_path();
        #[cfg(test)]
        let profile_path =
            std::env::temp_dir().join(format!("sono-renderer-profile-gui-test-{test_id}.json"));
        #[cfg(not(test))]
        let profile_path = crate::config_profiles::default_profile_store_path();
        let profile_open = crate::config_profiles::ConfigProfileManager::open_at(profile_path);
        let (profile_manager, profile_manager_error) = match profile_open {
            Ok(manager) => (Some(manager), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        #[cfg(test)]
        let performance_path =
            std::env::temp_dir().join(format!("sono-renderer-performance-gui-test-{test_id}.json"));
        #[cfg(not(test))]
        let performance_path = crate::performance::default_store_path();
        let performance_open = crate::performance::PerformanceStore::open_at(
            performance_path.clone(),
            crate::performance::DEFAULT_RETENTION,
        );
        let (performance_store, performance_error, performance_records) = match performance_open {
            Ok(store) => {
                let records = store.list(20).unwrap_or_default();
                (Some(store), None, records)
            }
            Err(error) => (None, Some(format!("{error:#}")), Vec::new()),
        };
        let diagnostic_open = crate::render_diagnostics::RenderDiagnosticStore::open_at(
            performance_path.join("diagnostics"),
            crate::render_diagnostics::DEFAULT_DIAGNOSTIC_RETENTION,
        );
        let diagnostic_store = diagnostic_open.ok();
        let active_profile = profile_manager
            .as_ref()
            .and_then(|manager| manager.active().ok().flatten());
        let active_profile_id = active_profile.as_ref().map(|profile| profile.id.clone());
        let profile_name_input = active_profile
            .as_ref()
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        if let Some(profile) = active_profile {
            config = profile.config;
        }
        let manager_open =
            crate::job_manager::RenderJobManager::open_at(manager_path, config.job_manager.clone())
                .and_then(|manager| {
                    if active_profile_id.is_some() {
                        manager.configure(config.job_manager.clone())?;
                    }
                    manager.start()?;
                    Ok(manager)
                });
        let (job_manager, job_manager_error) = match manager_open {
            Ok(manager) => (Some(Arc::new(manager)), None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        #[cfg(not(test))]
        let admin_server_open =
            crate::admin::AdminServerConfig::load_default().and_then(|admin_config| {
                match job_manager.as_ref() {
                    Some(manager) => crate::admin::start(&admin_config, manager.clone()),
                    None if !admin_config.enabled => Ok(None),
                    None => anyhow::bail!(
                        "admin service is enabled but the render job manager did not start"
                    ),
                }
            });
        #[cfg(test)]
        let admin_server_open: anyhow::Result<Option<crate::admin::AdminServerHandle>> = Ok(None);
        let (admin_server, admin_server_error) = match admin_server_open {
            Ok(server) => (server, None),
            Err(error) => (None, Some(format!("{error:#}"))),
        };
        let admin_config = crate::admin::AdminServerConfig::load_default().unwrap_or_default();
        if let Some(manager) = &job_manager {
            config.job_manager = manager.config();
        }
        let history_open = job_manager
            .as_ref()
            .map(|manager| manager.history_store())
            .unwrap_or_else(crate::render_history::RenderHistoryStore::open_default);
        let (history_store, history_error, history_entries) = match history_open {
            Ok(store) => {
                let page = store.list(&crate::render_history::HistoryQuery {
                    limit: Some(100),
                    ..Default::default()
                });
                let error = page
                    .issues
                    .first()
                    .map(|issue| format!("{}: {}", issue.path.display(), issue.message));
                let entries = page.entries;
                (Some(store), error, entries)
            }
            Err(error) => (None, Some(format!("{error:#}")), Vec::new()),
        };
        let history_retention_max = history_store
            .as_ref()
            .and_then(|store| store.retention().ok())
            .map(|policy| policy.max_entries)
            .unwrap_or(crate::render_history::DEFAULT_MAX_ENTRIES);
        let history_policy = history_store
            .as_ref()
            .and_then(|store| store.retention().ok())
            .unwrap_or_default();
        #[cfg(test)]
        let onboarding_path = std::env::temp_dir()
            .join(format!("sono-renderer-gui-test-{test_id}"))
            .join("onboarding.json");
        #[cfg(not(test))]
        let onboarding_path = crate::onboarding::default_path();
        let (onboarding_preferences, onboarding_error) =
            match crate::onboarding::OnboardingPreferences::load(&onboarding_path) {
                Ok(preferences) => (preferences, None),
                Err(error) => (Default::default(), Some(format!("{error:#}"))),
            };
        let onboarding_write_blocked = onboarding_error.is_some();
        let welcome_visible = onboarding_preferences.should_show_welcome();
        let quick_job_id = onboarding_preferences.first_successful_job_id.clone();
        #[cfg(test)]
        let workspace_preferences_path = std::env::temp_dir()
            .join(format!("sono-renderer-gui-test-{test_id}"))
            .join("workspace_preferences.json");
        #[cfg(not(test))]
        let workspace_preferences_path = crate::workspace::default_path();
        let (mut workspace_preferences, workspace_preferences_error) =
            match crate::workspace::WorkspacePreferences::load(&workspace_preferences_path) {
                Ok(preferences) => (preferences, None),
                Err(error) => (Default::default(), Some(format!("{error:#}"))),
            };
        let mut presentation_studio = PresentationStudioView::default();
        presentation_studio.timeline_zoom = workspace_preferences.timeline_zoom as f32;
        presentation_studio.timeline_track_height =
            workspace_preferences.timeline_track_height as f32;
        let workflow = if welcome_visible {
            crate::workspace::WorkflowId::Home
        } else {
            workspace_preferences.workflow
        };
        if welcome_visible {
            workspace_preferences.workflow = crate::workspace::WorkflowId::Home;
        }
        let (tab, quick_render_active) = match workflow {
            crate::workspace::WorkflowId::Home => (0, false),
            crate::workspace::WorkflowId::QuickRender => (0, true),
            crate::workspace::WorkflowId::ChartStudio => (14, false),
            crate::workspace::WorkflowId::PresentationStudio => (15, false),
            crate::workspace::WorkflowId::Production => (6, false),
            crate::workspace::WorkflowId::Tools => (8, false),
            crate::workspace::WorkflowId::Settings => (1, false),
            crate::workspace::WorkflowId::Diagnostics => (12, false),
        };
        Self {
            parent: None,
            ui_font_locale: String::new(),
            config,
            tab,
            live: Default::default(),
            job: None,
            summary: "Select your project files to begin.".into(),
            error: false,
            compatibility_report: None,
            compatibility_error: None,
            compatibility_dynamic: false,
            last_diagnostic: None,
            options: Vec::new(),
            engine_job: None,
            options_engine: PathBuf::new(),
            loaded_watch: None,
            gcc_status: GccUiStatus::Disabled,
            show_gcc_details: false,
            arbitrary_index: 0,
            arbitrary_value: 0.0,
            close_pending: false,
            chart_close_prompt: false,
            presentation_close_prompt: false,
            presentation_close_checkpointed: false,
            job_manager,
            job_manager_error,
            admin_server,
            admin_server_error,
            admin_config,
            admin_settings_message: None,
            selected_job: None,
            profile_manager,
            profile_manager_error,
            active_profile_id,
            last_profile_sync: Instant::now(),
            profile_name_input,
            confirm_profile_delete: false,
            confirm_profile_reset: false,
            gameplay_ui_asset_pack: None,
            gameplay_ui_import_report: None,
            gameplay_ui_selected_element: None,
            gameplay_ui_preview_texture: None,
            gameplay_ui_asset_thumbnail_cache: HashMap::new(),
            gameplay_ui_new_glyph_character: String::new(),
            gameplay_ui_new_glyph_asset: None,
            cache_inventory: None,
            cache_selection: None,
            cache_limit_gib: 10,
            cache_manager_error: None,
            performance_store,
            diagnostic_store,
            performance_records,
            performance_latest_report: None,
            performance_error,
            performance_job: None,
            performance_warmups: 1,
            performance_runs: 3,
            performance_compare_modes: false,
            performance_measure_overhead: false,
            performance_summary: "No benchmark has been run in this session.".into(),
            regression_reference: PathBuf::from("artifacts/regression-lab/references/current"),
            regression_fixture_id: "current-workload".into(),
            regression_checkpoint_time: 0.0,
            regression_compare_vm_gcc: false,
            regression_include_media: false,
            regression_timing_samples: 3,
            regression_job: None,
            regression_summary: "Choose a reference directory, then run or explicitly update it."
                .into(),
            regression_error: None,
            regression_latest_report: None,
            project: None,
            project_path: None,
            project_error: None,
            project_recents_path: crate::project::default_recent_projects_path(),
            project_recents: crate::project::RecentProjects::open(
                crate::project::default_recent_projects_path(),
            ),
            input_discovery: None,
            input_discovery_selection: std::collections::BTreeMap::new(),
            last_dropped_files: None,
            history_store,
            history_error,
            history_entries,
            history_search: String::new(),
            history_status: None,
            history_engine_filter: String::new(),
            history_project_filter: String::new(),
            history_backend_filter: String::new(),
            selected_history_id: None,
            history_rerender_config: None,
            history_rerender_source: None,
            history_retention_max,
            history_retention_age_days: history_policy.max_age_days.unwrap_or(365),
            history_retention_use_age: history_policy.max_age_days.is_some(),
            history_keep_failures: history_policy.keep_failures,
            history_keep_pinned: history_policy.keep_pinned,
            manifest: None,
            manifest_path: None,
            manifest_asset_overrides: std::collections::BTreeMap::new(),
            manifest_report: None,
            manifest_comparison: None,
            manifest_error: None,
            manifest_job: None,
            media_acquisition_requested: false,
            media_acquisition_url: String::new(),
            media_acquisition_inspected_url: None,
            media_acquisition_mode: crate::media_acquisition::AcquisitionMode::Audio,
            media_acquisition_info: None,
            media_acquisition_error: None,
            media_acquisition_job: None,
            media_acquisition_progress: Arc::new(Mutex::new(
                crate::media_acquisition::AcquisitionProgress::default(),
            )),
            preview_time: 0.0,
            preview_frame: None,
            preview_texture: None,
            preview_error: None,
            preview_job: None,
            preview_queued: None,
            preview_generation: 0,
            chart_studio: ChartStudioView::default(),
            presentation_studio,
            onboarding_preferences,
            onboarding_path,
            onboarding_error,
            onboarding_write_blocked,
            welcome_visible: welcome_visible || workflow == crate::workspace::WorkflowId::Home,
            quick_render_active,
            quick_job_id,
            quick_validation_job: None,
            quick_validation_result: None,
            quick_validation_config: None,
            help_open: false,
            help_search: String::new(),
            presentation_tour_open: false,
            presentation_tour_step: 0,
            settings_search: String::new(),
            workspace_preferences,
            workspace_preferences_path,
            workspace_preferences_error,
            workspace_name_input: String::new(),
            command_palette_open: false,
            command_palette_query: String::new(),
            command_palette_focus: false,
            shortcut_editor_open: false,
            shortcut_search: String::new(),
            shortcut_capture: None,
            shortcut_conflict: None,
            pending_keyframe_value: None,
        }
    }
}
fn apply_selection(path: &mut PathBuf, selection: Option<PathBuf>) {
    if let Some(selected) = selection {
        *path = selected;
    }
}
fn path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut PathBuf,
    extensions: &[&str],
    save: bool,
    parent: Option<&winit::window::Window>,
) {
    let help = match label {
        "Engine ZIP" => "Required. The Sonolus engine defines how this level behaves and renders.",
        "Level" => "Required. This is the Sonolus level or chart data to render.",
        "Skin / SCP" => "Required. The engine resource package supplies the skin and assets used by the level.",
        "Music / BGM" => "Optional if Include music is off. Otherwise select the chart's music file.",
        "MV (optional)" => "Optional background video shown behind gameplay. Supported containers depend on FFmpeg; common choices are MP4, WebM, MKV, and MOV.",
        "Output MP4" => "Choose where the finished video will be saved. Existing files are not overwritten by queued jobs.",
        "Particle SCP (optional)" => "Optional particle asset package override. The engine's default package is used when this is empty.",
        _ => "Choose the file used for this render input.",
    };
    ui.label(label).on_hover_text(help);
    let mut text = path.to_string_lossy().into_owned();
    if ui
        .add_sized(
            [
                (ui.ctx().content_rect().width() - 290.0).clamp(300.0, 560.0),
                24.0,
            ],
            egui::TextEdit::singleline(&mut text),
        )
        .changed()
    {
        *path = PathBuf::from(text);
    }
    if ui
        .button(if save { "Save as..." } else { "Browse..." })
        .clicked()
    {
        let mut dialog = rfd::FileDialog::new()
            .set_title(label)
            .add_filter(label, extensions)
            .add_filter("All files", &["*"]);
        if let Some(parent) = parent {
            dialog = dialog.set_parent(parent);
        }
        if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            dialog = dialog.set_directory(parent);
        }
        let selected = if save {
            dialog
                .set_file_name(if path.as_os_str().is_empty() {
                    "render.mp4"
                } else {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("render.mp4")
                })
                .save_file()
        } else {
            dialog.pick_file()
        };
        apply_selection(path, selected);
    }
    ui.end_row();
}
fn optional_path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut Option<PathBuf>,
    ext: &[&str],
    save: bool,
    parent: Option<&winit::window::Window>,
) {
    let mut value = path.clone().unwrap_or_default();
    path_row(ui, label, &mut value, ext, save, parent);
    *path = (!value.as_os_str().is_empty()).then_some(value);
}
fn obvious_validation(config: &RenderConfig) -> Result<()> {
    config.validate()?;
    for (name, path) in [
        ("engine", &config.engine),
        ("level", &config.level),
        ("resources", &config.resources),
    ] {
        if !path.is_file() {
            bail!(
                "{}",
                friendly_beginner_error(&format!(
                    "{name} input is missing or not a file: {}",
                    path.display()
                ))
            );
        }
    }
    if config
        .output
        .as_ref()
        .is_none_or(|p| p.as_os_str().is_empty())
    {
        bail!("Choose where to save the output MP4.");
    }
    if config.layers.bgm && config.music.as_ref().is_none_or(|p| !p.is_file()) {
        bail!("Music is enabled, but no readable music file is selected. Choose Music / BGM or turn off Include music.");
    }
    if config.width % 2 != 0 || config.height % 2 != 0 {
        bail!("MP4 requires even width and height");
    }
    Ok(())
}

fn friendly_beginner_error(details: &str) -> String {
    let lower = details.to_ascii_lowercase();
    if lower.contains("ffmpeg") || lower.contains("ffprobe") {
        "Video encoding could not use FFmpeg. Check the FFmpeg and output details below, then retry. On Windows, the renderer can provision its pinned FFmpeg on first export; `ffmpeg-info` shows the resolved tools.".into()
    } else if lower.contains("engine input") || lower.contains("engine:") {
        "Choose a readable Sonolus engine ZIP. The engine defines how the level behaves and renders.".into()
    } else if lower.contains("level input") || lower.contains("level:") {
        "Choose a readable Sonolus level or chart file.".into()
    } else if lower.contains("resources input") || lower.contains("skin / scp") {
        "Choose the engine resource package (usually an SCP file or ZIP).".into()
    } else if lower.contains("bgm") || lower.contains("music") {
        "Music is enabled, but a readable music file is missing. Select Music / BGM or turn off Include music.".into()
    } else if lower.contains("output") || lower.contains("mp4") {
        "Choose a new output MP4 path. Queued renders do not overwrite existing files.".into()
    } else if lower.contains("not a directory") || lower.contains("output folder") {
        "Choose an output path inside a folder that exists, or create the folder first.".into()
    } else {
        details
            .lines()
            .next()
            .unwrap_or("Check the selected inputs and try again.")
            .to_owned()
    }
}
impl Desktop {
    fn start(&mut self, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        if let Err(error) = obvious_validation(&self.config) {
            self.summary = error.to_string();
            self.error = true;
            self.live.lock().unwrap().log(format!("{error:#}"));
            return;
        }
        *self.live.lock().unwrap() = LiveState::default();
        let log = self.live.clone();
        let wake = ctx.clone();
        let control = ExportControl::with_log(move |message| {
            log.lock().unwrap().log(message);
            wake.request_repaint();
        });
        let token = control.clone();
        let live = self.live.clone();
        let config = Self::gui_export_config(&self.config);
        let config_snapshot = config.clone();
        let wake = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let progress = live.clone();
            let paint = wake.clone();
            let result = catch_export_panic(|| {
                config.render_video_controlled(
                    Box::new(move |event| {
                        progress.lock().unwrap().progress(event);
                        paint.request_repaint();
                    }),
                    token,
                )
            })
            .map_err(|error| {
                let phase = live.lock().unwrap().progress.as_ref().map(|p| p.phase);
                error.context(format!(
                    "GUI export during {phase:?}; engine={}, level={}, output={}",
                    config.engine.display(),
                    config.level.display(),
                    config
                        .output
                        .as_deref()
                        .unwrap_or_else(|| std::path::Path::new(""))
                        .display()
                ))
            });
            let result = result.map(|report| {
                (
                    format!(
                        "Finished: {} frames, {:.2}s. {}",
                        report.submitted_frames,
                        report.timeline.duration,
                        config.output.unwrap().display()
                    ),
                    report.diagnostics,
                )
            });
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.job = Some(Job {
            control,
            result: rx,
            worker,
            config_snapshot,
        });
        self.error = false;
        self.summary = "Export running".into();
    }

    fn gui_export_config(config: &RenderConfig) -> RenderConfig {
        config.clone()
    }

    fn config_file_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Load configuration...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Load Sono-Renderer configuration")
                    .add_filter("Renderer configuration", &["json"]);
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.pick_file() {
                    match RenderConfig::load_config(&path) {
                        Ok(config) => {
                            if let Some(manager) = &self.job_manager {
                                if let Err(error) = manager.configure(config.job_manager.clone()) {
                                    self.summary = format!("Queue configuration failed: {error:#}");
                                    self.error = true;
                                    return;
                                }
                            }
                            self.config = config;
                            self.summary = format!("Loaded configuration: {}", path.display());
                            self.error = false;
                        }
                        Err(error) => {
                            self.summary = format!("Configuration load failed: {error:#}");
                            self.error = true;
                        }
                    }
                }
            }
            if ui.button("Save configuration...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Save Sono-Renderer configuration")
                    .add_filter("Renderer configuration", &["json"])
                    .set_file_name("sono-renderer.json");
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.save_file() {
                    match self.config.save_config(&path) {
                        Ok(()) => {
                            self.summary = format!("Saved configuration: {}", path.display());
                            self.error = false;
                        }
                        Err(error) => {
                            self.summary = format!("Configuration save failed: {error:#}");
                            self.error = true;
                        }
                    }
                }
            }
        });
    }

    fn active_project_profile_ref(&self) -> Option<crate::project::ProjectProfileRef> {
        let id = self.active_profile_id.clone()?;
        let name = self.profile_manager.as_ref()?.get(&id).ok()?.name;
        Some(crate::project::ProjectProfileRef {
            id: Some(id),
            name: Some(name),
            ..crate::project::ProjectProfileRef::default()
        })
    }

    fn sync_project_from_config(&mut self) {
        let previous = self.project.clone();
        let mut project = crate::project::RenderProject::from_config(
            &self.config,
            previous
                .as_ref()
                .map(|project| project.profile.clone())
                .unwrap_or_else(|| self.active_project_profile_ref()),
        );
        if let Some(previous) = previous {
            project.metadata = previous.metadata;
            project.inputs.cover = previous.inputs.cover;
            project.inputs.extra = previous.inputs.extra;
            project.extra = previous.extra;
            project.overrides.output = previous.overrides.output;
            project.overrides.start_time = previous.overrides.start_time;
            project.overrides.duration = previous.overrides.duration;
            project.overrides.whole_chart = previous.overrides.whole_chart;
            project.overrides.fps = previous.overrides.fps;
            project.overrides.width = previous.overrides.width;
            project.overrides.height = previous.overrides.height;
            project.overrides.layers = previous.overrides.layers;
            project.overrides.apply_engine_background_rotation =
                previous.overrides.apply_engine_background_rotation;
            project.overrides.extra = previous.overrides.extra;
            if previous.profile.is_some() {
                project.profile = previous.profile;
            }
        }
        self.project = Some(project);
    }

    fn new_project(&mut self) {
        self.project = Some(crate::project::RenderProject::from_config(
            &self.config,
            self.active_project_profile_ref(),
        ));
        self.project_path = None;
        self.project_error = None;
        self.summary = "New project created from the current inputs and render settings. Save it to make relative paths portable.".into();
        self.error = false;
    }

    fn open_project(&mut self, path: PathBuf) {
        let loaded = crate::project::RenderProject::load(&path);
        match loaded {
            Ok(project) => {
                self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                self.input_discovery = None;
                self.input_discovery_selection.clear();
                match project.resolve_config(
                    &path,
                    &self.config,
                    self.profile_manager.as_ref(),
                    None,
                ) {
                    Ok((config, profile)) => {
                        self.config = config;
                        if let Some(profile) = profile {
                            self.active_profile_id = Some(profile.id.clone());
                            self.profile_name_input = profile.name;
                        }
                        self.project_error = None;
                        self.error = false;
                        self.summary = format!("Opened project: {}", path.display());
                    }
                    Err(error) => {
                        // Keep the project open for inspection and repair even
                        // when its required profile is unavailable.
                        let paths = project.resolved_inputs(
                            path.parent().unwrap_or_else(|| std::path::Path::new(".")),
                        );
                        self.config.engine = paths.engine.unwrap_or_default();
                        self.config.level = paths.level.unwrap_or_default();
                        self.config.resources = paths.resources.unwrap_or_default();
                        self.config.music = paths.music;
                        self.config.mv = paths.mv;
                        self.config.resource_overrides.particles = paths.particle_resources;
                        self.config.resource_overrides.effects = paths.effects_resources;
                        self.config.resource_overrides.background = paths.background_resources;
                        self.project_error =
                            Some(format!("Project configuration needs attention: {error:#}"));
                        self.error = true;
                        self.summary =
                            "Project opened, but its required configuration could not be resolved."
                                .into();
                    }
                }
                if let Some(snapshot) = project.extra.get(crate::chart_studio::PROJECT_LEVEL_KEY) {
                    match crate::chart_studio::read_project_chart_snapshot(snapshot) {
                        Ok(snapshot) => {
                            let source = project
                                .resolved_inputs(
                                    path.parent().unwrap_or_else(|| std::path::Path::new(".")),
                                )
                                .level;
                            let mut document = crate::chart_studio::ChartDocument::with_beat_grid(
                                snapshot.level_data,
                                snapshot.beat_grid,
                            )
                            .expect("validated project chart grid");
                            document.mark_saved(source);
                            if self.chart_studio.document.is_dirty() {
                                self.chart_studio.pending_document = Some(document);
                            } else {
                                self.install_chart_document(document);
                            }
                        }
                        Err(error) => {
                            self.chart_studio.error = Some(format!(
                                "Project chart snapshot could not be opened: {error}"
                            ))
                        }
                    }
                }
                self.project = Some(project);
                self.project_path = Some(path.clone());
                self.project_recents.touch(&path);
                let _ = self.project_recents.save(&self.project_recents_path);
            }
            Err(error) => {
                self.project_error = Some(format!("Could not open project: {error:#}"));
                self.summary = "Project could not be opened.".into();
                self.error = true;
            }
        }
    }

    fn save_project_as(&mut self, path: PathBuf) {
        if self.project.is_none() {
            self.new_project();
        }
        self.sync_project_from_config();
        if self.chart_studio.active {
            let previous_snapshot = self
                .project
                .as_ref()
                .and_then(|project| project.extra.get(crate::chart_studio::PROJECT_LEVEL_KEY))
                .cloned();
            match crate::chart_studio::project_level_snapshot(
                &self.chart_studio.document.level,
                self.chart_studio.document.beat_grid.as_ref(),
                previous_snapshot.as_ref(),
            ) {
                Ok(snapshot) => {
                    if let Some(project) = self.project.as_mut() {
                        project
                            .extra
                            .insert(crate::chart_studio::PROJECT_LEVEL_KEY.into(), snapshot);
                    }
                }
                Err(error) => {
                    self.project_error = Some(format!(
                        "Project chart snapshot could not be serialized: {error:#}"
                    ));
                    return;
                }
            }
        }
        let result = self.project.as_mut().unwrap().save(&path);
        match result {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.project_recents.touch(&path);
                let recent_result = self.project_recents.save(&self.project_recents_path);
                self.project_error = recent_result.err().map(|error| {
                    format!("Project saved, but recent list could not be updated: {error:#}")
                });
                self.summary = format!("Saved project: {}", path.display());
                self.error = self.project_error.is_some();
            }
            Err(error) => {
                self.project_error = Some(format!("Project save failed: {error:#}"));
                self.summary = "Project was not saved.".into();
                self.error = true;
            }
        }
    }

    fn save_project(&mut self) {
        if let Some(path) = self.project_path.clone() {
            self.save_project_as(path);
            return;
        }
        self.pick_save_project();
    }

    fn pick_open_project(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open Sono-Renderer project")
            .add_filter("Sono-Renderer project", &["sonoproject", "json"]);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_project(path);
        }
    }

    fn pick_save_project(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Sono-Renderer project")
            .add_filter("Sono-Renderer project", &["sonoproject"])
            .set_file_name("render.sonoproject");
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.save_file() {
            self.save_project_as(path);
        }
    }

    fn apply_project_config(&mut self) -> Result<Option<crate::config_profiles::ConfigProfile>> {
        self.sync_project_from_config();
        let project = self
            .project
            .as_ref()
            .context("create or open a project first")?;
        let path = self
            .project_path
            .as_ref()
            .context("save the project before creating its render job")?;
        let (config, profile) =
            project.resolve_config(path, &self.config, self.profile_manager.as_ref(), None)?;
        self.config = config;
        Ok(profile)
    }

    fn create_project_job(&mut self) {
        let result = (|| -> Result<String> {
            let profile = self.apply_project_config()?;
            obvious_validation(&self.config)?;
            self.config.validate_export_inputs()?;
            let manager = self
                .job_manager
                .as_ref()
                .context("render queue is unavailable")?;
            let project_path = self.project_path.clone();
            let id = manager.enqueue_with_metadata(
                self.config.clone(),
                None,
                profile.map(|profile| (profile.id, profile.name)),
                crate::job_manager::RenderJobMetadata {
                    project_path,
                    ..Default::default()
                },
            )?;
            Ok(id)
        })();
        match result {
            Ok(id) => {
                self.summary = format!("Created render job {id} from the saved project snapshot.");
                self.error = false;
            }
            Err(error) => {
                self.project_error = Some(format!("Could not create render job: {error:#}"));
                self.summary = "Project validation prevented job creation.".into();
                self.error = true;
            }
        }
    }

    fn process_dropped_files(&mut self, files: Vec<PathBuf>) {
        if files.len() == 1
            && files[0]
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sonocomp"))
        {
            let path = files[0].clone();
            self.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);
            self.presentation_load_project(path);
            return;
        }
        if files.len() == 1 && crate::project::RenderProject::load(&files[0]).is_ok() {
            let path = files[0].clone();
            self.open_project(path);
            return;
        }
        if self.tab == 15 {
            self.presentation_import_dropped_files(&files);
            return;
        }
        let discovery = crate::project::discover_inputs(&files);
        if self.tab == 14 {
            if let Some(levels) = discovery.candidates.get(&crate::project::InputRole::Level) {
                if levels.len() == 1 {
                    let path = levels[0].clone();
                    self.load_chart_level_path(path);
                    for (role, destination) in [
                        (crate::project::InputRole::Engine, 0),
                        (crate::project::InputRole::Resources, 1),
                    ] {
                        if let Some(candidates) = discovery
                            .candidates
                            .get(&role)
                            .filter(|items| items.len() == 1)
                        {
                            if destination == 0 {
                                self.config.engine = candidates[0].clone();
                            } else {
                                self.config.resources = candidates[0].clone();
                            }
                        }
                    }
                    return;
                }
            }
        }
        if discovery.project_files.len() == 1 && files.len() == 1 {
            self.open_project(discovery.project_files[0].clone());
            return;
        }
        self.input_discovery_selection.clear();
        let unresolved_choices = !discovery.project_files.is_empty()
            || discovery
                .candidates
                .values()
                .any(|candidates| candidates.len() > 1);
        if discovery.project_files.is_empty() {
            if let Some(mapping) = discovery.unique_required_mapping() {
                for (role, path) in mapping {
                    self.input_discovery_selection.insert(role, path);
                }
                for role in [
                    crate::project::InputRole::Music,
                    crate::project::InputRole::Mv,
                    crate::project::InputRole::Cover,
                ] {
                    if let Some(values) = discovery
                        .candidates
                        .get(&role)
                        .filter(|values| values.len() == 1)
                    {
                        self.input_discovery_selection
                            .insert(role, values[0].clone());
                    }
                }
                self.apply_discovered_inputs();
                if unresolved_choices {
                    self.input_discovery = Some(discovery);
                    self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                } else {
                    self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                }
                return;
            }
        }
        self.input_discovery = Some(discovery);
        self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
    }

    fn apply_discovered_inputs(&mut self) {
        let selected = self.input_discovery_selection.clone();
        if let Some(path) = selected.get(&crate::project::InputRole::Engine) {
            self.config.engine = path.clone();
        }
        if let Some(path) = selected.get(&crate::project::InputRole::Level) {
            self.config.level = path.clone();
        }
        if let Some(path) = selected.get(&crate::project::InputRole::Resources) {
            self.config.resources = path.clone();
        }
        self.config.music = selected.get(&crate::project::InputRole::Music).cloned();
        self.config.mv = selected.get(&crate::project::InputRole::Mv).cloned();
        self.config.resource_overrides.particles = selected
            .get(&crate::project::InputRole::ParticleResources)
            .cloned();
        self.config.resource_overrides.effects = selected
            .get(&crate::project::InputRole::EffectsResources)
            .cloned();
        self.config.resource_overrides.background = selected
            .get(&crate::project::InputRole::BackgroundResources)
            .cloned();
        self.config.output = None;
        self.new_project();
        if let Some(path) = selected.get(&crate::project::InputRole::Cover) {
            if let Some(project) = &mut self.project {
                project.inputs.cover = Some(path.clone());
            }
        }
        self.input_discovery = None;
        self.summary =
            "Inputs identified. Review them, then save a project or render directly.".into();
        self.error = false;
    }

    fn project_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("New Render Project").clicked() {
                self.dispatch_command(crate::workspace::CommandId::NewRenderProject, ui.ctx());
            }
            if ui.button("Open Render Project...").clicked() {
                self.dispatch_command(crate::workspace::CommandId::OpenRenderProject, ui.ctx());
            }
            if ui.button("Save Render Project").clicked() {
                self.dispatch_command(crate::workspace::CommandId::SaveRenderProject, ui.ctx());
            }
            if ui.button("Save Render Project As...").clicked() {
                self.dispatch_command(crate::workspace::CommandId::SaveRenderProjectAs, ui.ctx());
            }
            let selected = self
                .project_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "Recent projects".into());
            let mut open_recent = None;
            egui::ComboBox::from_id_salt("recent-projects")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if self.project_recents.paths.is_empty() {
                        ui.label("No recent projects");
                    }
                    for path in &self.project_recents.paths {
                        let label = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Project");
                        if ui
                            .selectable_label(false, format!("{label} | {}", path.display()))
                            .clicked()
                        {
                            open_recent = Some(path.clone());
                        }
                    }
                });
            if let Some(path) = open_recent {
                self.open_project(path);
            }
        });
        if let Some(path) = &self.project_path {
            ui.small(format!("Project: {}", path.display()));
        }
        if let Some(error) = &self.project_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(discovery) = self.input_discovery.clone() {
            ui.separator();
            ui.strong("Dropped inputs need an explicit mapping");
            let mut open_discovered_project = None;
            for path in &discovery.project_files {
                if ui
                    .button(format!("Open discovered project: {}", path.display()))
                    .clicked()
                {
                    open_discovered_project = Some(path.clone());
                }
            }
            for role in [
                crate::project::InputRole::Engine,
                crate::project::InputRole::Level,
                crate::project::InputRole::Resources,
                crate::project::InputRole::ParticleResources,
                crate::project::InputRole::EffectsResources,
                crate::project::InputRole::BackgroundResources,
                crate::project::InputRole::Music,
                crate::project::InputRole::Cover,
                crate::project::InputRole::Mv,
            ] {
                if let Some(candidates) = discovery.candidates.get(&role) {
                    if candidates.is_empty() {
                        continue;
                    }
                    let label = format!("{role:?}");
                    let current = self.input_discovery_selection.get(&role).cloned();
                    egui::ComboBox::from_id_salt(format!("discovery-{role:?}"))
                        .selected_text(
                            current
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| format!("Select {label}")),
                        )
                        .show_ui(ui, |ui| {
                            for candidate in candidates {
                                if ui
                                    .selectable_label(
                                        current.as_ref() == Some(candidate),
                                        candidate.display().to_string(),
                                    )
                                    .clicked()
                                {
                                    self.input_discovery_selection
                                        .insert(role, candidate.clone());
                                }
                            }
                        });
                }
            }
            if discovery.truncated {
                ui.label("Directory scan stopped at its 256-file bound.");
            }
            for message in &discovery.ignored {
                ui.small(message);
            }
            let required_ready = [
                crate::project::InputRole::Engine,
                crate::project::InputRole::Level,
                crate::project::InputRole::Resources,
            ]
            .iter()
            .all(|role| {
                self.input_discovery_selection
                    .get(role)
                    .is_some_and(|selected| {
                        discovery
                            .candidates
                            .get(role)
                            .is_some_and(|candidates| candidates.contains(selected))
                    })
            });
            if ui
                .add_enabled(required_ready, egui::Button::new("Use selected files"))
                .clicked()
            {
                self.apply_discovered_inputs();
            }
            if discovery.project_files.is_empty() && discovery.candidates.is_empty() {
                ui.label("No supported input types were identified. Choose files directly below.");
            }
            if let Some(path) = open_discovered_project {
                self.open_project(path);
            }
        }
    }

    fn project_workflow_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Reusable render project");
        ui.label("Projects store input references and project-level render defaults. Profiles hold reusable renderer preferences; jobs capture a resolved snapshot.");
        ui.label("Drop engine, level, resource, music, image, video, or .sonoproject files here, or choose them below. Directory discovery is bounded to 256 files and two levels.");
        if self.project.is_some() {
            let mut validate_clicked = false;
            let mut create_job_clicked = false;
            ui.horizontal(|ui| {
                ui.label("Project name");
                let mut name = self
                    .project
                    .as_ref()
                    .and_then(|project| project.metadata.name.clone())
                    .unwrap_or_default();
                if ui.text_edit_singleline(&mut name).changed() {
                    if let Some(project) = &mut self.project {
                        project.metadata.name = (!name.trim().is_empty()).then_some(name);
                    }
                }
                validate_clicked = ui.button("Validate project").clicked();
                create_job_clicked = ui.button("Create render job").clicked();
            });
            if validate_clicked {
                self.sync_project_from_config();
                let result = (|| -> Result<String> {
                    let project = self.project.as_ref().unwrap();
                    let path = self
                        .project_path
                        .as_ref()
                        .context("save the project before resolving relative paths")?;
                    let validation = project.validate_inputs(path)?;
                    let (config, _) = project.resolve_config(
                        path,
                        &self.config,
                        self.profile_manager.as_ref(),
                        None,
                    )?;
                    if config.layers.bgm
                        && config.music.as_ref().is_none_or(|music| !music.is_file())
                    {
                        anyhow::bail!(
                            "BGM is enabled but the project has no accessible music file"
                        );
                    }
                    if !validation.is_valid() {
                        anyhow::bail!("{}", validation.errors.join("; "));
                    }
                    if let Some(output) = &config.output {
                        let parent = output
                            .parent()
                            .filter(|parent| !parent.as_os_str().is_empty())
                            .unwrap_or(std::path::Path::new("."));
                        if parent.exists() && !parent.is_dir() {
                            anyhow::bail!("output parent is not a directory: {}", parent.display());
                        }
                        if output.exists() {
                            anyhow::bail!(
                                "output already exists; render jobs will not overwrite it: {}",
                                output.display()
                            );
                        }
                    } else {
                        anyhow::bail!(
                            "set a project or profile output path before creating a render job"
                        );
                    }
                    Ok(format!("Project inputs valid: {} level entities, {} resources; compatibility analysis completed.", validation.level_entities.unwrap_or(0), validation.resource_count.unwrap_or(0)))
                })();
                match result {
                    Ok(message) => {
                        self.summary = message;
                        self.project_error = None;
                        self.error = false;
                    }
                    Err(error) => {
                        self.project_error = Some(format!("Project validation: {error:#}"));
                        self.error = true;
                    }
                }
            }
            if create_job_clicked {
                self.create_project_job();
            }
            if let Some(profile) = self
                .project
                .as_ref()
                .and_then(|project| project.profile.clone())
            {
                ui.small(format!(
                    "Required profile: {} ({})",
                    profile.name.as_deref().unwrap_or("unnamed"),
                    profile.id.as_deref().unwrap_or("legacy name reference")
                ));
            } else {
                ui.small(
                    "No required profile; the current renderer configuration is used as the base.",
                );
            }
            let profile_list = self
                .profile_manager
                .as_ref()
                .and_then(|manager| manager.list().ok())
                .unwrap_or_default();
            let current_reference = self
                .project
                .as_ref()
                .and_then(|project| project.profile.clone());
            let current_label = current_reference
                .as_ref()
                .map(|reference| {
                    if let Some(id) = &reference.id {
                        profile_list
                            .iter()
                            .find(|profile| &profile.id == id)
                            .map(|profile| profile.name.clone())
                            .unwrap_or_else(|| format!("Missing profile ({id})"))
                    } else {
                        reference
                            .name
                            .clone()
                            .unwrap_or_else(|| "Missing profile name".into())
                    }
                })
                .unwrap_or_else(|| "Current renderer settings".into());
            egui::ComboBox::from_id_salt("project-required-profile")
                .selected_text(format!("Project profile: {current_label}"))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            current_reference.is_none(),
                            "Current renderer settings (no required profile)",
                        )
                        .clicked()
                    {
                        if let Some(project) = &mut self.project {
                            project.profile = None;
                        }
                    }
                    for profile in &profile_list {
                        let selected = current_reference
                            .as_ref()
                            .and_then(|reference| reference.id.as_ref())
                            == Some(&profile.id);
                        if ui
                            .selectable_label(
                                selected,
                                format!("{} ({})", profile.name, profile.id),
                            )
                            .clicked()
                        {
                            if let Some(project) = &mut self.project {
                                project.profile = Some(crate::project::ProjectProfileRef {
                                    id: Some(profile.id.clone()),
                                    name: Some(profile.name.clone()),
                                    ..crate::project::ProjectProfileRef::default()
                                });
                            }
                        }
                    }
                });
            let current_start = self.config.start_time;
            let current_duration = self.config.duration;
            let current_whole_chart = self.config.whole_chart;
            let current_fps = self.config.fps;
            let current_width = self.config.width;
            let current_height = self.config.height;
            let current_layers = self.config.layers.clone();
            let parent_window = self.parent.clone();
            egui::CollapsingHeader::new("Project-specific render overrides")
                .default_open(false)
                .show(ui, |ui| {
                    let Some(project) = &mut self.project else {
                        return;
                    };
                    let mut project_output = project.overrides.output.clone().unwrap_or_default();
                    egui::Grid::new("project-output-override").show(ui, |ui| {
                        path_row(
                            ui,
                            "Project output default",
                            &mut project_output,
                            &["mp4"],
                            true,
                            parent_window.as_deref(),
                        );
                    });
                    project.overrides.output =
                        (!project_output.as_os_str().is_empty()).then_some(project_output);
                    let mut range_override = project.overrides.start_time.is_some()
                        || project.overrides.duration.is_some()
                        || project.overrides.whole_chart.is_some();
                    if ui
                        .checkbox(&mut range_override, "Override range for this project")
                        .changed()
                    {
                        if range_override {
                            project.overrides.start_time = Some(current_start);
                            project.overrides.duration = Some(current_duration);
                            project.overrides.whole_chart = Some(current_whole_chart);
                        } else {
                            project.overrides.start_time = None;
                            project.overrides.duration = None;
                            project.overrides.whole_chart = None;
                        }
                    }
                    if range_override {
                        ui.horizontal(|ui| {
                            ui.label("Start");
                            ui.add(
                                egui::DragValue::new(
                                    project.overrides.start_time.get_or_insert(current_start),
                                )
                                .speed(0.1)
                                .range(0.0..=86400.0),
                            );
                            ui.label("Duration");
                            ui.add(
                                egui::DragValue::new(
                                    project.overrides.duration.get_or_insert(current_duration),
                                )
                                .speed(0.1)
                                .range(0.01..=86400.0),
                            );
                            ui.checkbox(
                                project
                                    .overrides
                                    .whole_chart
                                    .get_or_insert(current_whole_chart),
                                "Whole chart",
                            );
                        });
                    }
                    let mut video_override = project.overrides.width.is_some()
                        || project.overrides.height.is_some()
                        || project.overrides.fps.is_some();
                    if ui
                        .checkbox(&mut video_override, "Override resolution and frame rate")
                        .changed()
                    {
                        if video_override {
                            project.overrides.width = Some(current_width);
                            project.overrides.height = Some(current_height);
                            project.overrides.fps = Some(current_fps);
                        } else {
                            project.overrides.width = None;
                            project.overrides.height = None;
                            project.overrides.fps = None;
                        }
                    }
                    if video_override {
                        ui.horizontal(|ui| {
                            ui.label("Resolution");
                            ui.add(
                                egui::DragValue::new(
                                    project.overrides.width.get_or_insert(current_width),
                                )
                                .range(1..=8192),
                            );
                            ui.label("Height");
                            ui.add(
                                egui::DragValue::new(
                                    project.overrides.height.get_or_insert(current_height),
                                )
                                .range(1..=8192),
                            );
                            ui.label("FPS");
                            ui.add(
                                egui::DragValue::new(
                                    project.overrides.fps.get_or_insert(current_fps),
                                )
                                .range(1..=240),
                            );
                        });
                    }
                    let mut layer_override = project.overrides.layers.is_some();
                    if ui
                        .checkbox(&mut layer_override, "Override enabled render layers")
                        .changed()
                    {
                        project.overrides.layers = layer_override.then(|| current_layers.clone());
                    }
                    if layer_override {
                        let layers = project
                            .overrides
                            .layers
                            .get_or_insert_with(|| current_layers.clone());
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut layers.particles, "Particles");
                            ui.checkbox(&mut layers.sfx, "SFX");
                            ui.checkbox(&mut layers.bgm, "BGM");
                        });
                    }
                });
        } else {
            ui.label(
                "No project file is open. The direct-input renderer workflow remains available.",
            );
        }
    }

    fn adopt_config_profile(
        &mut self,
        profile: crate::config_profiles::ConfigProfile,
        success_message: String,
    ) {
        let queue_error = self
            .job_manager
            .as_ref()
            .and_then(|queue| queue.configure(profile.config.job_manager.clone()).err());
        let failed = queue_error.is_some();
        self.profile_name_input = profile.name.clone();
        self.active_profile_id = Some(profile.id);
        self.config = profile.config;
        self.summary = queue_error.map_or(success_message, |error| {
            format!("Profile applied; queue configuration failed: {error:#}")
        });
        self.error = failed;
    }

    fn config_profile_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Full-render profile");
            if let Some(error) = &self.profile_manager_error {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Profile store: {error}"));
            }
            let Some(manager) = self.profile_manager.clone() else {
                return;
            };
            let profiles = match manager.list() {
                Ok(profiles) => profiles,
                Err(error) => {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!("Profiles unavailable: {error:#}"),
                    );
                    return;
                }
            };
            let mut selected = self.active_profile_id.clone();
            egui::ComboBox::from_id_salt("full-render-profile-selector")
                .selected_text(
                    selected
                        .as_deref()
                        .and_then(|id| profiles.iter().find(|profile| profile.id == id))
                        .map(|profile| profile.name.as_str())
                        .unwrap_or("Current configuration"),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, "Current configuration");
                    for profile in &profiles {
                        ui.selectable_value(&mut selected, Some(profile.id.clone()), &profile.name);
                    }
                });
            if selected != self.active_profile_id {
                let selection = selected
                    .as_deref()
                    .map(|id| manager.select(id))
                    .transpose()
                    .and_then(|_| {
                        if selected.is_none() {
                            manager.select_current()?;
                            Ok(None)
                        } else {
                            manager.active()
                        }
                    });
                match selection {
                    Ok(profile) => {
                        if let Some(profile) = profile {
                            let name = profile.name.clone();
                            self.adopt_config_profile(profile, format!("Selected profile {name}."));
                            self.confirm_profile_delete = false;
                            self.confirm_profile_reset = false;
                        } else {
                            self.active_profile_id = None;
                            self.profile_name_input.clear();
                            self.confirm_profile_delete = false;
                            self.confirm_profile_reset = false;
                        }
                    }
                    Err(error) => {
                        self.summary = format!("Could not select profile: {error:#}");
                        self.error = true;
                    }
                }
            }
            if let Some(id) = self.active_profile_id.as_deref() {
                if let Ok(profile) = manager.get(id) {
                    let dirty = profile.config != self.config;
                    ui.label(if dirty {
                        "Unsaved profile edits"
                    } else {
                        "Saved"
                    });
                    if dirty && ui.button("Save changes").clicked() {
                        match manager.update(id, self.config.clone()) {
                            Ok(saved) => {
                                self.profile_name_input = saved.name;
                                self.summary = "Saved changes to the selected profile.".into();
                                self.error = false;
                            }
                            Err(error) => {
                                self.summary = format!("Could not save profile: {error:#}");
                                self.error = true;
                            }
                        }
                    }
                    if dirty && ui.button("Discard edits").clicked() {
                        self.config = profile.config;
                        self.summary = "Restored the saved profile configuration.".into();
                        self.error = false;
                    }
                }
            }
        });
        let Some(manager) = self.profile_manager.clone() else {
            return;
        };
        ui.collapsing("Manage full-render profiles", |ui| {
            ui.label("Editing renderer tabs changes the current configuration. Saved profiles change only when you explicitly save them.");
            ui.horizontal(|ui| {
                ui.label("Profile name");
                ui.add_sized(
                    [250.0, 24.0],
                    egui::TextEdit::singleline(&mut self.profile_name_input),
                );
                if ui.button("Save as new").clicked() {
                    match manager.create(&self.profile_name_input, self.config.clone()) {
                        Ok(profile) => {
                            let _ = manager.select(&profile.id);
                            let name = profile.name.clone();
                            self.adopt_config_profile(profile, format!("Created profile {name}."));
                        }
                        Err(error) => {
                            self.summary = format!("Could not create profile: {error:#}");
                            self.error = true;
                        }
                    }
                }
                if let Some(id) = self.active_profile_id.clone() {
                    if ui.button("Duplicate").clicked() {
                        match manager.duplicate(&id, &self.profile_name_input) {
                            Ok(profile) => {
                                let _ = manager.select(&profile.id);
                            let name = profile.name.clone();
                            self.adopt_config_profile(
                                profile,
                                format!("Duplicated profile as {name}."),
                            );
                            }
                            Err(error) => {
                                self.summary = format!("Could not duplicate profile: {error:#}");
                                self.error = true;
                            }
                        }
                    }
                    if ui.button("Rename").clicked() {
                        match manager.rename(&id, &self.profile_name_input) {
                            Ok(profile) => {
                                self.profile_name_input = profile.name.clone();
                                self.summary = format!("Renamed profile to {}.", profile.name);
                                self.error = false;
                            }
                            Err(error) => {
                                self.summary = format!("Could not rename profile: {error:#}");
                                self.error = true;
                            }
                        }
                    }
                    if ui.button("Reset to renderer defaults").clicked() {
                        self.confirm_profile_reset = true;
                    }
                    if ui.button("Delete...").clicked() {
                        self.confirm_profile_delete = true;
                    }
                }
            });
            if self.confirm_profile_reset {
                ui.horizontal(|ui| {
                    ui.label("Replace this saved profile with renderer defaults?");
                    if ui.button("Confirm reset").clicked() {
                        if let Some(id) = self.active_profile_id.as_deref() {
                            match manager.reset(id) {
                                Ok(profile) => {
                                    self.adopt_config_profile(
                                        profile,
                                        "Profile reset to renderer defaults.".into(),
                                    );
                                    self.confirm_profile_reset = false;
                                }
                                Err(error) => {
                                    self.summary = format!("Could not reset profile: {error:#}");
                                    self.error = true;
                                }
                            }
                        }
                    }
                    if ui.button("Keep profile").clicked() {
                        self.confirm_profile_reset = false;
                    }
                });
            }
            if self.confirm_profile_delete {
                ui.horizontal(|ui| {
                    ui.label("Delete the selected saved profile?");
                    if ui.button("Confirm delete").clicked() {
                        if let Some(id) = self.active_profile_id.clone() {
                            match manager.delete(&id) {
                                Ok(profile) => {
                                    self.active_profile_id = None;
                                    self.profile_name_input.clear();
                                    self.confirm_profile_delete = false;
                                    self.summary = format!("Deleted profile {}.", profile.name);
                                    self.error = false;
                                }
                                Err(error) => {
                                    self.summary = format!("Could not delete profile: {error:#}");
                                    self.error = true;
                                }
                            }
                        }
                    }
                    if ui.button("Keep profile").clicked() {
                        self.confirm_profile_delete = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.button("Import profile...").clicked() {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Import Sono-Renderer profile")
                        .add_filter("Sono-Renderer profile", &["json"]);
                    if let Some(parent) = self.parent.as_deref() {
                        dialog = dialog.set_parent(parent);
                    }
                    if let Some(path) = dialog.pick_file() {
                        match manager.import(&path, None).and_then(|profile| {
                            manager.select(&profile.id)
                        }) {
                            Ok(profile) => {
                                let name = profile.name.clone();
                                self.adopt_config_profile(profile, format!("Imported profile {name}."));
                            }
                            Err(error) => {
                                self.summary = format!("Could not import profile: {error:#}");
                                self.error = true;
                            }
                        }
                    }
                }
                if let Some(id) = self.active_profile_id.as_deref() {
                    if ui.button("Export profile...").clicked() {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title("Export Sono-Renderer profile")
                            .add_filter("Sono-Renderer profile", &["json"])
                            .set_file_name(format!("{}.sono-profile.json", self.profile_name_input));
                        if let Some(parent) = self.parent.as_deref() {
                            dialog = dialog.set_parent(parent);
                        }
                        if let Some(path) = dialog.save_file() {
                            match manager.export(id, &path) {
                                Ok(()) => {
                                    self.summary = format!("Exported profile to {}.", path.display());
                                    self.error = false;
                                }
                                Err(error) => {
                                    self.summary = format!("Could not export profile: {error:#}");
                                    self.error = true;
                                }
                            }
                        }
                    }
                }
            });
        });
    }

    fn import_gameplay_ui_pack(&mut self, path: &std::path::Path, root: Option<PathBuf>) {
        let previous = self.config.ui.clone();
        let previous_pack = previous
            .resolved_asset_pack_path()
            .and_then(|path| path.canonicalize().ok());
        match crate::render_ui::RendererUiConfig::import_asset_pack(path, root) {
            Ok((mut config, report)) => {
                if previous_pack.as_deref() == Some(report.manifest_path.as_path()) {
                    config.elements = previous.elements.clone();
                    for (name, font) in &previous.fonts {
                        config.fonts.insert(name.clone(), font.clone());
                    }
                    config.enabled = previous.enabled;
                    config.design_size = previous.design_size;
                    config.fit = previous.fit;
                    if let Err(error) = config.load_external_data() {
                        self.summary = format!("Asset root was changed, but the current UI profile could not be restored: {error:#}");
                        self.error = true;
                        return;
                    }
                }
                let layout_source = if config.profile_name.contains("Sono-Overlay reference HUD") {
                    "Sono-Overlay reference layout loaded"
                } else {
                    "editable fallback layout generated"
                };
                let message = format!(
                    "Imported {}: {} of {} supported image assets loaded; {} missing, {} invalid, {} unsupported media types; {} glyph families; {} HUD elements. {}.",
                    report.pack_name,
                    report.loaded_assets.len(),
                    report.total_assets.saturating_sub(report.unsupported_assets.len()),
                    report.missing_assets.len(),
                    report.invalid_assets.len(),
                    report.unsupported_assets.len(),
                    report.glyph_families.len(),
                    config.elements.len(),
                    layout_source,
                );
                self.config.ui = config;
                self.gameplay_ui_asset_thumbnail_cache.clear();
                self.gameplay_ui_asset_pack =
                    crate::presentation_assets::PresentationAssetPack::load(path).ok();
                self.gameplay_ui_import_report = Some(report);
                self.gameplay_ui_selected_element = self
                    .config
                    .ui
                    .elements
                    .first()
                    .map(|element| element.id.clone());
                self.summary = message;
                self.error = false;
            }
            Err(error) => {
                self.summary = format!("Could not import presentation asset pack: {error:#}");
                self.error = true;
            }
        }
    }

    fn gameplay_ui_scoring_editor(&mut self, ui: &mut egui::Ui) {
        let event_names = self
            .config
            .ui
            .loaded_events
            .iter()
            .map(|event| event.event.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let has_event_file = self.config.ui.events_file.is_some();
        let mut remove_rule = None;
        ui.collapsing("Gameplay scoring", |ui| {
            ui.label("Chart simulation is a deterministic Sono-Overlay all-perfect forecast, not recorded player performance. Verified non-scoring entities are ignored; unknown beat-bearing archetypes are skipped with a best-effort warning while recognized notes continue. Use an authoritative event track for actual judgments and life changes.");
            if self.config.ui.scoring.is_none() {
                match self.config.ui.resolved_scoring_source() {
                    crate::render_ui::UiScoringSource::EventTrack => {
                        ui.label("Gameplay values: A manually supplied event track takes precedence; configure event mappings to use its events.");
                    }
                    crate::render_ui::UiScoringSource::Disabled => {
                        ui.label("Gameplay values are disabled.");
                    }
                    crate::render_ui::UiScoringSource::Automatic
                    | crate::render_ui::UiScoringSource::SonoOverlay => {
                        ui.label("Gameplay values: Auto-generated All Perfect when a chart is previewed or exported.");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Configure All Perfect scoring").clicked() {
                        let scoring = crate::render_ui::UiScoringConfig::default();
                        self.config.ui.scoring = Some(scoring);
                    }
                    if ui.button("Configure event mappings").clicked() {
                        let mut scoring = crate::render_ui::UiScoringConfig::default();
                        scoring.source = crate::render_ui::UiScoringSource::EventTrack;
                        self.config.ui.scoring = Some(scoring);
                    }
                });
                if self.config.ui.scoring.is_none() {
                    return;
                }
            }

            let scoring = self.config.ui.scoring.as_mut().expect("scoring was checked above");
            egui::ComboBox::from_label("Scoring source")
                .selected_text(match scoring.source {
                    crate::render_ui::UiScoringSource::Automatic => "Automatic",
                    crate::render_ui::UiScoringSource::SonoOverlay => "Sono-Overlay simulation",
                    crate::render_ui::UiScoringSource::EventTrack => "Event track / replay",
                    crate::render_ui::UiScoringSource::Disabled => "Disabled",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut scoring.source,
                        crate::render_ui::UiScoringSource::Automatic,
                        "Automatic (event track if supplied, otherwise simulation)",
                    );
                    ui.selectable_value(
                        &mut scoring.source,
                        crate::render_ui::UiScoringSource::SonoOverlay,
                        "Sono-Overlay chart simulation",
                    );
                    ui.selectable_value(
                        &mut scoring.source,
                        crate::render_ui::UiScoringSource::EventTrack,
                        "Authoritative event track / replay",
                    );
                    ui.selectable_value(
                        &mut scoring.source,
                        crate::render_ui::UiScoringSource::Disabled,
                        "Disabled",
                    );
                });
            if scoring.source == crate::render_ui::UiScoringSource::SonoOverlay
                || scoring.source == crate::render_ui::UiScoringSource::Automatic
            {
                ui.separator();
                ui.label("Sono-Overlay forecast settings");
                ui.horizontal(|ui| {
                    ui.label("Team power");
                    ui.add(egui::DragValue::new(&mut scoring.forecast.team_power).speed(100.0));
                    let mut has_rating = scoring.forecast.level_rating.is_some();
                    if ui.checkbox(&mut has_rating, "Set level rating").changed() {
                        scoring.forecast.level_rating = has_rating.then_some(5.0);
                    }
                    if let Some(rating) = &mut scoring.forecast.level_rating {
                        ui.add(egui::DragValue::new(rating).speed(0.1).range(0.0..=99.0));
                    }
                });
                ui.small("Rating priority: explicit override, then valid level metadata, then default 0.0.");
                if let Some(frame) = &self.preview_frame {
                    if let Some(rating) = frame
                        .ui_state
                        .gameplay
                        .as_ref()
                        .and_then(|gameplay| gameplay.variables.get("effective_level_rating"))
                    {
                        ui.label(format!("Effective rating in the last Program Preview: {rating:.1}."));
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Score rules");
                    egui::ComboBox::from_id_salt("sono-overlay-score-mode")
                        .selected_text(match scoring.forecast.score_mode {
                            crate::sono_overlay_scoring::ScoreMode::Standard => "Standard",
                            crate::sono_overlay_scoring::ScoreMode::Tournament => "Tournament",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut scoring.forecast.score_mode,
                                crate::sono_overlay_scoring::ScoreMode::Standard,
                                "Standard",
                            );
                            ui.selectable_value(
                                &mut scoring.forecast.score_mode,
                                crate::sono_overlay_scoring::ScoreMode::Tournament,
                                "Tournament",
                            );
                        });
                    ui.checkbox(&mut scoring.forecast.all_flick, "All-flick scoring variant");
                    ui.checkbox(&mut scoring.forecast.chart_skills_enabled, "Chart Skill markers (2x for 5s)");
                    egui::ComboBox::from_id_salt("sono-overlay-judgment-policy")
                        .selected_text(scoring.forecast.judgment.as_deref().unwrap_or("No judgment art"))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut scoring.forecast.judgment, Some("PERFECT".into()), "Reference PERFECT forecast");
                            ui.selectable_value(&mut scoring.forecast.judgment, None, "No judgment art");
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Initial life");
                    ui.add(egui::DragValue::new(&mut scoring.forecast.initial_life).speed(1.0));
                    ui.label("Life bounds");
                    ui.add(egui::DragValue::new(&mut scoring.forecast.minimum_life).speed(1.0));
                    ui.add(egui::DragValue::new(&mut scoring.forecast.maximum_life).speed(1.0));
                });
                ui.small("Sono-Overlay supplies a chart/user life track, not per-note life-change formulas. This forecast holds the configured life value; event tracks can carry verified dynamic life results.");
            }

            let event_source = scoring.source == crate::render_ui::UiScoringSource::EventTrack
                || (scoring.source == crate::render_ui::UiScoringSource::Automatic
                    && has_event_file);
            if event_source {
                ui.separator();
                ui.label("Event-to-HUD rules");
                ui.small("Map exact event names from the selected event file. Event timestamps are Watch seconds; Watch rendering itself does not create player-result events.");
            ui.horizontal(|ui| {
                ui.label("Initial score");
                ui.add(egui::DragValue::new(&mut scoring.initial_score).speed(10.0));
                ui.label("Combo");
                ui.add(egui::DragValue::new(&mut scoring.initial_combo));
            });
            ui.horizontal(|ui| {
                ui.label("Initial life");
                ui.add(egui::DragValue::new(&mut scoring.initial_life).speed(1.0));
                ui.label("Life min");
                ui.add(egui::DragValue::new(&mut scoring.minimum_life).speed(1.0));
                ui.label("max");
                ui.add(egui::DragValue::new(&mut scoring.maximum_life).speed(1.0));
            });
            if ui.button("Use Sono-Overlay starting life (1000)").clicked() {
                scoring.initial_score = 0.0;
                scoring.initial_combo = 0;
                scoring.initial_life = 1_000.0;
                scoring.minimum_life = 0.0;
                scoring.maximum_life = 1_000.0;
            }

            for (index, rule) in scoring.rules.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("Rule {}", index + 1));
                        egui::ComboBox::from_id_salt(("gameplay-event-name", index))
                            .selected_text(if rule.event.is_empty() {
                                "Choose event".to_owned()
                            } else {
                                rule.event.clone()
                            })
                            .show_ui(ui, |ui| {
                                for event in &event_names {
                                    ui.selectable_value(&mut rule.event, event.clone(), event);
                                }
                            });
                        ui.text_edit_singleline(&mut rule.event);
                        if ui.small_button("Remove").clicked() {
                            remove_rule = Some(index);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Score Δ");
                        ui.add(egui::DragValue::new(&mut rule.score).speed(1.0));
                        ui.label("Life Δ");
                        ui.add(egui::DragValue::new(&mut rule.life).speed(1.0));
                        ui.label("Judgment");
                        ui.text_edit_singleline(&mut rule.judgement);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Combo");
                        egui::ComboBox::from_id_salt(("gameplay-combo-change", index))
                            .selected_text(match &rule.combo {
                                crate::render_ui::UiComboChange::Preserve => "Preserve",
                                crate::render_ui::UiComboChange::Increment => "Increment",
                                crate::render_ui::UiComboChange::Reset => "Reset",
                                crate::render_ui::UiComboChange::Set(_) => "Set value",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut rule.combo,
                                    crate::render_ui::UiComboChange::Preserve,
                                    "Preserve",
                                );
                                ui.selectable_value(
                                    &mut rule.combo,
                                    crate::render_ui::UiComboChange::Increment,
                                    "Increment",
                                );
                                ui.selectable_value(
                                    &mut rule.combo,
                                    crate::render_ui::UiComboChange::Reset,
                                    "Reset",
                                );
                                ui.selectable_value(
                                    &mut rule.combo,
                                    crate::render_ui::UiComboChange::Set(0),
                                    "Set value",
                                );
                            });
                        if let crate::render_ui::UiComboChange::Set(value) = &mut rule.combo {
                            ui.add(egui::DragValue::new(value));
                        }
                        let mut match_note_type = rule.note_type.is_some();
                        if ui
                            .checkbox(&mut match_note_type, "Match note type")
                            .changed()
                        {
                            rule.note_type = match_note_type.then(String::new);
                        }
                        if let Some(note_type) = &mut rule.note_type {
                            ui.text_edit_singleline(note_type);
                        }
                    });
                });
            }
            if let Some(index) = remove_rule {
                scoring.rules.remove(index);
            }
            if ui.button("Add event rule").clicked() {
                let event = event_names.iter().next().cloned().unwrap_or_default();
                scoring.rules.push(crate::render_ui::UiScoreRule {
                    event,
                    ..Default::default()
                });
            }
            }
        });
    }

    fn gameplay_ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Gameplay UI profile");
        ui.label("Profiles arrange artwork over gameplay. Asset packs supply artwork and metadata; they do not define gameplay calculations.");
        if ui
            .button("Take the optional gameplay presentation tour")
            .clicked()
        {
            self.presentation_tour_step = 0;
            self.tab = 0;
            self.presentation_tour_open = true;
        }

        ui.horizontal(|ui| {
            if ui.button("Import Asset Pack...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Import presentation asset pack")
                    .add_filter("Presentation asset pack", &["json"]);
                if let Some(parent) = self.parent.as_deref() { dialog = dialog.set_parent(parent); }
                if let Some(path) = dialog.pick_file() {
                    self.import_gameplay_ui_pack(&path, None);
                    let has_missing = self.gameplay_ui_import_report.as_ref().is_some_and(|report| !report.missing_assets.is_empty());
                    if has_missing {
                        let repair = rfd::MessageDialog::new()
                            .set_title("Some pack assets are missing")
                            .set_description("Choose the folder that contains the relative paths listed by the manifest? You can also keep the partial import and repair it later.")
                            .set_level(rfd::MessageLevel::Warning)
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show() == rfd::MessageDialogResult::Yes;
                        if repair {
                            let mut folder = rfd::FileDialog::new().set_title("Select presentation asset root");
                            if let Some(parent) = self.parent.as_deref() { folder = folder.set_parent(parent); }
                            if let Some(root) = folder.pick_folder() { self.import_gameplay_ui_pack(&path, Some(root)); }
                        }
                    }
                }
            }
            if ui.button("Load UI Profile...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Load gameplay UI profile")
                    .add_filter("JSON profile or asset pack", &["json"]);
                if let Some(parent) = self.parent.as_deref() { dialog = dialog.set_parent(parent); }
                if let Some(path) = dialog.pick_file() {
                    let is_pack = std::fs::read(&path).ok()
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                        .and_then(|json| json.get("format").and_then(serde_json::Value::as_str).map(str::to_owned))
                        .is_some_and(|format| format == crate::presentation_assets::ASSET_PACK_FORMAT);
                    if is_pack {
                        let import = rfd::MessageDialog::new()
                            .set_title("Presentation asset pack selected")
                            .set_description("This file is an asset pack, not a gameplay UI profile. Import it and create a profile from its assets?")
                            .set_level(rfd::MessageLevel::Info)
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .show() == rfd::MessageDialogResult::Yes;
                        if import { self.import_gameplay_ui_pack(&path, None); }
                    } else {
                        match self.config.set_ui_profile(&path).and_then(|()| self.config.ui.load_external_data()) {
                            Ok(()) => {
                                self.gameplay_ui_asset_thumbnail_cache.clear();
                                self.gameplay_ui_asset_pack = self.config.ui.resolved_asset_pack_path()
                                    .and_then(|pack| crate::presentation_assets::PresentationAssetPack::load(pack).ok());
                                self.gameplay_ui_import_report = None;
                                self.gameplay_ui_selected_element = self.config.ui.elements.first().map(|element| element.id.clone());
                                self.summary = format!("Loaded gameplay UI profile: {}", path.display());
                                self.error = false;
                            }
                            Err(error) => {
                                let repairable = crate::render_ui::RendererUiConfig::load_profile_for_repair(&path)
                                    .ok()
                                    .filter(|ui| ui.asset_pack.is_some());
                                if let Some(ui) = repairable {
                                    self.config.ui = ui;
                                    self.gameplay_ui_asset_thumbnail_cache.clear();
                                    self.gameplay_ui_asset_pack = self.config.ui.resolved_asset_pack_path()
                                        .and_then(|pack| crate::presentation_assets::PresentationAssetPack::load(pack).ok());
                                    self.gameplay_ui_import_report = None;
                                    self.gameplay_ui_selected_element = self.config.ui.elements.first().map(|element| element.id.clone());
                                    self.summary = format!("Profile layout loaded with missing external assets. {error:#} Choose Select / repair asset root to relink the pack without rebuilding the HUD.");
                                    self.error = true;
                                } else {
                                    self.summary = format!("Gameplay UI profile failed: {error:#}");
                                    self.error = true;
                                }
                            }
                        }
                    }
                }
            }
            if ui.button("Save UI Profile As...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Save gameplay UI profile")
                    .add_filter("Gameplay UI profile", &["json"])
                    .set_file_name("gameplay-ui.json");
                if let Some(parent) = self.parent.as_deref() { dialog = dialog.set_parent(parent); }
                if let Some(path) = dialog.save_file() {
                    match self.config.ui.save_profile(&path) {
                        Ok(()) => { self.summary = format!("Saved gameplay UI profile to {}.", path.display()); self.error = false; }
                        Err(error) => { self.summary = format!("Could not save gameplay UI profile: {error:#}"); self.error = true; }
                    }
                }
            }
            ui.label(format!("Current profile: {}", self.config.ui.profile_name));
        });

        ui.checkbox(&mut self.config.ui.enabled, "Enable gameplay UI");
        egui::ComboBox::from_label("Rendering mode")
            .selected_text(match self.config.ui.mode {
                crate::render_ui::GameplayUiMode::Custom => "Custom profile",
                crate::render_ui::GameplayUiMode::Legacy => "Built-in legacy HUD",
                crate::render_ui::GameplayUiMode::Off => "Off",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.ui.mode,
                    crate::render_ui::GameplayUiMode::Custom,
                    "Custom profile",
                );
                ui.selectable_value(
                    &mut self.config.ui.mode,
                    crate::render_ui::GameplayUiMode::Legacy,
                    "Built-in legacy HUD",
                );
                ui.selectable_value(
                    &mut self.config.ui.mode,
                    crate::render_ui::GameplayUiMode::Off,
                    "Off",
                );
            });
        if self.config.ui.mode == crate::render_ui::GameplayUiMode::Legacy {
            ui.collapsing("Legacy HUD components", |ui| {
                ui.checkbox(&mut self.config.ui.primary_metric.enabled, "Primary metric");
                ui.checkbox(
                    &mut self.config.ui.secondary_metric.enabled,
                    "Secondary metric",
                );
                ui.checkbox(&mut self.config.ui.combo.enabled, "Combo");
                ui.checkbox(&mut self.config.ui.judgment.enabled, "Judgment");
                ui.checkbox(&mut self.config.ui.progress, "Progress bar");
            });
        }

        ui.separator();
        ui.heading("Assets and glyph families");
        if self.gameplay_ui_asset_pack.is_none() {
            self.gameplay_ui_asset_pack =
                self.config.ui.resolved_asset_pack_path().and_then(|path| {
                    crate::presentation_assets::PresentationAssetPack::load(path).ok()
                });
        }
        if let Some(report) = &self.gameplay_ui_import_report {
            ui.label(format!(
                "{} assets discovered · {} loaded · {} missing · {} invalid · {} unsupported",
                report.total_assets,
                report.loaded_assets.len(),
                report.missing_assets.len(),
                report.invalid_assets.len(),
                report.unsupported_assets.len()
            ));
            for missing in report.missing_assets.iter().take(8) {
                ui.colored_label(egui::Color32::YELLOW, format!("Missing: {missing}"));
            }
            for invalid in report.invalid_assets.iter().take(8) {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Invalid: {invalid}"));
            }
            if !report.unconfigured_element_families.is_empty() {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!(
                        "Unassigned glyph families: {}. Map digits explicitly before using these as counters.",
                        report.unconfigured_element_families.join(", ")
                    ),
                );
            }
            if !report.unresolved_glyph_assets.is_empty() {
                ui.label(format!(
                    "{} glyph images need an explicit character assignment.",
                    report.unresolved_glyph_assets.len()
                ));
            }
            if report.missing_assets.len() > 8 || report.invalid_assets.len() > 8 {
                ui.label("Additional issues are retained in the import summary above.");
            }
        } else {
            ui.label(format!(
                "{} registered assets · {} bitmap glyph fonts",
                self.config.ui.assets.len(),
                self.config.ui.fonts.len()
            ));
        }

        let glyph_groups: Vec<(String, Vec<(String, Option<String>, bool)>)>;
        let mut assets_changed = false;
        let mut represented_glyph_families = std::collections::BTreeSet::new();
        let mut image_ids: Vec<String> = self.config.ui.assets.keys().cloned().collect();
        image_ids.sort();
        if let Some(pack) = &self.gameplay_ui_asset_pack {
            let mut groups: std::collections::BTreeMap<
                String,
                Vec<(String, Option<String>, bool)>,
            > = std::collections::BTreeMap::new();
            for (font_name, font) in &self.config.ui.fonts {
                let mapped_family = font.glyph_assets.values().find_map(|alias| {
                    let reference = self.config.ui.assets.get(alias)?;
                    let asset_id = reference
                        .to_string_lossy()
                        .strip_prefix("pack://")?
                        .to_owned();
                    pack.assets
                        .iter()
                        .find(|entry| entry.id == asset_id)
                        .and_then(|entry| entry.family.clone())
                });
                let pack_family = mapped_family.or_else(|| {
                    let expected = font_name.to_ascii_lowercase();
                    pack.assets
                        .iter()
                        .filter_map(|entry| entry.family.as_ref())
                        .find(|family| {
                            let family = family.to_ascii_lowercase();
                            family == expected || family.starts_with(&format!("{expected}."))
                        })
                        .cloned()
                });
                let Some(pack_family) = pack_family else {
                    continue;
                };
                represented_glyph_families.insert(pack_family.clone());
                let choices = pack
                    .assets
                    .iter()
                    .filter(|entry| {
                        entry.kind == crate::presentation_assets::PresentationAssetKind::Glyph
                            && entry.family.as_ref() == Some(&pack_family)
                    })
                    .map(|entry| {
                        (
                            entry.id.clone(),
                            entry.character.clone(),
                            self.config.ui.loaded_asset_path(&entry.id).is_some(),
                        )
                    })
                    .collect::<Vec<_>>();
                if !choices.is_empty() {
                    groups.insert(font_name.clone(), choices);
                }
            }
            glyph_groups = groups.into_iter().collect();
            ui.label(format!(
                "Pack: {} · {} manifest entries",
                pack.name,
                pack.assets.len()
            ));
            for (family, choices) in &glyph_groups {
                let mapped = self
                    .config
                    .ui
                    .fonts
                    .get(family)
                    .map_or(0, |font| font.glyph_assets.len());
                ui.collapsing(
                    format!(
                        "{family} · {mapped} mapped glyphs · {} pack assets",
                        choices.len()
                    ),
                    |ui| {
                        if choices.iter().any(|(_, _, _)| true)
                            && choices.iter().all(|(_, _, loaded)| !loaded)
                        {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "No glyph image is currently available from this family.",
                            );
                        }
                        let mut character_keys = ('0'..='9')
                            .map(|character| character.to_string())
                            .collect::<std::collections::BTreeSet<_>>();
                        if let Some(font) = self.config.ui.fonts.get(family) {
                            character_keys.extend(font.glyph_assets.keys().cloned());
                        }
                        for character_key in character_keys {
                            let mut selected = self
                                .config
                                .ui
                                .fonts
                                .get(family)
                                .and_then(|font| font.glyph_assets.get(&character_key))
                                .cloned();
                            let selected_text = selected
                                .as_ref()
                                .map_or_else(|| "Unassigned".to_owned(), |id| id.clone());
                            let character_label = if character_key.chars().count() == 1 {
                                format!("Glyph '{}': {selected_text}", character_key)
                            } else {
                                format!("Glyph {character_key}: {selected_text}")
                            };
                            egui::ComboBox::from_id_salt((
                                "gameplay-ui-glyph",
                                family,
                                character_key.clone(),
                            ))
                            .width(260.0)
                            .selected_text(character_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut selected, None, "Unassigned");
                                for (id, _, available) in choices {
                                    if *available {
                                        ui.selectable_value(&mut selected, Some(id.clone()), id);
                                    }
                                }
                            });
                            if let Some(asset) = selected.as_deref() {
                                let path = self
                                    .config
                                    .ui
                                    .loaded_asset_path(asset)
                                    .map(Path::to_path_buf);
                                if let Some(texture) = path.and_then(|path| {
                                    gameplay_ui_asset_texture(
                                        ui.ctx(),
                                        &mut self.gameplay_ui_asset_thumbnail_cache,
                                        asset,
                                        &path,
                                    )
                                }) {
                                    ui.add(egui::Image::new((texture, egui::vec2(28.0, 28.0))));
                                }
                            }
                            let current = self
                                .config
                                .ui
                                .fonts
                                .get(family)
                                .and_then(|font| font.glyph_assets.get(&character_key));
                            if current != selected.as_ref() {
                                if let Some(asset) = selected {
                                    self.config.ui.assets.entry(asset.clone()).or_insert_with(
                                        || PathBuf::from(format!("pack://{asset}")),
                                    );
                                    let font =
                                        self.config.ui.fonts.entry(family.clone()).or_insert_with(
                                            || crate::render_ui::UiBitmapFont {
                                                asset: String::new(),
                                                glyph_width: 1,
                                                glyph_height: 1,
                                                columns: 1,
                                                first_codepoint: 32,
                                                advance: 1.0,
                                                glyph_map: Default::default(),
                                                glyph_assets: Default::default(),
                                                glyph_scales: Default::default(),
                                                glyph_advances: Default::default(),
                                                center_spacing: None,
                                                glyph_max_dimension: None,
                                            },
                                        );
                                    font.glyph_assets.insert(character_key, asset);
                                    assets_changed = true;
                                } else if let Some(font) = self.config.ui.fonts.get_mut(family) {
                                    font.glyph_assets.remove(&character_key);
                                    if font.glyph_assets.is_empty() {
                                        self.config.ui.fonts.remove(family);
                                    }
                                    assets_changed = true;
                                }
                            }
                        }

                        ui.separator();
                        ui.label("Map one explicitly selected character");
                        ui.horizontal(|ui| {
                            ui.label("Character (one Unicode character)");
                            ui.text_edit_singleline(&mut self.gameplay_ui_new_glyph_character);
                        });
                        let selected_text = self
                            .gameplay_ui_new_glyph_asset
                            .as_deref()
                            .unwrap_or("Choose an image from this family");
                        egui::ComboBox::from_id_salt(("gameplay-ui-new-glyph", family))
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for (id, _, available) in choices {
                                    if *available {
                                        ui.selectable_value(
                                            &mut self.gameplay_ui_new_glyph_asset,
                                            Some(id.clone()),
                                            id,
                                        );
                                    }
                                }
                            });
                        let character = self.gameplay_ui_new_glyph_character.trim();
                        let valid_character = character.chars().count() == 1;
                        if !valid_character && !character.is_empty() {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "Enter exactly one Unicode character.",
                            );
                        }
                        if ui
                            .add_enabled(
                                valid_character && self.gameplay_ui_new_glyph_asset.is_some(),
                                egui::Button::new("Assign character to selected image"),
                            )
                            .clicked()
                        {
                            if let Some(asset) = self.gameplay_ui_new_glyph_asset.take() {
                                self.config
                                    .ui
                                    .assets
                                    .entry(asset.clone())
                                    .or_insert_with(|| PathBuf::from(format!("pack://{asset}")));
                                let font =
                                    self.config.ui.fonts.entry(family.clone()).or_insert_with(
                                        || crate::render_ui::UiBitmapFont {
                                            asset: String::new(),
                                            glyph_width: 1,
                                            glyph_height: 1,
                                            columns: 1,
                                            first_codepoint: 32,
                                            advance: 1.0,
                                            glyph_map: Default::default(),
                                            glyph_assets: Default::default(),
                                            glyph_scales: Default::default(),
                                            glyph_advances: Default::default(),
                                            center_spacing: None,
                                            glyph_max_dimension: None,
                                        },
                                    );
                                font.glyph_assets.insert(character.to_owned(), asset);
                                assets_changed = true;
                            }
                        }
                    },
                );
            }
            let mut pack_families = pack
                .assets
                .iter()
                .filter(|entry| {
                    entry.kind == crate::presentation_assets::PresentationAssetKind::Glyph
                })
                .filter_map(|entry| entry.family.clone())
                .collect::<std::collections::BTreeSet<_>>();
            pack_families.retain(|family| !represented_glyph_families.contains(family));
            if !pack_families.is_empty() {
                ui.collapsing("Unmapped glyph families", |ui| {
                    ui.label("These families are not currently assigned to a counter. Add one to edit its character mappings.");
                    for family in &pack_families {
                        ui.horizontal(|ui| {
                            ui.label(family);
                            if ui.button("Add editable glyph family").clicked() {
                                self.config.ui.fonts.insert(
                                    family.clone(),
                                    crate::render_ui::UiBitmapFont {
                                        asset: String::new(),
                                        glyph_width: 1,
                                        glyph_height: 1,
                                        columns: 1,
                                        first_codepoint: 32,
                                        advance: 1.0,
                                        glyph_map: Default::default(),
                                        glyph_assets: Default::default(),
                                        glyph_scales: Default::default(),
                                        glyph_advances: Default::default(),
                                        center_spacing: None,
                                        glyph_max_dimension: None,
                                    },
                                );
                            }
                        });
                    }
                });
            }
        } else if self.config.ui.asset_pack.is_some() {
            ui.colored_label(egui::Color32::YELLOW, "The profile references an asset pack that could not be opened. Use Import Asset Pack or choose a different root.");
        } else {
            ui.label("Import a presentation asset pack to browse artwork and map exact glyphs.");
        }

        if !image_ids.is_empty() {
            egui::CollapsingHeader::new("Image and sprite assets").show(ui, |ui| {
                for id in image_ids.iter().take(80) {
                    let path = self
                        .config
                        .ui
                        .loaded_asset_path(id)
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "not loaded".into());
                    ui.label(format!("{id} — {path}"));
                }
                if image_ids.len() > 80 {
                    ui.label(format!(
                        "{} additional registered assets",
                        image_ids.len() - 80
                    ));
                }
            });
        }
        if !image_ids.is_empty() {
            egui::CollapsingHeader::new("Artwork previews").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for id in image_ids.iter().take(80) {
                            ui.horizontal(|ui| {
                                let path =
                                    self.config.ui.loaded_asset_path(id).map(Path::to_path_buf);
                                if let Some(texture) = path.as_ref().and_then(|path| {
                                    gameplay_ui_asset_texture(
                                        ui.ctx(),
                                        &mut self.gameplay_ui_asset_thumbnail_cache,
                                        id,
                                        path,
                                    )
                                }) {
                                    ui.add(egui::Image::new((texture, egui::vec2(48.0, 48.0))));
                                } else {
                                    ui.allocate_ui(egui::vec2(48.0, 48.0), |ui| {
                                        ui.centered_and_justified(|ui| ui.label("No preview"));
                                    });
                                }
                                ui.vertical(|ui| {
                                    ui.label(id);
                                    ui.label(
                                        path.as_ref()
                                            .map(|path| path.display().to_string())
                                            .unwrap_or_else(|| "Not loaded".into()),
                                    );
                                });
                            });
                        }
                    });
                if image_ids.len() > 80 {
                    ui.label(format!(
                        "{} additional assets have previews when selected",
                        image_ids.len() - 80
                    ));
                }
            });
        }
        if let Some(path) = self.config.ui.resolved_asset_pack_path() {
            if ui.button("Select / repair asset root...").clicked() {
                let mut dialog = rfd::FileDialog::new().set_title("Select presentation asset root");
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(root) = dialog.pick_folder() {
                    self.import_gameplay_ui_pack(&path, Some(root));
                }
            }
        }

        ui.separator();
        ui.heading("HUD elements");
        ui.horizontal_wrapped(|ui| {
            for (label, source) in [
                ("Add Score Counter", crate::render_ui::UiStateSource::Score),
                ("Add Combo Counter", crate::render_ui::UiStateSource::Combo),
                ("Add Life Counter", crate::render_ui::UiStateSource::Life),
            ] {
                if ui.button(label).clicked() {
                    let id = format!(
                        "{}-counter",
                        label
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("value")
                            .to_lowercase()
                    );
                    let id = crate::render_ui::unique_ui_element_id(&self.config.ui.elements, &id);
                    self.config.ui.elements.push(crate::render_ui::UiElement {
                        id: id.clone(),
                        z_index: self.config.ui.elements.len() as i32,
                        layout: crate::render_ui::UiLayout {
                            x: 0.05,
                            y: 0.05 + 0.08 * self.config.ui.elements.len() as f64,
                            width: 0.30,
                            height: 0.07,
                            ..Default::default()
                        },
                        kind: crate::render_ui::UiElementKind::Counter {
                            value: crate::render_ui::UiExpression::Source { source },
                            prefix: String::new(),
                            suffix: String::new(),
                            digits: 0,
                            max_digits: None,
                            color: [255, 255, 255, 255],
                            font_size: 42.0,
                            font: None,
                            align: crate::render_ui::UiTextAlign::Left,
                        },
                        ..Default::default()
                    });
                    self.config.ui.intentionally_empty = false;
                    self.gameplay_ui_selected_element = Some(id);
                }
            }
            if ui
                .add_enabled(
                    !image_ids.is_empty(),
                    egui::Button::new("Add Image Element"),
                )
                .clicked()
            {
                let asset = image_ids[0].clone();
                let id = crate::render_ui::unique_ui_element_id(
                    &self.config.ui.elements,
                    "image-element",
                );
                self.config.ui.elements.push(crate::render_ui::UiElement {
                    id: id.clone(),
                    z_index: self.config.ui.elements.len() as i32,
                    layout: crate::render_ui::UiLayout {
                        x: 0.35,
                        y: 0.35,
                        width: 0.30,
                        height: 0.20,
                        ..Default::default()
                    },
                    kind: crate::render_ui::UiElementKind::Image { asset, tint: None },
                    ..Default::default()
                });
                self.config.ui.intentionally_empty = false;
                self.gameplay_ui_selected_element = Some(id);
                assets_changed = true;
            }
        });
        ui.label("Counter bindings read the existing gameplay state. If no authoritative Watch/event source provides a value, that element stays blank in real renders.");
        if self.config.ui.elements.is_empty() {
            ui.colored_label(egui::Color32::YELLOW, "This profile is empty. You can restore the bundled Sono-Overlay layout or keep an intentional blank profile.");
            if ui.button("Populate with the default HUD").clicked()
                && self.config.ui.populate_with_default_hud()
            {
                match self.config.ui.load_external_data() {
                    Ok(()) => {
                        self.gameplay_ui_selected_element = self
                            .config
                            .ui
                            .elements
                            .first()
                            .map(|element| element.id.clone());
                        self.summary = "Restored the bundled gameplay HUD layout.".into();
                        self.error = false;
                    }
                    Err(error) => {
                        self.summary = format!("The HUD layout was restored, but its assets could not be loaded: {error:#}");
                        self.error = true;
                    }
                }
            }
        }

        let ids: Vec<String> = self
            .config
            .ui
            .elements
            .iter()
            .map(|element| element.id.clone())
            .collect();
        if self
            .gameplay_ui_selected_element
            .as_ref()
            .is_none_or(|selected| !ids.contains(selected))
        {
            self.gameplay_ui_selected_element = ids.first().cloned();
        }
        let mut duplicate_element = None;
        let mut delete_element = None;
        for element in &mut self.config.ui.elements {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        self.gameplay_ui_selected_element.as_deref() == Some(element.id.as_str()),
                        &element.id,
                    )
                    .clicked()
                {
                    self.gameplay_ui_selected_element = Some(element.id.clone());
                }
                ui.checkbox(&mut element.enabled, "Visible");
                if ui.small_button("Duplicate").clicked() {
                    duplicate_element = Some(element.id.clone());
                }
                if ui.small_button("Delete").clicked() {
                    delete_element = Some(element.id.clone());
                }
            });
        }
        if let Some(element_id) = delete_element {
            self.config
                .ui
                .elements
                .retain(|element| element.id != element_id);
            self.config.ui.intentionally_empty = self.config.ui.elements.is_empty();
            self.gameplay_ui_selected_element = self
                .config
                .ui
                .elements
                .first()
                .map(|element| element.id.clone());
        }
        if let Some(element_id) = duplicate_element {
            if let Some(mut element) = self
                .config
                .ui
                .elements
                .iter()
                .find(|element| element.id == element_id)
                .cloned()
            {
                let base_id = format!("{}-copy", element.id);
                element.id =
                    crate::render_ui::unique_ui_element_id(&self.config.ui.elements, &base_id);
                element.z_index = self
                    .config
                    .ui
                    .elements
                    .iter()
                    .map(|element| element.z_index)
                    .max()
                    .unwrap_or(element.z_index)
                    .saturating_add(1);
                element.layout.x += 0.015;
                element.layout.y += 0.015;
                let id = element.id.clone();
                self.config.ui.elements.push(element);
                self.config.ui.intentionally_empty = false;
                self.gameplay_ui_selected_element = Some(id);
            }
        }
        if let Some(selected_id) = self.gameplay_ui_selected_element.clone() {
            if let Some(element) = self
                .config
                .ui
                .elements
                .iter_mut()
                .find(|element| element.id == selected_id)
            {
                ui.group(|ui| {
                    ui.label(format!("Properties: {}", element.id));
                    ui.horizontal(|ui| {
                        ui.label("X");
                        ui.add(
                            egui::DragValue::new(&mut element.layout.x)
                                .speed(0.002)
                                .range(-1.0..=2.0),
                        );
                        ui.label("Y");
                        ui.add(
                            egui::DragValue::new(&mut element.layout.y)
                                .speed(0.002)
                                .range(-1.0..=2.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Width");
                        ui.add(
                            egui::DragValue::new(&mut element.layout.width)
                                .speed(0.002)
                                .range(0.001..=2.0),
                        );
                        ui.label("Height");
                        ui.add(
                            egui::DragValue::new(&mut element.layout.height)
                                .speed(0.002)
                                .range(0.001..=2.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Scale X/Y");
                        ui.add(
                            egui::DragValue::new(&mut element.layout.scale[0])
                                .speed(0.01)
                                .range(0.01..=10.0),
                        );
                        ui.add(
                            egui::DragValue::new(&mut element.layout.scale[1])
                                .speed(0.01)
                                .range(0.01..=10.0),
                        );
                        ui.label("Opacity");
                        ui.add(egui::Slider::new(&mut element.opacity, 0.0..=1.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Layer order");
                        ui.add(egui::DragValue::new(&mut element.z_index));
                        egui::ComboBox::from_label("Anchor")
                            .selected_text(format!("{:?}", element.layout.anchor))
                            .show_ui(ui, |ui| {
                                for (anchor, label) in [
                                    (crate::render_ui::UiAnchor::TopLeft, "Top left"),
                                    (crate::render_ui::UiAnchor::Top, "Top"),
                                    (crate::render_ui::UiAnchor::TopRight, "Top right"),
                                    (crate::render_ui::UiAnchor::Left, "Left"),
                                    (crate::render_ui::UiAnchor::Center, "Center"),
                                    (crate::render_ui::UiAnchor::Right, "Right"),
                                    (crate::render_ui::UiAnchor::BottomLeft, "Bottom left"),
                                    (crate::render_ui::UiAnchor::Bottom, "Bottom"),
                                    (crate::render_ui::UiAnchor::BottomRight, "Bottom right"),
                                ] {
                                    ui.selectable_value(&mut element.layout.anchor, anchor, label);
                                }
                            });
                    });
                    match &mut element.kind {
                        crate::render_ui::UiElementKind::Counter {
                            value, font, align, ..
                        } => {
                            if let crate::render_ui::UiExpression::Source { source } = value {
                                egui::ComboBox::from_label("Gameplay binding")
                                    .selected_text(format!("{:?}", source))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            source,
                                            crate::render_ui::UiStateSource::Score,
                                            "Score",
                                        );
                                        ui.selectable_value(
                                            source,
                                            crate::render_ui::UiStateSource::Combo,
                                            "Combo",
                                        );
                                        ui.selectable_value(
                                            source,
                                            crate::render_ui::UiStateSource::Life,
                                            "Life",
                                        );
                                    });
                            } else {
                                ui.label("Custom expression binding");
                            }
                            let mut selected_font = font.clone();
                            egui::ComboBox::from_label("Bitmap glyph family")
                                .selected_text(
                                    selected_font
                                        .clone()
                                        .unwrap_or_else(|| "Built-in text".into()),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected_font, None, "Built-in text");
                                    for name in self.config.ui.fonts.keys() {
                                        ui.selectable_value(
                                            &mut selected_font,
                                            Some(name.clone()),
                                            name,
                                        );
                                    }
                                });
                            *font = selected_font;
                            egui::ComboBox::from_label("Text alignment")
                                .selected_text(format!("{:?}", align))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        align,
                                        crate::render_ui::UiTextAlign::Left,
                                        "Left",
                                    );
                                    ui.selectable_value(
                                        align,
                                        crate::render_ui::UiTextAlign::Center,
                                        "Center",
                                    );
                                    ui.selectable_value(
                                        align,
                                        crate::render_ui::UiTextAlign::Right,
                                        "Right",
                                    );
                                });
                        }
                        crate::render_ui::UiElementKind::Image { asset, .. }
                        | crate::render_ui::UiElementKind::Icon { asset, .. }
                        | crate::render_ui::UiElementKind::Sprite { asset, .. } => {
                            let mut selected = asset.clone();
                            egui::ComboBox::from_label("Image asset")
                                .selected_text(selected.clone())
                                .show_ui(ui, |ui| {
                                    for id in &image_ids {
                                        ui.selectable_value(&mut selected, id.clone(), id);
                                    }
                                });
                            assets_changed |= selected != *asset;
                            *asset = selected;
                        }
                        crate::render_ui::UiElementKind::Gauge {
                            fill_asset,
                            mask_asset,
                            ..
                        }
                        | crate::render_ui::UiElementKind::ProgressBar {
                            fill_asset,
                            mask_asset,
                            ..
                        } => {
                            let mut selected_fill = fill_asset.clone();
                            egui::ComboBox::from_label("Fill image asset")
                                .selected_text(
                                    selected_fill
                                        .clone()
                                        .unwrap_or_else(|| "Solid-color fill".into()),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut selected_fill,
                                        None,
                                        "Solid-color fill",
                                    );
                                    for id in &image_ids {
                                        ui.selectable_value(
                                            &mut selected_fill,
                                            Some(id.clone()),
                                            id,
                                        );
                                    }
                                });
                            assets_changed |= selected_fill != *fill_asset;
                            *fill_asset = selected_fill;

                            let mut selected_mask = mask_asset.clone();
                            egui::ComboBox::from_label("Fill mask asset")
                                .selected_text(
                                    selected_mask.clone().unwrap_or_else(|| "No mask".into()),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected_mask, None, "No mask");
                                    for id in &image_ids {
                                        ui.selectable_value(
                                            &mut selected_mask,
                                            Some(id.clone()),
                                            id,
                                        );
                                    }
                                });
                            assets_changed |= selected_mask != *mask_asset;
                            *mask_asset = selected_mask;
                        }
                        _ => {
                            ui.label(
                                "This element's visual type uses its existing profile settings.",
                            );
                        }
                    }
                });
            }
        }
        if assets_changed {
            self.gameplay_ui_asset_thumbnail_cache.clear();
            if let Err(error) = self.config.ui.load_external_data() {
                self.summary = format!("Could not load selected glyph artwork: {error:#}");
                self.error = true;
            }
        }

        ui.separator();
        ui.heading("Gameplay values");
        if let Some(path) = self.config.ui.events_file.clone() {
            ui.label("Gameplay values: Manually loaded event track (overrides the automatic All Perfect forecast).");
            ui.label(format!(
                "Event track: {} ({} events)",
                path.display(),
                self.config.ui.loaded_events.len()
            ));
            ui.label(format!(
                "{} scoring rules are configured in this profile.",
                self.config
                    .ui
                    .scoring
                    .as_ref()
                    .map_or(0, |scoring| scoring.rules.len())
            ));
            ui.label("Event values come from this event track and the profile rules below. The frame renderer applies events in Watch-time order.");
        } else {
            match self.config.ui.resolved_scoring_source() {
                crate::render_ui::UiScoringSource::Automatic
                | crate::render_ui::UiScoringSource::SonoOverlay => {
                    ui.label("Gameplay values: Auto-generated All Perfect from recognized chart notes for preview and export.");
                }
                crate::render_ui::UiScoringSource::EventTrack => {
                    ui.label("Gameplay values: Event track selected, but no event file is loaded.");
                }
                crate::render_ui::UiScoringSource::Disabled => {
                    ui.label("Gameplay values are disabled. Watch rendering still runs normally.");
                }
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Load / replace event track...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Load gameplay event track")
                    .add_filter("Gameplay event track", &["json"]);
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.pick_file() {
                    let mut candidate = self.config.ui.clone();
                    candidate.events_file = Some(path.clone());
                    candidate
                        .scoring
                        .get_or_insert_with(crate::render_ui::UiScoringConfig::default)
                        .source = crate::render_ui::UiScoringSource::EventTrack;
                    match candidate.load_external_data() {
                        Ok(()) => {
                            self.config.ui = candidate;
                            self.summary = format!(
                                "Loaded {} gameplay events from {}.",
                                self.config.ui.loaded_events.len(),
                                path.display()
                            );
                            self.error = false;
                        }
                        Err(error) => {
                            self.summary = format!("Could not load gameplay events: {error:#}");
                            self.error = true;
                        }
                    }
                }
            }
            if self.config.ui.events_file.is_some() && ui.button("Remove event track").clicked() {
                let mut candidate = self.config.ui.clone();
                candidate.events_file = None;
                if let Some(scoring) = &mut candidate.scoring {
                    if scoring.source == crate::render_ui::UiScoringSource::EventTrack {
                        scoring.source = crate::render_ui::UiScoringSource::Automatic;
                    }
                }
                match candidate.load_external_data() {
                    Ok(()) => {
                        self.config.ui = candidate;
                        self.summary = "Removed event track. Chart preview and export use automatic All Perfect values.".into();
                        self.error = false;
                    }
                    Err(error) => {
                        self.summary = format!("Could not clear gameplay events: {error:#}");
                        self.error = true;
                    }
                }
            }
        });

        self.gameplay_ui_scoring_editor(ui);

        ui.separator();
        ui.heading("Real gameplay preview");
        ui.label("Render a still frame from the configured engine and level through the production Watch and HUD compositor. Program Preview reports which event-backed values were actually supplied for that frame.");
        if ui.button("Open Program Preview").clicked() {
            self.select_workflow_page(crate::workspace::WorkflowId::Tools, 13);
        }
        ui.label("Choose a Watch timestamp there to see the gameplay background with this HUD profile. The design preview below uses sample values only.");

        ui.separator();
        ui.heading("Design preview — sample values");
        ui.label("Sample values: score 123456, combo 789, life 880, judgment PERFECT. These values are for layout only and never enter Watch or an export.");
        let preview_width = 480u32;
        let preview_height = 270u32;
        let mut rgb = vec![24u8; preview_width as usize * preview_height as usize * 3];
        for pixel in rgb.chunks_exact_mut(3) {
            pixel.copy_from_slice(&[20, 25, 36]);
        }
        let mut preview_config = self.config.ui.clone();
        preview_config.enabled = true;
        preview_config.mode = crate::render_ui::GameplayUiMode::Custom;
        if let Err(error) = preview_config.prepare_external_data() {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("Preview assets could not be loaded: {error:#}"),
            );
        }
        let sample = crate::render_ui::UiFrameState {
            gameplay: Some(crate::render_ui::UiGameplayState {
                score: 123456.0,
                combo: 789,
                life: 880.0,
                judgment: Some("PERFECT".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        match crate::render_ui::render_gameplay_ui(
            &mut rgb,
            preview_width,
            preview_height,
            &preview_config,
            &sample,
        ) {
            Ok(()) => {
                let image = egui::ColorImage::from_rgb(
                    [preview_width as usize, preview_height as usize],
                    &rgb,
                );
                if let Some(texture) = &mut self.gameplay_ui_preview_texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.gameplay_ui_preview_texture = Some(ui.ctx().load_texture(
                        "gameplay-ui-design-preview",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                if let Some(texture) = &self.gameplay_ui_preview_texture {
                    ui.add(egui::Image::new((texture.id(), egui::vec2(480.0, 270.0))));
                }
            }
            Err(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("Preview unavailable: {error:#}"),
                );
            }
        }
        ui.label(format!(
            "{} elements · {} registered image assets · {} mapped glyph fonts",
            self.config.ui.elements.len(),
            self.config.ui.assets.len(),
            self.config.ui.fonts.len()
        ));
        ui.label("Profile keyframes animate presentation values; they do not change authoritative Sonolus gameplay state.");
    }

    fn watermark_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Renderer watermark");
        ui.label("One master switch controls both the fixed attribution and the optional moving watermark.");
        ui.checkbox(
            &mut self.config.watermark.enabled,
            "Enable watermark system",
        );
        if !self.config.watermark.enabled {
            self.config.watermark.moving.enabled = false;
        }
        ui.horizontal(|ui| {
            if ui.button("Load watermark profile...").clicked() {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Load watermark profile")
                    .add_filter("JSON profile", &["json"]);
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.pick_file() {
                    match self.config.set_watermark_profile(&path) {
                        Ok(()) => {
                            self.summary = format!("Loaded watermark profile: {}", path.display());
                            self.error = false;
                        }
                        Err(error) => {
                            self.summary = format!("Watermark profile failed: {error:#}");
                            self.error = true;
                        }
                    }
                }
            }
            ui.label(format!(
                "{} custom image assets",
                self.config.watermark.assets.len()
            ));
        });
        ui.label(crate::watermark::ATTRIBUTION_TEXT);
        ui.label("Fixed attribution location: bottom-left. Text and location cannot be changed.");
        ui.add_enabled_ui(self.config.watermark.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Slider::new(&mut self.config.watermark.attribution.font_size, 6.0..=96.0)
                        .text("Attribution size"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.watermark.attribution.opacity, 0.0..=1.0)
                        .text("Opacity"),
                );
            });
            ui.checkbox(
                &mut self.config.watermark.moving.enabled,
                "Enable moving watermark",
            );
            ui.add_enabled_ui(self.config.watermark.moving.enabled, |ui| {
                let moving = &mut self.config.watermark.moving;
                if let crate::watermark::WatermarkContent::Text { text } = &mut moving.content {
                    ui.horizontal(|ui| {
                        ui.label("Moving text");
                        ui.text_edit_singleline(text);
                    });
                } else {
                    ui.label(
                        "Moving image and sprite source are configured in the watermark profile.",
                    );
                }
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut moving.font_size, 6.0..=256.0).text("Text size"));
                    ui.add(
                        egui::Slider::new(&mut moving.image_height, 1.0..=1024.0)
                            .text("Image height"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut moving.opacity, 0.0..=1.0).text("Opacity"));
                    ui.add(egui::Slider::new(&mut moving.scale, 0.05..=8.0).text("Scale"));
                    ui.add(
                        egui::Slider::new(&mut moving.rotation_degrees, -180.0..=180.0)
                            .text("Rotation"),
                    );
                });
                egui::ComboBox::from_label("Anchor")
                    .selected_text(format!("{:?}", moving.anchor))
                    .show_ui(ui, |ui| {
                        for (anchor, name) in [
                            (crate::watermark::WatermarkAnchor::TopLeft, "Top left"),
                            (crate::watermark::WatermarkAnchor::TopCenter, "Top center"),
                            (crate::watermark::WatermarkAnchor::TopRight, "Top right"),
                            (crate::watermark::WatermarkAnchor::CenterLeft, "Center left"),
                            (crate::watermark::WatermarkAnchor::Center, "Center"),
                            (
                                crate::watermark::WatermarkAnchor::CenterRight,
                                "Center right",
                            ),
                            (crate::watermark::WatermarkAnchor::BottomLeft, "Bottom left"),
                            (
                                crate::watermark::WatermarkAnchor::BottomCenter,
                                "Bottom center",
                            ),
                            (
                                crate::watermark::WatermarkAnchor::BottomRight,
                                "Bottom right",
                            ),
                        ] {
                            ui.selectable_value(&mut moving.anchor, anchor, name);
                        }
                    });
                let motion = match moving.movement {
                    crate::watermark::WatermarkMovement::Static => 0,
                    crate::watermark::WatermarkMovement::Linear { .. } => 1,
                    crate::watermark::WatermarkMovement::Waypoints { .. } => 2,
                    crate::watermark::WatermarkMovement::RandomWaypoints { .. } => 3,
                };
                let mut selected_motion = motion;
                egui::ComboBox::from_label("Movement")
                    .selected_text(["Static", "Linear", "Waypoints", "Random waypoints"][motion])
                    .show_ui(ui, |ui| {
                        for (value, label) in ["Static", "Linear", "Waypoints", "Random waypoints"]
                            .into_iter()
                            .enumerate()
                        {
                            ui.selectable_value(&mut selected_motion, value, label);
                        }
                    });
                if selected_motion != motion {
                    moving.movement = match selected_motion {
                        0 => crate::watermark::WatermarkMovement::Static,
                        1 => crate::watermark::WatermarkMovement::Linear {
                            velocity: [0.08, 0.06],
                            boundary: crate::watermark::LinearBoundary::Bounce,
                        },
                        2 => crate::watermark::WatermarkMovement::Waypoints {
                            points: vec![[0.15, 0.2], [0.85, 0.8]],
                            segment_seconds: 2.0,
                            interpolation: Default::default(),
                            repeat: true,
                        },
                        _ => crate::watermark::WatermarkMovement::RandomWaypoints {
                            segment_seconds: 2.0,
                            interpolation: Default::default(),
                        },
                    };
                }
                if let crate::watermark::WatermarkMovement::Linear { velocity, boundary } =
                    &mut moving.movement
                {
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut velocity[0], -2.0..=2.0).text("X speed"));
                        ui.add(egui::Slider::new(&mut velocity[1], -2.0..=2.0).text("Y speed"));
                        egui::ComboBox::from_label("Boundary")
                            .selected_text(format!("{:?}", boundary))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    boundary,
                                    crate::watermark::LinearBoundary::Bounce,
                                    "Bounce",
                                );
                                ui.selectable_value(
                                    boundary,
                                    crate::watermark::LinearBoundary::Wrap,
                                    "Wrap",
                                );
                                ui.selectable_value(
                                    boundary,
                                    crate::watermark::LinearBoundary::Clamp,
                                    "Clamp",
                                );
                            });
                    });
                }
                if let crate::watermark::WatermarkMovement::RandomWaypoints {
                    segment_seconds,
                    ..
                }
                | crate::watermark::WatermarkMovement::Waypoints {
                    segment_seconds, ..
                } = &mut moving.movement
                {
                    ui.add(
                        egui::Slider::new(segment_seconds, 0.1..=60.0)
                            .text("Movement interval (seconds)"),
                    );
                }
                let mut animate = moving.animation.is_some();
                ui.checkbox(&mut animate, "Animate scale/rotation/opacity");
                if animate && moving.animation.is_none() {
                    moving.animation = Some(Default::default());
                } else if !animate {
                    moving.animation = None;
                }
                if let Some(animation) = &mut moving.animation {
                    ui.add(
                        egui::Slider::new(&mut animation.period_seconds, 0.1..=120.0)
                            .text("Animation period (seconds)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut animation.scale_amplitude, 0.0..=1.0)
                            .text("Scale amplitude"),
                    );
                    ui.add(
                        egui::Slider::new(&mut animation.rotation_amplitude_degrees, 0.0..=180.0)
                            .text("Rotation amplitude"),
                    );
                    ui.add(
                        egui::Slider::new(&mut animation.opacity_amplitude, 0.0..=1.0)
                            .text("Opacity amplitude"),
                    );
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut moving.timing.start_time)
                            .speed(0.1)
                            .range(0.0..=86400.0)
                            .prefix("Start (s) "),
                    );
                    let mut indefinite = moving.timing.duration.is_none();
                    ui.checkbox(&mut indefinite, "Until end");
                    if indefinite {
                        moving.timing.duration = None;
                    } else {
                        let mut duration = moving.timing.duration.unwrap_or(10.0);
                        ui.add(
                            egui::DragValue::new(&mut duration)
                                .speed(0.1)
                                .range(0.1..=86400.0)
                                .prefix("Duration (s) "),
                        );
                        moving.timing.duration = Some(duration);
                    }
                    ui.checkbox(&mut moving.timing.repeat, "Repeat");
                });
                egui::ComboBox::from_label("Layer")
                    .selected_text(format!("{:?}", moving.layer))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut moving.layer,
                            crate::watermark::WatermarkLayer::BeforeGameplayUi,
                            "Before gameplay UI",
                        );
                        ui.selectable_value(
                            &mut moving.layer,
                            crate::watermark::WatermarkLayer::AfterGameplayUi,
                            "After gameplay UI",
                        );
                        ui.selectable_value(
                            &mut moving.layer,
                            crate::watermark::WatermarkLayer::Topmost,
                            "Topmost",
                        );
                    });
            });
        });
    }

    fn notifications_settings(&mut self, ui: &mut egui::Ui) {
        use crate::notifications::ProgressMode;
        ui.heading("Notifications");
        ui.label("Notifications run on a bounded background worker. Delivery errors never stop an export.");
        ui.checkbox(
            &mut self.config.notifications.enabled,
            "Enable notifications",
        );
        ui.add_enabled_ui(self.config.notifications.enabled, |ui| {
            ui.separator();
            ui.checkbox(&mut self.config.notifications.native.enabled, "Native desktop notifications");
            ui.add_enabled_ui(self.config.notifications.native.enabled, |ui| {
                event_filter_ui(ui, "Native events", &mut self.config.notifications.native.events);
                ui.collapsing("Native message templates", |ui| {
                    event_templates_ui(ui, &mut self.config.notifications.native.templates, false);
                });
            });

            ui.separator();
            ui.checkbox(&mut self.config.notifications.discord.enabled, "Discord bot notifications");
            ui.add_enabled_ui(self.config.notifications.discord.enabled, |ui| {
                ui.label("The bot token is read from an environment variable and is never saved in this configuration.");
                ui.horizontal(|ui| {
                    ui.label("Bot ID (optional)");
                    ui.text_edit_singleline(
                        self.config.notifications.discord.bot_id.get_or_insert_with(String::new),
                    );
                    if self.config.notifications.discord.bot_id.as_deref() == Some("") {
                        self.config.notifications.discord.bot_id = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Token environment variable");
                    ui.text_edit_singleline(&mut self.config.notifications.discord.bot_token_env);
                });
                ui.label("Set that variable in the environment that launches Sono-Renderer. Bot needs permission to view and send messages in each selected channel.");
                let mut remove_destination = None;
                for (index, destination) in self.config.notifications.discord.destinations.iter_mut().enumerate() {
                    ui.push_id(("discord-destination", index), |ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut destination.enabled, format!("Destination {}", index + 1));
                                if ui.small_button("Remove").clicked() {
                                    remove_destination = Some(index);
                                }
                            });
                            ui.add_enabled_ui(destination.enabled, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Guild/server ID (optional)");
                                    let mut guild = destination.guild_id.clone().unwrap_or_default();
                                    if ui.text_edit_singleline(&mut guild).changed() {
                                        destination.guild_id = (!guild.trim().is_empty()).then(|| guild.trim().to_owned());
                                    }
                                });
                                ui.horizontal(|ui| {
                                    ui.label("Channel ID");
                                    ui.text_edit_singleline(&mut destination.channel_id);
                                });
                                event_filter_ui(ui, "Destination events", &mut destination.events);
                                ui.collapsing("Message templates", |ui| {
                                    event_templates_ui(ui, &mut destination.templates, true);
                                });
                            });
                        });
                    });
                }
                if let Some(index) = remove_destination {
                    self.config.notifications.discord.destinations.remove(index);
                }
                if ui.button("Add Discord destination").clicked() {
                    self.config.notifications.discord.destinations.push(Default::default());
                }
            });

            ui.separator();
            ui.heading("Progress notifications");
            ui.label("Progress messages also need Render progress selected in the target backend's event filter above.");
            egui::ComboBox::from_label("Progress mode")
                .selected_text(match self.config.notifications.progress_mode {
                    ProgressMode::Disabled => "Disabled",
                    ProgressMode::Milestones => "Milestones",
                    ProgressMode::Periodic => "Periodic",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.config.notifications.progress_mode, ProgressMode::Disabled, "Disabled");
                    ui.selectable_value(&mut self.config.notifications.progress_mode, ProgressMode::Milestones, "Milestones");
                    ui.selectable_value(&mut self.config.notifications.progress_mode, ProgressMode::Periodic, "Periodic");
                });
            ui.add_enabled_ui(self.config.notifications.progress_mode == ProgressMode::Periodic, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Minimum interval (seconds)");
                    ui.add(egui::DragValue::new(&mut self.config.notifications.progress_interval_seconds).range(1..=3600));
                });
            });
            ui.add_enabled_ui(self.config.notifications.progress_mode == ProgressMode::Milestones, |ui| {
                let mut remove = None;
                for (index, milestone) in self.config.notifications.progress_milestones.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(milestone, 1..=100).suffix("%"));
                        if ui.small_button("Remove").clicked() { remove = Some(index); }
                    });
                }
                if let Some(index) = remove {
                    self.config.notifications.progress_milestones.remove(index);
                }
                if ui.button("Add milestone").clicked() {
                    self.config.notifications.progress_milestones.push(50);
                    self.config.notifications.progress_milestones.sort_unstable();
                    self.config.notifications.progress_milestones.dedup();
                }
            });
            for diagnostic in self.config.notifications.diagnostics() {
                ui.colored_label(egui::Color32::YELLOW, format!("Configuration note: {diagnostic}"));
            }
        });
    }

    fn job_manager_ui(&mut self, ui: &mut egui::Ui) {
        use crate::job_manager::JobState;
        ui.heading("Render job manager");
        ui.label("Each job captures the current render configuration. Queue state is saved between application runs.");
        if let Some(error) = &self.job_manager_error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("Queue unavailable: {error}"),
            );
            return;
        }
        let Some(manager) = &self.job_manager else {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Queue manager was not initialized.",
            );
            return;
        };
        ui.label(format!(
            "Persistent queue: {}",
            manager.store_path().display()
        ));
        let mut scheduler = self.config.job_manager.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Maximum concurrent renders");
            changed |= ui
                .add(egui::Slider::new(&mut scheduler.max_concurrent_jobs, 1..=8))
                .changed();
        });
        egui::ComboBox::from_label("Default priority")
            .selected_text(format!("{:?}", scheduler.default_priority))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(
                        &mut scheduler.default_priority,
                        crate::job_manager::JobPriority::High,
                        "High",
                    )
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut scheduler.default_priority,
                        crate::job_manager::JobPriority::Normal,
                        "Normal",
                    )
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut scheduler.default_priority,
                        crate::job_manager::JobPriority::Low,
                        "Low",
                    )
                    .changed();
            });
        if changed {
            self.config.job_manager = scheduler.clone();
            if let Err(error) = manager.configure(scheduler) {
                self.summary = format!("Queue setting failed: {error:#}");
                self.error = true;
            }
        }
        let mut enqueue_current = false;
        ui.horizontal(|ui| {
            if ui.button("Add current render to queue").clicked() {
                enqueue_current = true;
            }
            match manager.is_paused() {
                Ok(true) => {
                    if ui.button("Resume queue").clicked() {
                        if let Err(error) = manager.resume() {
                            self.summary = format!("Could not resume queue: {error:#}");
                            self.error = true;
                        }
                    }
                }
                Ok(false) => {
                    if ui.button("Pause queue").clicked() {
                        if let Err(error) = manager.pause() {
                            self.summary = format!("Could not pause queue: {error:#}");
                            self.error = true;
                        }
                    }
                }
                Err(error) => {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!("Queue state unavailable: {error:#}"),
                    );
                }
            }
            if ui.button("Cancel queued jobs").clicked() {
                match manager.cancel_queued() {
                    Ok(count) => self.summary = format!("Cancelled {count} queued job(s)."),
                    Err(error) => {
                        self.summary = format!("Could not cancel queued jobs: {error:#}");
                        self.error = true;
                    }
                }
            }
            if ui.button("Clear completed / failed").clicked() {
                match manager.clear_terminal() {
                    Ok(count) => self.summary = format!("Removed {count} terminal job(s)."),
                    Err(error) => {
                        self.summary = format!("Could not clear jobs: {error:#}");
                        self.error = true;
                    }
                }
            }
        });
        if enqueue_current {
            if let Err(error) = obvious_validation(&self.config) {
                self.summary = format!("Cannot queue render: {error:#}");
                self.error = true;
            } else {
                let profile_source = self
                    .active_profile_id
                    .as_deref()
                    .and_then(|id| self.profile_manager.as_ref()?.get(id).ok());
                let enqueue = manager.enqueue_with_metadata(
                    self.config.clone(),
                    None,
                    profile_source.map(|profile| (profile.id, profile.name)),
                    crate::job_manager::RenderJobMetadata {
                        project_path: self.project_path.clone(),
                        ..Default::default()
                    },
                );
                match enqueue {
                    Ok(id) => {
                        self.summary = format!("Queued render job {id}");
                        self.error = false;
                        self.selected_job = Some(id);
                    }
                    Err(error) => {
                        self.summary = format!("Could not queue render: {error:#}");
                        self.error = true;
                    }
                }
            }
        }

        let jobs = match manager.jobs() {
            Ok(jobs) => jobs,
            Err(error) => {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("Could not read queue: {error:#}"),
                );
                return;
            }
        };
        let queued_positions: std::collections::HashMap<String, usize> = jobs
            .iter()
            .filter(|job| job.state == JobState::Queued)
            .enumerate()
            .map(|(index, job)| (job.id.clone(), index))
            .collect();
        #[derive(Clone)]
        enum Action {
            Cancel(String),
            Retry(String),
            Remove(String),
            Move(String, usize),
        }
        let mut action = None;
        ui.separator();
        if jobs.is_empty() {
            ui.label("No render jobs yet.");
        }
        egui::ScrollArea::vertical()
            .max_height(390.0)
            .show(ui, |ui| {
                for job in &jobs {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    self.selected_job.as_deref() == Some(&job.id),
                                    &job.id,
                                )
                                .clicked()
                            {
                                self.selected_job = Some(job.id.clone());
                            }
                            ui.strong(format!("{:?}", job.state));
                            ui.label(format!(
                                "{:?} priority · attempt {} · {}",
                                job.priority, job.attempt, job.execution_backend
                            ));
                        });
                        if let Some(profile_name) = &job.profile_name {
                            ui.label(format!(
                                "Source profile: {profile_name} (resolved snapshot)"
                            ));
                        }
                        ui.label(
                            job.output_path()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                        );
                        if let Some(progress) = &job.progress {
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "{:?} · {} / {} frames · {:.1}s",
                                    progress.phase,
                                    progress.completed_frames,
                                    progress
                                        .total_frames
                                        .map_or_else(|| "?".into(), |n| n.to_string()),
                                    progress.elapsed_seconds
                                ));
                                if let Some(percent) = progress.percentage {
                                    ui.add(
                                        egui::ProgressBar::new((percent / 100.0) as f32)
                                            .desired_width(130.0),
                                    );
                                }
                                if let Some(eta) = progress.eta_seconds {
                                    ui.label(format!("ETA {eta:.0}s"));
                                }
                            });
                        }
                        if let Some(diagnostic) = &job.diagnostic {
                            ui.colored_label(egui::Color32::LIGHT_RED, diagnostic);
                        }
                        if let Some(outcome) = &job.outcome {
                            ui.label(format!(
                                "{} frames · RGB SHA-1 {} · SFX SHA-1 {}",
                                outcome.submitted_frames,
                                outcome.rgb_sequence_sha1,
                                outcome.sfx_event_sha1
                            ));
                        }
                        ui.horizontal(|ui| {
                            if job.state == JobState::Queued {
                                if ui.small_button("Up").clicked() {
                                    if let Some(position) = queued_positions.get(&job.id) {
                                        action = Some(Action::Move(
                                            job.id.clone(),
                                            position.saturating_sub(1),
                                        ));
                                    }
                                }
                                if ui.small_button("Down").clicked() {
                                    if let Some(position) = queued_positions.get(&job.id) {
                                        action = Some(Action::Move(job.id.clone(), position + 1));
                                    }
                                }
                            }
                            if job.state.is_active() || job.state == JobState::Queued {
                                if ui.small_button("Cancel").clicked() {
                                    action = Some(Action::Cancel(job.id.clone()));
                                }
                            }
                            if job.state == JobState::Failed && ui.small_button("Retry").clicked() {
                                action = Some(Action::Retry(job.id.clone()));
                            }
                            if job.state.is_terminal() && ui.small_button("Remove").clicked() {
                                action = Some(Action::Remove(job.id.clone()));
                            }
                            ui.label(format!("{} log entries", job.logs.len()));
                        });
                    });
                }
            });
        if let Some(action) = action {
            let result = match action {
                Action::Cancel(id) => manager
                    .cancel(&id)
                    .map(|_| format!("Cancellation requested for {id}.")),
                Action::Retry(id) => manager
                    .retry(&id)
                    .map(|_| format!("Retry queued for {id}.")),
                Action::Remove(id) => manager.remove(&id).map(|_| format!("Removed {id}.")),
                Action::Move(id, position) => manager
                    .reorder(&id, position)
                    .map(|_| format!("Moved {id} to position {}.", position + 1)),
            };
            match result {
                Ok(message) => {
                    self.summary = message;
                    self.error = false;
                }
                Err(error) => {
                    self.summary = format!("Queue action failed: {error:#}");
                    self.error = true;
                }
            }
        }
        if let Some(id) = self.selected_job.clone() {
            if let Ok(job) = manager.inspect(&id) {
                ui.separator();
                ui.heading(format!("Job details: {id}"));
                egui::CollapsingHeader::new("Configuration snapshot").show(ui, |ui| {
                    if let Ok(mut json) = serde_json::to_string_pretty(&job.config) {
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut json)
                                        .code_editor()
                                        .desired_rows(8),
                                );
                            });
                    }
                });
                egui::CollapsingHeader::new("Job log")
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(150.0)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in &job.logs {
                                    ui.label(format!("{}  {}", line.unix_time_ms, line.message));
                                }
                            });
                        for previous in &job.previous_attempt_diagnostics {
                            ui.collapsing("Previous attempt diagnostic", |ui| {
                                ui.label(previous);
                            });
                        }
                    });
            }
        }
    }

    fn start_engine_job(&mut self, engine: PathBuf, compile_gcc: bool) {
        if compile_gcc {
            self.gcc_status = GccUiStatus::Loading;
        }
        let (tx, rx) = mpsc::sync_channel(2);
        let worker_engine = engine.clone();
        let worker = std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<EngineLoaded> {
                let package = crate::formats::load_engine(&worker_engine)
                    .map_err(|error| anyhow::anyhow!("loading engine package: {error:#}"))?;
                let options = package.configuration["options"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let nodes = package.watch.nodes.clone();
                let gcc = if compile_gcc {
                    let coverage = crate::sono_gcc::inspect_coverage(&nodes);
                    let _ = tx.send(EngineJobMessage::CheckingCache {
                        engine: worker_engine.clone(),
                        graph_nodes: coverage.graph_nodes,
                        eligible_operations: coverage.eligible_operations,
                    });
                    let compile_engine = worker_engine.clone();
                    let progress_tx = tx.clone();
                    Some(
                        crate::sono_gcc::precompile_with_progress(&nodes, move || {
                            let _ = progress_tx.send(EngineJobMessage::Compiling {
                                engine: compile_engine,
                                graph_nodes: coverage.graph_nodes,
                                eligible_operations: coverage.eligible_operations,
                            });
                        })
                        .map_err(|error| format!("{error:#}")),
                    )
                } else {
                    None
                };
                Ok(EngineLoaded {
                    options: Some(options),
                    nodes,
                    gcc,
                })
            })()
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(EngineJobMessage::Complete {
                engine: worker_engine,
                result,
            });
        });
        self.engine_job = Some(EngineJob { result: rx, worker });
    }

    fn start_gcc_job(&mut self, engine: PathBuf, nodes: Arc<Vec<crate::watch::EngineNode>>) {
        let coverage = crate::sono_gcc::inspect_coverage(&nodes);
        self.gcc_status = GccUiStatus::CheckingCache {
            graph_nodes: coverage.graph_nodes,
            eligible_operations: coverage.eligible_operations,
        };
        let (tx, rx) = mpsc::sync_channel(2);
        let worker_engine = engine.clone();
        let worker = std::thread::spawn(move || {
            let _ = tx.send(EngineJobMessage::CheckingCache {
                engine: worker_engine.clone(),
                graph_nodes: coverage.graph_nodes,
                eligible_operations: coverage.eligible_operations,
            });
            let compile_engine = worker_engine.clone();
            let progress_tx = tx.clone();
            let result = crate::sono_gcc::precompile_with_progress(&nodes, move || {
                let _ = progress_tx.send(EngineJobMessage::Compiling {
                    engine: compile_engine,
                    graph_nodes: coverage.graph_nodes,
                    eligible_operations: coverage.eligible_operations,
                });
            })
            .map_err(|error| format!("{error:#}"))
            .map(|report| EngineLoaded {
                options: None,
                nodes,
                gcc: Some(Ok(report)),
            });
            let _ = tx.send(EngineJobMessage::Complete {
                engine: worker_engine,
                result,
            });
        });
        self.engine_job = Some(EngineJob { result: rx, worker });
    }

    fn set_gcc_report(&mut self, report: crate::sono_gcc::SonoGccPrecompileReport) {
        self.gcc_status = status_for_gcc_report(report);
    }

    fn start_performance_benchmark(&mut self, ctx: &egui::Context) {
        if self.performance_job.is_some() {
            return;
        }
        let config = Self::gui_export_config(&self.config);
        let warmups = self.performance_warmups;
        let runs = self.performance_runs;
        let compare = self.performance_compare_modes;
        let measure_overhead = self.performance_measure_overhead;
        let wake = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result =
                crate::performance::run_benchmark(config, warmups, runs, compare, measure_overhead)
                    .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.performance_job = Some(PerformanceJob { result: rx, worker });
        self.performance_error = None;
        self.performance_summary =
            "Running real renderer benchmark in a temporary output directory...".into();
    }

    fn start_regression(&mut self, ctx: &egui::Context, update_reference: bool, replace: bool) {
        if self.regression_job.is_some() || self.job.is_some() || self.performance_job.is_some() {
            return;
        }
        let config = Self::gui_export_config(&self.config);
        let reference = self.regression_reference.clone();
        let options = crate::regression::RegressionOptions {
            fixture_id: self.regression_fixture_id.clone(),
            checkpoint_times: vec![self.regression_checkpoint_time],
            resolutions: vec![[config.width, config.height]],
            compare_vm_gcc: self.regression_compare_vm_gcc,
            timing_samples: self.regression_timing_samples,
            include_media: self.regression_include_media,
        };
        let wake = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let result = catch_export_panic(|| {
                if update_reference {
                    crate::regression::update_reference(&config, &options, &reference, replace)
                        .map(GuiRegressionOutcome::Updated)
                } else {
                    crate::regression::run_regression(&config, &reference, None, 0)
                        .map(GuiRegressionOutcome::Compared)
                }
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.regression_job = Some(RegressionJob { result: rx, worker });
        self.regression_error = None;
        self.regression_summary = if update_reference {
            "Rendering a new reference in a staging directory...".into()
        } else {
            "Rendering current checkpoints and comparing semantic, pixel, and timing layers..."
                .into()
        };
    }

    fn refresh_performance_history(&mut self) {
        if let Some(store) = &self.performance_store {
            match store.list(40) {
                Ok(records) => {
                    self.performance_records = records;
                    self.performance_error = None;
                }
                Err(error) => self.performance_error = Some(format!("{error:#}")),
            }
        }
    }

    fn poll(&mut self) {
        if let Some(job) = &self.job {
            if let Ok(result) = job.result.try_recv() {
                let job = self.job.take().unwrap();
                let _ = job.worker.join();
                match result {
                    Ok((text, diagnostic)) => {
                        self.summary = text;
                        self.error = false;
                        self.last_diagnostic = Some(diagnostic);
                        self.live.lock().unwrap().log(self.summary.clone());
                    }
                    Err(error) => {
                        let cancelled = job.control.is_cancelled() || error.is::<Cancelled>();
                        self.error = !cancelled;
                        let phase = self
                            .live
                            .lock()
                            .unwrap()
                            .progress
                            .as_ref()
                            .map(|p| format!("{:?}", p.phase));
                        let error_chain =
                            error.chain().map(ToString::to_string).collect::<Vec<_>>();
                        let diagnostic =
                            crate::render_diagnostics::RenderDiagnosticRecord::from_failure(
                                format!("diagnostic-gui-{}", crate::performance::unix_time_ms()),
                                None,
                                &job.config_snapshot,
                                phase,
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
                            self.performance_error = Some(format!(
                                "Could not persist GUI failure diagnostic: {persist_error:#}"
                            ));
                        }
                        self.last_diagnostic = Some(diagnostic);
                        self.summary = if cancelled {
                            "Cancelled. No export was published.".into()
                        } else {
                            error
                                .chain()
                                .last()
                                .unwrap()
                                .to_string()
                                .lines()
                                .next()
                                .unwrap_or("Export failed")
                                .to_owned()
                        };
                        self.live.lock().unwrap().log(format!("{error:#}"));
                    }
                }
            }
        }
        let manifest_result =
            self.manifest_job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("manifest worker ended without a result".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(result) = manifest_result {
            if let Some(job) = self.manifest_job.take() {
                let _ = job.worker.join();
            }
            match result {
                Ok(ManifestJobOutcome::Loaded {
                    path,
                    manifest,
                    report,
                }) => {
                    self.manifest_path = Some(path);
                    self.manifest = Some(*manifest);
                    self.manifest_report = Some(*report);
                    self.manifest_comparison = None;
                    self.manifest_error = None;
                }
                Ok(ManifestJobOutcome::Verified(report)) => {
                    self.manifest_report = Some(report);
                    self.manifest_error = None;
                }
                Ok(ManifestJobOutcome::Exported { path, sha256 }) => {
                    self.summary =
                        format!("Render manifest exported to {} ({sha256}).", path.display());
                    self.manifest_error = None;
                }
                Ok(ManifestJobOutcome::ReproductionPrepared { config, manifest }) => {
                    let result = self
                        .job_manager
                        .as_ref()
                        .context("render queue is unavailable")
                        .and_then(|manager| {
                            let mut config = *config;
                            // Queue concurrency belongs to this installation,
                            // not to the portable render manifest.
                            config.job_manager = manager.config();
                            manager.enqueue_with_metadata(
                                config,
                                None,
                                manifest
                                    .provenance
                                    .profile_id
                                    .clone()
                                    .zip(manifest.provenance.profile_name.clone()),
                                crate::job_manager::RenderJobMetadata {
                                    original_job_id: manifest.provenance.job_id.clone(),
                                    rerendered_from_history_id: manifest
                                        .provenance
                                        .history_id
                                        .clone(),
                                    reproduced_from_manifest_sha256: Some(
                                        manifest.manifest_sha256.clone(),
                                    ),
                                    ..Default::default()
                                },
                            )
                        });
                    match result {
                        Ok(job_id) => {
                            self.summary =
                                format!("Queued manifest reproduction as new job {job_id}.");
                            self.selected_job = Some(job_id);
                            self.manifest_error = None;
                            self.error = false;
                        }
                        Err(error) => self.manifest_error = Some(format!("{error:#}")),
                    }
                }
                Ok(ManifestJobOutcome::Compared(comparison)) => {
                    self.manifest_comparison = Some(comparison);
                    self.manifest_error = None;
                }
                Err(error) => self.manifest_error = Some(error),
            }
        }
        let performance_result =
            self.performance_job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("benchmark worker ended without a report".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(result) = performance_result {
            if let Some(job) = self.performance_job.take() {
                let _ = job.worker.join();
            }
            match result {
                Ok(report) => {
                    self.performance_latest_report = Some(report.clone());
                    self.performance_summary = report
                        .vm_vs_sono_gcc
                        .as_ref()
                        .map(|comparison| {
                            let summary = format!(
                                "Benchmark complete. VM median {:.1}ms; Sono-GCC median {:.1}ms ({}; {:+.1}ms).",
                                comparison.interpreter_median_ms,
                                comparison.sono_gcc_median_ms,
                                comparison.classification,
                                comparison.absolute_difference_ms
                            );
                            comparison.reason.as_ref().map_or(summary.clone(), |reason| {
                                format!("{summary} Comparison withheld: {reason}.")
                            })
                        })
                        .unwrap_or_else(|| {
                            let result = report.backends.first().unwrap();
                            format!(
                                "Benchmark complete. {} median {:.1}ms across {} measured run(s).",
                                result.execution_mode,
                                result.distribution.median_ms,
                                result.distribution.samples
                            )
                        });
                    if let Some(estimate) = &report.telemetry_overhead {
                        self.performance_summary.push_str(&format!(
                            " Detailed profiling cost estimate: {:+.1}ms / {:+.1}% ({})",
                            estimate.overhead_ms,
                            estimate.overhead_percent.unwrap_or(0.0),
                            estimate.classification
                        ));
                    }
                    self.refresh_performance_history();
                }
                Err(error) => {
                    self.performance_summary = format!("Benchmark failed: {error}");
                    self.performance_error = Some(error);
                }
            }
        }
        let regression_result =
            self.regression_job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("regression worker ended without a result".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some(result) = regression_result {
            if let Some(job) = self.regression_job.take() {
                let _ = job.worker.join();
            }
            match result {
                Ok(GuiRegressionOutcome::Updated(report)) => {
                    self.regression_summary = format!(
                        "Reference {} at {}. {} capture(s), {} checkpoint(s).",
                        report.fixture_id,
                        report.reference_directory.display(),
                        report.captured_modes,
                        report.captured_frames
                    );
                    if report.replaced_existing {
                        self.regression_summary.push_str(&format!(
                            " Observed changes: {}.",
                            if report.changed_layers.is_empty() {
                                "no layer differences".into()
                            } else {
                                report.changed_layers.join("; ")
                            }
                        ));
                    } else {
                        self.regression_summary.push_str(
                            " Review the stored PNGs before treating this first capture as known-good.",
                        );
                    }
                }
                Ok(GuiRegressionOutcome::Compared(report)) => {
                    self.regression_summary = format!(
                        "Regression {}: {} ({} compared frame(s)); current images: {}",
                        report.fixture_id,
                        if !report.comparable || !report.coverage_differences.is_empty() {
                            "NOT COMPARABLE"
                        } else if report.passed
                            && report
                                .vm_vs_gcc
                                .iter()
                                .any(|comparison| !comparison.comparable)
                        {
                            "REFERENCE PASS; VM/GCC PARITY INCOMPLETE"
                        } else if report.passed {
                            "PASS"
                        } else {
                            "FAIL"
                        },
                        report.frames.len(),
                        report.current_capture_directory.display()
                    );
                    self.regression_latest_report = Some(report);
                }
                Err(error) => {
                    self.regression_summary = format!("Regression lab failed: {error}");
                    self.regression_error = Some(error);
                }
            }
        }
        let mut completed = None;
        if let Some(job) = &self.engine_job {
            for message in job.result.try_iter() {
                match message {
                    EngineJobMessage::CheckingCache {
                        engine,
                        graph_nodes,
                        eligible_operations,
                    } if gcc_engine_message_applies(
                        &engine,
                        &self.config.engine,
                        self.config.execution_mode,
                    ) =>
                    {
                        self.gcc_status = GccUiStatus::CheckingCache {
                            graph_nodes,
                            eligible_operations,
                        };
                    }
                    EngineJobMessage::Compiling {
                        engine,
                        graph_nodes,
                        eligible_operations,
                    } if gcc_engine_message_applies(
                        &engine,
                        &self.config.engine,
                        self.config.execution_mode,
                    ) =>
                    {
                        self.gcc_status = GccUiStatus::Compiling {
                            graph_nodes,
                            eligible_operations,
                        };
                    }
                    EngineJobMessage::Complete { engine, result } => {
                        completed = Some((engine, result));
                    }
                    EngineJobMessage::Compiling { .. } => {}
                    EngineJobMessage::CheckingCache { .. } => {}
                }
            }
        }
        if let Some((engine, result)) = completed {
            let job = self.engine_job.take().unwrap();
            let _ = job.worker.join();
            if engine == self.config.engine {
                match result {
                    Ok(loaded) => {
                        if let Some(options) = loaded.options {
                            self.options = options;
                        }
                        self.loaded_watch = Some((engine.clone(), loaded.nodes));
                        if self.config.execution_mode
                            == crate::sono_gcc::WatchExecutionMode::SonoGcc
                        {
                            match loaded.gcc {
                                Some(Ok(report)) => self.set_gcc_report(report),
                                Some(Err(diagnostic)) => {
                                    self.gcc_status = GccUiStatus::Failed {
                                        graph_nodes: self
                                            .loaded_watch
                                            .as_ref()
                                            .map_or(0, |(_, nodes)| nodes.len()),
                                        diagnostic,
                                    };
                                }
                                None => self.gcc_status = GccUiStatus::Loading,
                            }
                        } else {
                            self.gcc_status = GccUiStatus::Disabled;
                        }
                    }
                    Err(diagnostic) => {
                        self.options.clear();
                        if self.config.execution_mode
                            == crate::sono_gcc::WatchExecutionMode::SonoGcc
                        {
                            self.gcc_status = GccUiStatus::Failed {
                                graph_nodes: 0,
                                diagnostic: diagnostic.clone(),
                            };
                        }
                        self.live
                            .lock()
                            .unwrap()
                            .log(format!("Engine/GCC preparation: {diagnostic}"));
                    }
                }
            }
        }

        if self.config.engine != self.options_engine {
            self.options_engine = self.config.engine.clone();
            self.options.clear();
            self.loaded_watch = None;
            self.gcc_status =
                if self.config.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
                    GccUiStatus::Loading
                } else {
                    GccUiStatus::Disabled
                };
            self.show_gcc_details = false;
        }

        let engine = self.config.engine.clone();
        let wants_gcc = should_precompile(self.config.execution_mode);
        if !wants_gcc {
            self.gcc_status = GccUiStatus::Disabled;
        }
        if engine.is_file() && self.engine_job.is_none() {
            if let Some((loaded_engine, nodes)) = &self.loaded_watch {
                if *loaded_engine == engine {
                    if wants_gcc
                        && matches!(
                            self.gcc_status,
                            GccUiStatus::Disabled | GccUiStatus::Loading
                        )
                    {
                        self.start_gcc_job(engine.clone(), nodes.clone());
                    }
                } else {
                    self.start_engine_job(engine.clone(), wants_gcc);
                }
            } else {
                self.start_engine_job(engine.clone(), wants_gcc);
            }
        }
    }
    fn media_acquisition_ui(&mut self, ui: &mut egui::Ui) {
        let busy = self.media_acquisition_job.is_some();
        let focus_url = std::mem::take(&mut self.media_acquisition_requested);
        let mut inspect = false;
        let mut download = false;
        let mut acquisition_header =
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id("media-acquisition-controls"),
                false,
            );
        if focus_url {
            acquisition_header.set_open(true);
        }
        acquisition_header
            .show_header(ui, |ui| ui.label("Acquire media from URL"))
            .body(|ui| {
            ui.label("For media you are authorized to download. One item only; playlists and live/upcoming sources are refused. Acquired files become ordinary Music/BGM and MV file inputs.");
            ui.label("The pinned yt-dlp executable is installed from its official release only after you inspect a URL. Local file import and rendering do not depend on yt-dlp.");
            let url_response = ui
                .add_enabled(!busy, egui::TextEdit::singleline(&mut self.media_acquisition_url).hint_text("https://…"))
                ;
            if focus_url {
                url_response.request_focus();
            }
            let changed = url_response.changed();
            if changed {
                self.media_acquisition_info = None;
                self.media_acquisition_inspected_url = None;
                self.media_acquisition_error = None;
            }
            ui.horizontal(|ui| {
                ui.label("Mode");
                egui::ComboBox::from_id_salt("media-acquisition-mode")
                    .selected_text(self.media_acquisition_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in [
                            crate::media_acquisition::AcquisitionMode::Audio,
                            crate::media_acquisition::AcquisitionMode::AudioMp3,
                            crate::media_acquisition::AcquisitionMode::Video,
                            crate::media_acquisition::AcquisitionMode::VideoAudio,
                        ] {
                            ui.selectable_value(&mut self.media_acquisition_mode, mode, mode.label());
                        }
                    });
            });
            let current_url = self.media_acquisition_url.trim();
            let inspected = self.media_acquisition_inspected_url.as_deref() == Some(current_url)
                && self.media_acquisition_info.is_some();
            ui.horizontal(|ui| {
                inspect = ui
                    .add_enabled(!busy && !current_url.is_empty(), egui::Button::new("Inspect URL"))
                    .clicked();
                download = ui
                    .add_enabled(!busy && inspected, egui::Button::new("Download and assign"))
                    .clicked();
                if busy && ui.button("Cancel acquisition").clicked() {
                    if let Some(job) = &self.media_acquisition_job {
                        job.control.cancel();
                    }
                }
            });
            if let Some(info) = &self.media_acquisition_info {
                ui.label(format!("Title: {}", info.title.as_deref().unwrap_or("unavailable")));
                ui.label(format!("Uploader: {}", info.uploader.as_deref().unwrap_or("unavailable")));
                ui.label(format!("Duration: {}", info.duration_seconds.map(|value| format!("{value:.2}s")).unwrap_or_else(|| "unavailable".into())));
                ui.label(format!("Known formats: {}", info.format_count.map(|value| value.to_string()).unwrap_or_else(|| "unavailable".into())));
            }
            let progress = self
                .media_acquisition_progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if busy || !progress.detail.is_empty() {
                ui.label(format!("{:?}: {}", progress.stage, progress.detail));
                if let Some(percent) = progress.percent {
                    ui.add(egui::ProgressBar::new(percent / 100.0).text(format!("{percent:.1}%")));
                }
                if let Some(speed) = progress.speed {
                    ui.small(format!("Speed: {speed}"));
                }
                if let Some(eta) = progress.eta {
                    ui.small(format!("ETA: {eta}"));
                }
            }
            if let Some(error) = &self.media_acquisition_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.small("Audio modes assign Music/BGM; video assigns MV; video + audio assigns both. Media is validated with FFprobe and saved to the project's media folder, or the renderer's per-user media folder when no saved project is open.");
            ui.small("Advanced source authentication/cookies, playlist batches, live capture, and Presentation Studio timeline editing are not supported.");
            });
        if inspect {
            self.inspect_media_url(ui.ctx());
        }
        if download {
            self.download_media_url(ui.ctx());
        }
    }

    fn presentation_new(&mut self) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error = Some(
                "Stop and finalize the microphone take before replacing this composition.".into(),
            );
            return;
        }
        self.presentation_studio = PresentationStudioView::default();
        self.presentation_studio.message =
            "New standalone composition. No engine or gameplay source is required.".into();
    }

    fn presentation_refresh_audio_devices(&mut self) {
        match crate::audio_input::enumerate_input_devices() {
            Ok(devices) => {
                self.presentation_studio.audio_devices = devices;
                self.presentation_studio.audio_devices_loaded = true;
                self.presentation_studio.audio_device_check = Instant::now();
                self.refresh_recoverable_recordings();
                let selected_missing = self
                    .presentation_studio
                    .audio_preferences
                    .selected_device_id
                    .as_deref()
                    .is_some_and(|selected| {
                        !self
                            .presentation_studio
                            .audio_devices
                            .iter()
                            .any(|device| device.id == selected)
                    });
                if selected_missing {
                    self.presentation_studio.audio_preferences_error = Some(
                        "The saved microphone is unavailable. Recording can use System Default, or reconnect the device and refresh.".into(),
                    );
                } else {
                    self.presentation_studio.audio_preferences_error = None;
                }
            }
            Err(error) => {
                self.presentation_studio.audio_devices_loaded = true;
                self.presentation_studio.audio_device_check = Instant::now();
                self.presentation_studio.audio_preferences_error =
                    Some(format!("Microphone inputs are unavailable: {error:#}"));
            }
        }
    }

    fn refresh_recoverable_recordings(&mut self) {
        let mut directories = vec![crate::audio_input::recording_directory(
            self.presentation_studio.project_path.as_deref(),
        )];
        if self.presentation_studio.project_path.is_some() {
            directories.push(crate::audio_input::recording_directory(None));
        }
        let mut takes = Vec::new();
        for directory in directories {
            if let Ok(found) = crate::audio_input::recoverable_recordings(&directory) {
                takes.extend(found);
            }
        }
        takes.sort_by(|a, b| a.partial_path.cmp(&b.partial_path));
        takes.dedup_by(|a, b| a.partial_path == b.partial_path);
        if let Some(active) = self
            .presentation_studio
            .audio_capture
            .as_ref()
            .and_then(|capture| capture.active_partial_path())
        {
            takes.retain(|take| take.partial_path != active);
        }
        self.presentation_studio.recoverable_takes = takes;
    }

    fn presentation_remember_undo(&mut self, previous: crate::composition::CompositionProject) {
        self.presentation_studio.edit_history.remember(previous);
        self.presentation_studio.dirty = true;
        self.presentation_studio.autosave_pending = true;
    }

    fn presentation_undo(&mut self) {
        if !self
            .presentation_studio
            .edit_history
            .undo(&mut self.presentation_studio.project)
        {
            return;
        }
        self.presentation_studio.selected_clip = None;
        self.presentation_studio.selected_clips.clear();
        self.presentation_studio.dirty = true;
        self.presentation_studio.autosave_pending = true;
        self.presentation_studio.message = "Undid the last composition edit.".into();
    }

    fn presentation_redo(&mut self) {
        if !self
            .presentation_studio
            .edit_history
            .redo(&mut self.presentation_studio.project)
        {
            return;
        }
        self.presentation_studio.selected_clip = None;
        self.presentation_studio.selected_clips.clear();
        self.presentation_studio.dirty = true;
        self.presentation_studio.autosave_pending = true;
        self.presentation_studio.message = "Redid the composition edit.".into();
    }

    fn save_workspace_preferences(&mut self) {
        if self.workspace_preferences.workflow == crate::workspace::WorkflowId::PresentationStudio {
            self.workspace_preferences.timeline_zoom = self
                .presentation_studio
                .timeline_zoom
                .round()
                .clamp(10.0, 180.0) as u16;
            self.workspace_preferences.timeline_track_height =
                self.presentation_studio
                    .timeline_track_height
                    .round()
                    .clamp(24.0, 120.0) as u16;
            self.workspace_preferences.capture_active_layout();
        }
        match self
            .workspace_preferences
            .save(&self.workspace_preferences_path)
        {
            Ok(()) => self.workspace_preferences_error = None,
            Err(error) => self.workspace_preferences_error = Some(format!("{error:#}")),
        }
    }

    fn activate_workspace_layout(&mut self, workspace: crate::workspace::WorkspaceId) {
        self.workspace_preferences.timeline_zoom = self
            .presentation_studio
            .timeline_zoom
            .round()
            .clamp(10.0, 180.0) as u16;
        self.workspace_preferences.timeline_track_height = self
            .presentation_studio
            .timeline_track_height
            .round()
            .clamp(24.0, 120.0) as u16;
        self.workspace_preferences.activate(workspace);
        self.presentation_studio.timeline_zoom = self.workspace_preferences.timeline_zoom as f32;
        self.presentation_studio.timeline_track_height =
            self.workspace_preferences.timeline_track_height as f32;
        self.save_workspace_preferences();
    }

    fn navigate_workflow(&mut self, workflow: crate::workspace::WorkflowId) {
        use crate::workspace::{WorkflowId as Workflow, WorkspaceId};
        if self.workspace_preferences.workflow == Workflow::PresentationStudio {
            self.workspace_preferences.timeline_zoom = self
                .presentation_studio
                .timeline_zoom
                .round()
                .clamp(10.0, 180.0) as u16;
            self.workspace_preferences.timeline_track_height =
                self.presentation_studio
                    .timeline_track_height
                    .round()
                    .clamp(24.0, 120.0) as u16;
            self.workspace_preferences.capture_active_layout();
        }
        if workflow == Workflow::Home {
            self.welcome_visible = true;
        } else if self.welcome_visible {
            self.finish_welcome();
        }
        self.workspace_preferences.workflow = workflow;
        self.quick_render_active = workflow == Workflow::QuickRender;
        self.welcome_visible = workflow == Workflow::Home;
        self.tab = match workflow {
            Workflow::Home | Workflow::QuickRender => 0,
            Workflow::ChartStudio => 14,
            Workflow::PresentationStudio => 15,
            Workflow::Production => 6,
            Workflow::Tools => 8,
            Workflow::Settings => 1,
            Workflow::Diagnostics => 12,
        };
        if workflow == Workflow::PresentationStudio
            && !matches!(
                self.workspace_preferences.active,
                WorkspaceId::Editing | WorkspaceId::Voiceover
            )
        {
            self.workspace_preferences.activate(WorkspaceId::Editing);
        }
        if workflow == Workflow::PresentationStudio {
            self.presentation_studio.timeline_zoom =
                self.workspace_preferences.timeline_zoom as f32;
            self.presentation_studio.timeline_track_height =
                self.workspace_preferences.timeline_track_height as f32;
        }
        self.save_workspace_preferences();
    }

    fn select_workflow_page(&mut self, workflow: crate::workspace::WorkflowId, tab: usize) {
        self.navigate_workflow(workflow);
        self.tab = tab;
        self.save_workspace_preferences();
    }

    fn command_disabled_reason(&self, command: &crate::workspace::CommandSpec) -> Option<String> {
        use crate::workspace::{CommandContext as Context, CommandId as Id};
        let in_presentation = !self.quick_render_active && self.tab == 15;
        let in_chart = !self.quick_render_active && self.tab == 14;
        if matches!(
            command.context,
            Context::Presentation
                | Context::PresentationSelection
                | Context::PresentationAudioSelection
        ) {
            if self.presentation_studio.audio_capture.is_some() && command.id != Id::StopRecording {
                return Some(
                    "Stop and finalize microphone capture before editing or exporting.".into(),
                );
            }
            if self.presentation_studio.job.is_some() && command.id != Id::StopRecording {
                return Some("Wait for the current composition task to finish.".into());
            }
            if matches!(
                command.id,
                Id::NewComposition
                    | Id::OpenComposition
                    | Id::SaveComposition
                    | Id::SaveCompositionAs
            ) && (self.presentation_studio.playback.is_some()
                || self.presentation_studio.playback_start.is_some())
            {
                return Some("Stop playback before replacing or saving the composition.".into());
            }
        }
        match command.context {
            Context::Global => None,
            Context::Presentation if !in_presentation => {
                Some("Open the Video Editing workspace first.".into())
            }
            Context::PresentationSelection if !in_presentation => {
                Some("Open the Video Editing workspace first.".into())
            }
            Context::PresentationAudioSelection if !in_presentation => {
                Some("Open the Video Editing workspace first.".into())
            }
            Context::PresentationSelection
                if self
                    .presentation_studio
                    .selected_clip
                    .as_ref()
                    .is_none_or(|id| {
                        !self
                            .presentation_studio
                            .project
                            .clips
                            .iter()
                            .any(|clip| &clip.id == id)
                    }) =>
            {
                Some("Select a timeline clip first.".into())
            }
            Context::PresentationSelection
                if self
                    .presentation_studio
                    .selected_clip
                    .as_deref()
                    .is_some_and(|id| {
                        let link = self
                            .presentation_studio
                            .project
                            .clips
                            .iter()
                            .find(|clip| clip.id == id)
                            .and_then(|clip| clip.link_group_id.as_deref());
                        self.presentation_studio
                            .project
                            .clips
                            .iter()
                            .filter(|clip| {
                                clip.id == id
                                    || link.is_some_and(|link_id| {
                                        clip.link_group_id.as_deref() == Some(link_id)
                                    })
                            })
                            .any(|clip| {
                                self.presentation_studio
                                    .project
                                    .track_for_clip(clip)
                                    .is_some_and(|track| track.locked)
                            })
                    }) =>
            {
                Some("The selected clip is on a locked track.".into())
            }
            Context::PresentationSelection if command.id == Id::SplitClip => {
                let Some(selected) =
                    self.presentation_studio
                        .selected_clip
                        .as_deref()
                        .and_then(|id| {
                            self.presentation_studio
                                .project
                                .clips
                                .iter()
                                .find(|clip| clip.id == id)
                        })
                else {
                    return Some("Select a timeline clip first.".into());
                };
                let link_id = selected.link_group_id.as_deref();
                let linked: Vec<_> = self
                    .presentation_studio
                    .project
                    .clips
                    .iter()
                    .filter(|clip| {
                        clip.id == selected.id
                            || link_id
                                .is_some_and(|link| clip.link_group_id.as_deref() == Some(link))
                    })
                    .collect();
                let frame = 1.0 / f64::from(self.presentation_studio.project.fps.max(1));
                if linked.iter().any(|clip| {
                    let within = self.presentation_studio.playhead - clip.timeline_start;
                    within < frame || within > clip.duration - frame
                }) {
                    Some("Move the playhead inside the selected clip, leaving at least one frame on each side.".into())
                } else if self
                    .presentation_studio
                    .project
                    .transitions
                    .iter()
                    .any(|transition| {
                        linked.iter().any(|clip| {
                            transition.from_clip_id == clip.id || transition.to_clip_id == clip.id
                        }) && transition.start_time < self.presentation_studio.playhead
                            && transition.start_time + transition.duration
                                > self.presentation_studio.playhead
                    })
                {
                    Some("The playhead is inside a transition. Move it outside or remove the transition before splitting.".into())
                } else {
                    None
                }
            }
            Context::PresentationAudioSelection => {
                let selected = self
                    .presentation_studio
                    .selected_clip
                    .as_deref()
                    .and_then(|id| {
                        self.presentation_studio
                            .project
                            .clips
                            .iter()
                            .find(|clip| clip.id == id)
                    });
                let has_audio = selected.is_some_and(|clip| {
                    clip.source_id.as_deref().is_some_and(|source_id| {
                        self.presentation_studio
                            .project
                            .sources
                            .iter()
                            .any(|source| source.id == source_id && source.has_audio)
                    }) && clip.source_time_mode == crate::composition::SourceTimeMode::Forward
                });
                if !has_audio {
                    Some(
                        "Select a forward-playing video or audio clip with an audio stream.".into(),
                    )
                } else if !crate::subtitles::setup_status().ready() {
                    Some("Install the local recognizer and multilingual model in Subtitles / Captions first.".into())
                } else if self.presentation_studio.audio_capture.is_some() {
                    Some("Stop microphone capture before starting transcription.".into())
                } else if self.presentation_studio.job.is_some() {
                    Some("Wait for the current composition task to finish.".into())
                } else {
                    None
                }
            }
            Context::Presentation if command.id == Id::StopRecording => self
                .presentation_studio
                .audio_capture
                .is_none()
                .then(|| "No microphone capture is active.".into()),
            Context::Presentation
                if self.presentation_studio.audio_capture.is_some()
                    && command.id != Id::StopRecording =>
            {
                Some("Stop and finalize microphone capture before editing or exporting.".into())
            }
            Context::Presentation if command.id == Id::RecordVoiceover => {
                if self.presentation_studio.audio_capture.is_some() {
                    Some("A microphone capture is already active; use Stop Recording.".into())
                } else if self.presentation_studio.job.is_some() {
                    Some("Wait for the current composition task to finish.".into())
                } else {
                    None
                }
            }
            Context::Presentation
                if command.id == Id::SaveComposition
                    && self.presentation_studio.audio_capture.is_some() =>
            {
                Some("Stop and finalize the microphone take before saving.".into())
            }
            Context::Presentation
                if command.id == Id::ExportComposition
                    && self.presentation_studio.audio_capture.is_some() =>
            {
                Some("Stop microphone capture before exporting.".into())
            }
            Context::Presentation
                if matches!(
                    command.id,
                    Id::NewComposition
                        | Id::OpenComposition
                        | Id::SaveComposition
                        | Id::SaveCompositionAs
                ) && (self.presentation_studio.playback.is_some()
                    || self.presentation_studio.playback_start.is_some()) =>
            {
                Some("Stop playback before replacing or saving the composition.".into())
            }
            Context::Presentation
                if self.presentation_studio.job.is_some()
                    && !matches!(command.id, Id::StopRecording) =>
            {
                Some("Wait for the current composition task to finish.".into())
            }
            Context::Chart if !in_chart => Some("Open Chart Studio first.".into()),
            Context::ChartWithMusic if !in_chart => Some("Open Chart Studio first.".into()),
            Context::ChartWithMusic if !self.chart_studio.music_path.is_file() => {
                Some("Choose chart audio or use the current project's Music / BGM input.".into())
            }
            Context::ChartWithMusic if self.chart_studio.tempo_job.is_some() => {
                Some("Tempo analysis is already running.".into())
            }
            Context::Render => {
                let queue_active = self.job_manager.as_ref().is_some_and(|manager| {
                    manager
                        .jobs()
                        .is_ok_and(|jobs| jobs.iter().any(|job| job.state.is_active()))
                });
                if self.job.is_some() || self.quick_validation_job.is_some() || queue_active {
                    Some("A render is already running.".into())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn dispatch_command(&mut self, id: crate::workspace::CommandId, ctx: &egui::Context) -> bool {
        use crate::workspace::{CommandId as Id, WorkflowId as Workflow, WorkspaceId};
        let Some(spec) = crate::workspace::COMMANDS
            .iter()
            .find(|command| command.id == id)
        else {
            return false;
        };
        if let Some(reason) = self.command_disabled_reason(spec) {
            self.summary = reason;
            self.error = false;
            return false;
        }
        if id != Id::ShowCommandPalette && id != Id::OpenShortcutSettings {
            self.command_palette_open = false;
        }
        match id {
            Id::ShowCommandPalette => {
                self.command_palette_open = true;
                self.command_palette_focus = true;
            }
            Id::OpenShortcutSettings => self.shortcut_editor_open = true,
            Id::GoHome => self.navigate_workflow(Workflow::Home),
            Id::GoQuickRender => self.navigate_workflow(Workflow::QuickRender),
            Id::GoChartStudio => self.navigate_workflow(Workflow::ChartStudio),
            Id::GoPresentationStudio => self.navigate_workflow(Workflow::PresentationStudio),
            Id::GoVoiceoverWorkspace => {
                if self.workspace_preferences.workflow != Workflow::PresentationStudio
                    || self.workspace_preferences.active != WorkspaceId::Voiceover
                {
                    self.navigate_workflow(Workflow::PresentationStudio);
                    self.activate_workspace_layout(WorkspaceId::Voiceover);
                }
            }
            Id::GoProduction => self.navigate_workflow(Workflow::Production),
            Id::GoRenderQueue => self.select_workflow_page(Workflow::Production, 6),
            Id::GoRenderHistory => self.select_workflow_page(Workflow::Production, 7),
            Id::GoTools => self.navigate_workflow(Workflow::Tools),
            Id::GoCacheManager => self.select_workflow_page(Workflow::Tools, 8),
            Id::GoPerformanceLab => self.select_workflow_page(Workflow::Tools, 9),
            Id::GoRegressionLab => self.select_workflow_page(Workflow::Tools, 10),
            Id::GoCompatibility => self.select_workflow_page(Workflow::Tools, 11),
            Id::GoProgramPreview => self.select_workflow_page(Workflow::Tools, 13),
            Id::GoSettings => self.navigate_workflow(Workflow::Settings),
            Id::GoConfigurationSettings => self.select_workflow_page(Workflow::Settings, 0),
            Id::GoEngineOptionsSettings => self.select_workflow_page(Workflow::Settings, 1),
            Id::GoRenderSettings => self.select_workflow_page(Workflow::Settings, 2),
            Id::GoGameplayUiSettings => self.select_workflow_page(Workflow::Settings, 3),
            Id::GoWatermarkSettings => self.select_workflow_page(Workflow::Settings, 4),
            Id::GoNotificationSettings => self.select_workflow_page(Workflow::Settings, 5),
            Id::GoAdvancedDiagnostics => self.navigate_workflow(Workflow::Diagnostics),
            Id::OpenMediaAcquisition => {
                self.navigate_workflow(Workflow::QuickRender);
                self.media_acquisition_requested = true;
            }
            Id::NewRenderProject => self.new_project(),
            Id::OpenRenderProject => self.pick_open_project(),
            Id::SaveRenderProject => self.save_project(),
            Id::SaveRenderProjectAs => self.pick_save_project(),
            Id::ResetWorkspace => {
                self.workspace_preferences.reset_active_layout();
                self.presentation_studio.timeline_zoom =
                    self.workspace_preferences.timeline_zoom as f32;
                self.presentation_studio.timeline_track_height =
                    self.workspace_preferences.timeline_track_height as f32;
                self.save_workspace_preferences();
            }
            Id::NewComposition => self.presentation_new(),
            Id::OpenComposition => self.presentation_open(),
            Id::SaveComposition => self.presentation_save(false),
            Id::SaveCompositionAs => self.presentation_save(true),
            Id::ExportComposition => self.start_presentation_export(ctx),
            Id::Undo => self.presentation_undo(),
            Id::Redo => self.presentation_redo(),
            Id::CopyClip => {
                self.presentation_studio.clip_clipboard = self
                    .presentation_studio
                    .selected_clip
                    .as_deref()
                    .and_then(|id| {
                        self.presentation_studio
                            .project
                            .clips
                            .iter()
                            .find(|clip| clip.id == id)
                            .cloned()
                    });
                self.presentation_studio.message =
                    if self.presentation_studio.clip_clipboard.is_some() {
                        "Copied the selected clip. Ctrl+V pastes another linked instance.".into()
                    } else {
                        "Select a clip before copying.".into()
                    };
            }
            Id::PasteClip => {
                if let Some(clip) = self.presentation_studio.clip_clipboard.clone() {
                    let before = self.presentation_studio.project.clone();
                    match self.presentation_studio.project.paste_clip(&clip) {
                        Ok(id) => {
                            self.presentation_studio.selected_clip = Some(id);
                            self.presentation_remember_undo(before);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
            }
            Id::DuplicateClip | Id::DeleteClip | Id::SplitClip | Id::RippleDeleteClip => {
                let Some(selected_id) = self.presentation_studio.selected_clip.clone() else {
                    return false;
                };
                let before = self.presentation_studio.project.clone();
                let operation = match id {
                    Id::DuplicateClip => self
                        .presentation_studio
                        .project
                        .duplicate_clip(&selected_id),
                    Id::DeleteClip => self
                        .presentation_studio
                        .project
                        .remove_clip(&selected_id)
                        .map(|_| String::new()),
                    Id::SplitClip => {
                        let is_caption = self
                            .presentation_studio
                            .project
                            .clips
                            .iter()
                            .find(|clip| clip.id == selected_id)
                            .is_some_and(|clip| clip.caption.is_some());
                        if is_caption {
                            crate::subtitles::split_caption_at(
                                &mut self.presentation_studio.project,
                                &selected_id,
                                self.presentation_studio.playhead,
                            )
                        } else {
                            self.presentation_studio
                                .project
                                .split_clip(&selected_id, self.presentation_studio.playhead)
                        }
                    }
                    Id::RippleDeleteClip => self
                        .presentation_studio
                        .project
                        .ripple_delete_clip(&selected_id)
                        .map(|_| String::new()),
                    _ => unreachable!(),
                };
                match operation {
                    Ok(new_id) => {
                        if matches!(id, Id::DeleteClip | Id::RippleDeleteClip) {
                            self.presentation_studio.selected_clip = None;
                        } else {
                            self.presentation_studio.selected_clip = Some(new_id);
                        }
                        self.presentation_remember_undo(before);
                        self.presentation_studio.error = None;
                    }
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
            Id::TogglePlayback => {
                let playing = self
                    .presentation_studio
                    .playback
                    .as_ref()
                    .is_some_and(|playback| playback.is_playing());
                if let Some(playback) = &self.presentation_studio.playback {
                    playback.set_playing(!playing);
                } else if self.presentation_studio.playback_start.is_none() {
                    self.start_presentation_playback(ctx, false);
                }
            }
            Id::PreviousFrame | Id::NextFrame => {
                if self.presentation_studio.audio_capture.is_none() {
                    let direction = if id == Id::PreviousFrame { -1.0 } else { 1.0 };
                    let target = self.presentation_studio.playhead
                        + direction / f64::from(self.presentation_studio.project.fps);
                    self.seek_presentation_playback(ctx, target);
                }
            }
            Id::ToggleTimelineSnapping => {
                self.presentation_studio.timeline_snapping =
                    !self.presentation_studio.timeline_snapping;
            }
            Id::AddKeyframe => {
                let Some(id) = self.presentation_studio.selected_clip.clone() else {
                    return false;
                };
                let property = self.presentation_studio.selected_animated_property;
                let playhead = self.presentation_studio.playhead;
                let previous = self.presentation_studio.project.clone();
                let result = self
                    .presentation_studio
                    .project
                    .clips
                    .iter_mut()
                    .find(|clip| clip.id == id)
                    .ok_or_else(|| anyhow::anyhow!("selected clip no longer exists"))
                    .and_then(|clip| {
                        let value = self
                            .pending_keyframe_value
                            .take()
                            .unwrap_or_else(|| clip.animated_value(property, playhead));
                        clip.add_keyframe(
                            property,
                            playhead,
                            value,
                            self.presentation_studio.pending_keyframe_easing,
                        )
                    });
                match result {
                    Ok(()) => self.presentation_remember_undo(previous),
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
            Id::GenerateSubtitles => {
                if let Some(clip_id) = self.presentation_studio.selected_clip.clone() {
                    self.start_presentation_transcription(ctx, clip_id);
                }
            }
            Id::RecordVoiceover => {
                let playing = self
                    .presentation_studio
                    .playback
                    .as_ref()
                    .is_some_and(|playback| !playback.is_ended());
                if playing {
                    self.presentation_start_audio(true);
                    if let Some(playback) = &self.presentation_studio.playback {
                        playback.set_playing(true);
                    }
                } else {
                    self.presentation_studio.record_voiceover_after_playback = true;
                    self.start_presentation_playback(ctx, true);
                }
            }
            Id::StopRecording => self.presentation_stop_audio(),
            Id::AnalyzeTempo => self.start_chart_tempo_analysis(ctx),
            Id::ExportLevelData => self.choose_chart_export(),
            Id::StartRender => self.start(ctx),
        }
        true
    }

    fn apply_saved_workspace(&mut self, name: &str) {
        self.workspace_preferences.timeline_zoom = self
            .presentation_studio
            .timeline_zoom
            .round()
            .clamp(10.0, 180.0) as u16;
        self.workspace_preferences.timeline_track_height = self
            .presentation_studio
            .timeline_track_height
            .round()
            .clamp(24.0, 120.0) as u16;
        self.workspace_preferences.capture_active_layout();
        if !self.workspace_preferences.apply_saved(name) {
            self.workspace_preferences_error =
                Some(format!("Saved workspace {name:?} no longer exists."));
            return;
        }
        self.presentation_studio.timeline_zoom = self.workspace_preferences.timeline_zoom as f32;
        self.presentation_studio.timeline_track_height =
            self.workspace_preferences.timeline_track_height as f32;
        self.save_workspace_preferences();
    }

    fn shortcut_label(&self, command: &crate::workspace::CommandSpec) -> String {
        let bindings = self.workspace_preferences.bindings_for(command);
        if bindings.is_empty() {
            "Unbound".into()
        } else {
            bindings
                .into_iter()
                .map(|binding| binding.label())
                .collect::<Vec<_>>()
                .join(" / ")
        }
    }

    fn command_for_shortcut_event(
        &self,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> Option<crate::workspace::CommandId> {
        let key_name = format!("{key:?}");
        crate::workspace::COMMANDS.iter().find_map(|command| {
            let matches = self
                .workspace_preferences
                .bindings_for(command)
                .into_iter()
                .any(|binding| {
                    binding.key == key_name
                        && binding.primary == modifiers.command
                        && binding.shift == modifiers.shift
                        && binding.alt == modifiers.alt
                });
            (matches && self.command_disabled_reason(command).is_none()).then_some(command.id)
        })
    }

    fn poll_shortcut_capture(&mut self, ctx: &egui::Context) {
        let Some(command_id) = self.shortcut_capture else {
            return;
        };
        let event = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = event else {
            return;
        };
        if key == egui::Key::Escape && !modifiers.command && !modifiers.alt && !modifiers.shift {
            self.shortcut_capture = None;
            return;
        }
        let mut parts = Vec::new();
        if modifiers.command {
            parts.push("Ctrl");
        }
        if modifiers.shift {
            parts.push("Shift");
        }
        if modifiers.alt {
            parts.push("Alt");
        }
        let key_text = format!("{key:?}");
        parts.push(&key_text);
        let binding = match crate::workspace::ShortcutBinding::parse(&parts.join("+")) {
            Ok(binding) => binding,
            Err(error) => {
                self.workspace_preferences_error =
                    Some(format!("Shortcut was not assigned: {error}"));
                return;
            }
        };
        let Some(spec) = crate::workspace::COMMANDS
            .iter()
            .find(|command| command.id == command_id)
        else {
            self.shortcut_capture = None;
            return;
        };
        let conflicts = self
            .workspace_preferences
            .conflicts(spec.stable_id, &binding);
        if !conflicts.is_empty() {
            self.shortcut_conflict = Some((
                spec.stable_id.into(),
                binding,
                conflicts.into_iter().map(str::to_owned).collect(),
            ));
            self.shortcut_capture = None;
            return;
        }
        match self
            .workspace_preferences
            .assign_binding(spec.stable_id, binding, false)
        {
            Ok(_) => {
                self.shortcut_capture = None;
                self.save_workspace_preferences();
            }
            Err(error) => {
                self.workspace_preferences_error =
                    Some(format!("Could not assign shortcut: {error:#}"))
            }
        }
    }

    fn command_menu_item(&mut self, ui: &mut egui::Ui, id: crate::workspace::CommandId) {
        let Some(command) = crate::workspace::COMMANDS
            .iter()
            .find(|command| command.id == id)
        else {
            return;
        };
        let shortcut = self.shortcut_label(command);
        let disabled = self.command_disabled_reason(command);
        let label = if shortcut == "Unbound" {
            command.name.to_owned()
        } else {
            format!("{}\t{shortcut}", command.name)
        };
        let response = ui.add_enabled(disabled.is_none(), egui::Button::new(label));
        let response = if let Some(reason) = disabled {
            response.on_hover_text(reason)
        } else {
            response.on_hover_text(command.description)
        };
        if response.clicked() {
            self.dispatch_command(id, ui.ctx());
        }
    }

    fn global_toolbar(&mut self, ui: &mut egui::Ui) {
        use crate::workspace::{CommandId as Id, WorkflowId as Workflow};
        let workflow = self.workspace_preferences.workflow;
        let locale = self.workspace_preferences.locale.clone();
        ui.horizontal_wrapped(|ui| {
            ui.heading(crate::localization::tr(&locale, "app.name"));
            ui.weak(workflow.localized_label(&locale));
            for (key, command) in [
                ("nav.home", Id::GoHome),
                ("nav.quick_render", Id::GoQuickRender),
                ("nav.chart_studio", Id::GoChartStudio),
                ("nav.presentation_studio", Id::GoPresentationStudio),
                ("nav.production", Id::GoProduction),
            ] {
                let label = crate::workspace::COMMANDS
                    .iter()
                    .find(|item| item.id == command)
                    .map(|command| crate::workspace::localized_command_name(command, &locale))
                    .unwrap_or_else(|| crate::localization::tr(&locale, key));
                if ui
                    .selectable_label(workflow == workflow_for_command(command), label)
                    .clicked()
                {
                    self.dispatch_command(command, ui.ctx());
                }
            }
            ui.menu_button("More", |ui| {
                for (label, command) in [
                    ("Tools", Id::GoTools),
                    ("Settings", Id::GoSettings),
                    ("Diagnostics", Id::GoAdvancedDiagnostics),
                ] {
                    if ui
                        .selectable_label(workflow == workflow_for_command(command), label)
                        .clicked()
                    {
                        self.dispatch_command(command, ui.ctx());
                        ui.close();
                    }
                }
                ui.separator();
                ui.menu_button("Admin Console", |ui| {
                    if let Some(server) = self.admin_server.as_ref() {
                        let url = server.url();
                        ui.hyperlink_to("Open running console", &url);
                        ui.weak(format!("Listening at {url}"));
                    } else if let Some(error) = self.admin_server_error.as_deref() {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            format!("Service unavailable: {error}"),
                        );
                    } else {
                        ui.weak("Service is stopped.");
                    }
                    ui.separator();
                    ui.checkbox(
                        &mut self.admin_config.enabled,
                        "Enable on application start",
                    );
                    ui.label("Bind address (literal IP)");
                    ui.text_edit_singleline(&mut self.admin_config.bind_address);
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        ui.add(egui::DragValue::new(&mut self.admin_config.port));
                        ui.weak("0 = choose an available port");
                    });
                    if let Some(message) = self.admin_settings_message.as_deref() {
                        ui.label(message);
                    }
                    if ui.button("Save; restart to apply").clicked() {
                        let result = self
                            .admin_config
                            .validate()
                            .and_then(|_| {
                                crate::admin::validate_startup_security(&self.admin_config)
                            })
                            .and_then(|_| self.admin_config.save_default());
                        self.admin_settings_message = Some(match result {
                            Ok(()) => {
                                "Saved. Restart Sono-Renderer to apply the server setting.".into()
                            }
                            Err(error) => format!("Settings not saved: {error:#}"),
                        });
                    }
                });
                ui.separator();
                ui.menu_button("Open a tool", |ui| {
                    for (label, command) in [
                        ("Tempo Analysis (Chart Studio)", Id::GoChartStudio),
                        ("Program Preview", Id::GoProgramPreview),
                        ("Compatibility", Id::GoCompatibility),
                        ("Cache Manager", Id::GoCacheManager),
                        ("Performance Lab", Id::GoPerformanceLab),
                        ("Regression Lab", Id::GoRegressionLab),
                    ] {
                        if ui.button(label).clicked() {
                            self.dispatch_command(command, ui.ctx());
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if ui.button("Home / Getting Started").clicked() {
                    self.dispatch_command(Id::GoHome, ui.ctx());
                    ui.close();
                }
                if ui.button("Help and workflows").clicked() {
                    self.help_open = true;
                    ui.close();
                }
                if let Some(server) = self.admin_server.as_ref() {
                    let url = server.url();
                    ui.hyperlink_to("Open Admin Console", &url);
                } else if let Some(error) = self.admin_server_error.as_deref() {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Admin service unavailable: {error}"),
                    );
                }
                if ui.button("Command Palette…").clicked() {
                    self.dispatch_command(Id::ShowCommandPalette, ui.ctx());
                    ui.close();
                }
                if ui.button("Keyboard Shortcuts…").clicked() {
                    self.dispatch_command(Id::OpenShortcutSettings, ui.ctx());
                    ui.close();
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.menu_button("File", |ui| match workflow {
                Workflow::Home => {
                    self.command_menu_item(ui, Id::OpenRenderProject);
                    if ui.button("Open Video Composition…").clicked() {
                        self.dispatch_command(Id::GoPresentationStudio, ui.ctx());
                        self.dispatch_command(Id::OpenComposition, ui.ctx());
                        ui.close();
                    }
                }
                Workflow::QuickRender => {
                    self.command_menu_item(ui, Id::OpenRenderProject);
                    self.command_menu_item(ui, Id::SaveRenderProject);
                    self.command_menu_item(ui, Id::SaveRenderProjectAs);
                    ui.separator();
                    self.command_menu_item(ui, Id::OpenMediaAcquisition);
                    self.command_menu_item(ui, Id::StartRender);
                }
                Workflow::ChartStudio => {
                    self.command_menu_item(ui, Id::OpenRenderProject);
                    self.command_menu_item(ui, Id::SaveRenderProject);
                    self.command_menu_item(ui, Id::ExportLevelData);
                }
                Workflow::PresentationStudio => {
                    self.command_menu_item(ui, Id::NewComposition);
                    self.command_menu_item(ui, Id::OpenComposition);
                    self.command_menu_item(ui, Id::SaveComposition);
                    self.command_menu_item(ui, Id::SaveCompositionAs);
                    self.command_menu_item(ui, Id::ExportComposition);
                }
                _ => {
                    self.command_menu_item(ui, Id::OpenRenderProject);
                    if ui.button("Open Video Composition…").clicked() {
                        self.dispatch_command(Id::GoPresentationStudio, ui.ctx());
                        self.dispatch_command(Id::OpenComposition, ui.ctx());
                        ui.close();
                    }
                }
            });
            if workflow == Workflow::PresentationStudio {
                ui.menu_button("Edit", |ui| {
                    self.command_menu_item(ui, Id::Undo);
                    self.command_menu_item(ui, Id::Redo);
                    ui.separator();
                    self.command_menu_item(ui, Id::CopyClip);
                    self.command_menu_item(ui, Id::PasteClip);
                    self.command_menu_item(ui, Id::DuplicateClip);
                    self.command_menu_item(ui, Id::DeleteClip);
                });
                ui.menu_button("Timeline", |ui| {
                    self.command_menu_item(ui, Id::SplitClip);
                    self.command_menu_item(ui, Id::RippleDeleteClip);
                    self.command_menu_item(ui, Id::AddKeyframe);
                    self.command_menu_item(ui, Id::ToggleTimelineSnapping);
                    self.command_menu_item(ui, Id::TogglePlayback);
                    self.command_menu_item(ui, Id::PreviousFrame);
                    self.command_menu_item(ui, Id::NextFrame);
                    ui.separator();
                    self.command_menu_item(ui, Id::GenerateSubtitles);
                    self.command_menu_item(ui, Id::RecordVoiceover);
                    self.command_menu_item(ui, Id::StopRecording);
                });
                self.presentation_layout_menu(ui);
            }
        });
        if let Some(error) = &self.workspace_preferences_error {
            ui.colored_label(egui::Color32::YELLOW, error);
        }
    }

    fn presentation_layout_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Layout", |ui| {
            ui.weak("A layout changes panel visibility and timeline scale; it does not change the project or workflow.");
            for workspace in [
                crate::workspace::WorkspaceId::Editing,
                crate::workspace::WorkspaceId::Voiceover,
            ] {
                let selected = self.workspace_preferences.active == workspace;
                if ui.selectable_label(selected, workspace.label()).clicked() {
                    self.activate_workspace_layout(workspace);
                    ui.close();
                }
            }
            let saved = self.workspace_preferences.saved.iter().map(|layout| layout.name.clone()).collect::<Vec<_>>();
            if !saved.is_empty() {
                ui.separator();
                ui.label("Saved layouts");
                for name in saved {
                    if ui.button(&name).clicked() {
                        self.apply_saved_workspace(&name);
                        ui.close();
                    }
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.workspace_name_input)
                    .desired_width(140.0).hint_text("Layout name"));
                if ui.button("Save Layout As…").clicked() {
                    self.workspace_preferences.timeline_zoom = self.presentation_studio.timeline_zoom.round().clamp(10.0, 180.0) as u16;
                    self.workspace_preferences.timeline_track_height = self.presentation_studio.timeline_track_height.round().clamp(24.0, 120.0) as u16;
                    match self.workspace_preferences.save_custom(&self.workspace_name_input) {
                        Ok(()) => {
                            self.workspace_name_input.clear();
                            self.save_workspace_preferences();
                            ui.close();
                        }
                        Err(error) => self.workspace_preferences_error = Some(format!("Could not save layout: {error:#}")),
                    }
                }
            });
            ui.separator();
            let panels = &mut self.workspace_preferences.panels;
            let mut changed = false;
            changed |= ui.checkbox(&mut panels.composition, "Composition settings").changed();
            changed |= ui.checkbox(&mut panels.tracks, "Tracks").changed();
            changed |= ui.checkbox(&mut panels.media, "Media sources").changed();
            changed |= ui.checkbox(&mut panels.source_monitor, "Source monitor").changed();
            changed |= ui.checkbox(&mut panels.timeline, "Timeline").changed();
            changed |= ui.checkbox(&mut panels.inspector, "Clip inspector").changed();
            changed |= ui.checkbox(&mut panels.captions, "Subtitles / captions").changed();
            changed |= ui.checkbox(&mut panels.voiceover, "Voiceover recording").changed();
            ui.separator();
            changed |= ui.add(egui::Slider::new(&mut self.presentation_studio.timeline_zoom, 10.0..=180.0).text("Timeline zoom")).changed();
            changed |= ui.add(egui::Slider::new(&mut self.presentation_studio.timeline_track_height, 24.0..=120.0).text("Track height")).changed();
            self.command_menu_item(ui, crate::workspace::CommandId::ResetWorkspace);
            if changed {
                self.save_workspace_preferences();
            }
        });
    }

    fn command_palette_ui(&mut self, ctx: &egui::Context) {
        if !self.command_palette_open {
            return;
        }
        let mut open = self.command_palette_open;
        let mut selected = None;
        let locale = self.workspace_preferences.locale.clone();
        egui::Window::new("Command Palette")
            .id(egui::Id::new("application-command-palette"))
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut self.command_palette_query).hint_text("Search commands: split, bpm, subtitle, voiceover, render…").desired_width(f32::INFINITY));
                    if self.command_palette_focus { response.request_focus(); self.command_palette_focus = false; }
                    if ui.small_button("Clear").clicked() { self.command_palette_query.clear(); }
                });
                let results = crate::workspace::search_commands_localized(
                    &self.command_palette_query,
                    &locale,
                );
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                let mut first_enabled = None;
                egui::ScrollArea::vertical().max_height(410.0).show(ui, |ui| {
                    if results.is_empty() { ui.weak("No matching commands."); }
                    for command in results {
                        let disabled = self.command_disabled_reason(command);
                        let command_name = crate::workspace::localized_command_name(command, &locale);
                        if disabled.is_none() && first_enabled.is_none() { first_enabled = Some(command.id); }
                        ui.horizontal(|ui| {
                            let response = ui.add_enabled(disabled.is_none(), egui::Button::new(command_name).min_size(egui::vec2(255.0, 26.0)));
                            let response = if let Some(reason) = &disabled { response.on_hover_text(reason) } else { response.on_hover_text(command.description) };
                            if response.clicked() { selected = Some(command.id); }
                            ui.small(command.category);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.small(self.shortcut_label(command)); });
                        });
                        if let Some(reason) = disabled { ui.small(format!("  {reason}")); }
                    }
                });
                if enter { selected = first_enabled; }
                ui.separator();
                ui.weak("Commands run in the current workspace. Disabled actions explain what they need.");
            });
        self.command_palette_open = open;
        if let Some(command) = selected {
            self.dispatch_command(command, ctx);
        }
    }

    fn shortcut_editor_ui(&mut self, ctx: &egui::Context) {
        if !self.shortcut_editor_open {
            return;
        }
        let mut open = self.shortcut_editor_open;
        let mut reset_all = false;
        egui::Window::new("Keyboard Shortcuts")
            .id(egui::Id::new("keyboard-shortcut-editor"))
            .default_width(680.0)
            .default_height(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Bindings use the platform primary modifier (Ctrl on Windows/Linux, Command on macOS). Single-letter bindings require a modifier so typing in editors and search fields stays safe.");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.shortcut_search).hint_text("Search commands, categories, keywords").desired_width(340.0));
                    if ui.button("Restore All Defaults").clicked() { reset_all = true; }
                });
                if let Some((command_id, binding, conflicts)) = self.shortcut_conflict.clone() {
                    let command_name = crate::workspace::command_by_stable_id(&command_id).map_or(command_id.as_str(), |command| command.name);
                    let conflicting_names = conflicts.iter().map(|id| crate::workspace::command_by_stable_id(id).map_or(id.as_str(), |command| command.name)).collect::<Vec<_>>().join(", ");
                    ui.colored_label(egui::Color32::YELLOW, format!("{} conflicts with {}.", binding.label(), conflicting_names));
                    ui.horizontal(|ui| {
                        if ui.button(format!("Replace existing binding(s) and assign to {command_name}")).clicked() {
                            if let Err(error) = self.workspace_preferences.assign_binding(&command_id, binding, true) {
                                self.workspace_preferences_error = Some(format!("Could not replace shortcut: {error:#}"));
                            } else {
                                self.shortcut_conflict = None;
                                self.save_workspace_preferences();
                            }
                        }
                        if ui.button("Cancel").clicked() { self.shortcut_conflict = None; }
                    });
                }
                if let Some(error) = &self.workspace_preferences_error { ui.colored_label(egui::Color32::YELLOW, error); }
                if let Some(command) = self.shortcut_capture {
                    let name = crate::workspace::COMMANDS.iter().find(|item| item.id == command).map_or("command", |item| item.name);
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("Press a shortcut for {name}. Escape cancels. A conflicting binding will ask before replacement."));
                }
                ui.separator();
                let query = self.shortcut_search.trim().to_owned();
                egui::ScrollArea::vertical().max_height(410.0).show(ui, |ui| {
                    for command in crate::workspace::search_commands(&query) {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(command.name);
                                ui.small(format!("{}  ·  {}", command.category, command.description));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Reset").clicked() {
                                    self.workspace_preferences.shortcut_overrides.remove(command.stable_id);
                                    self.save_workspace_preferences();
                                }
                                if ui.button("Clear").clicked() {
                                    self.workspace_preferences.shortcut_overrides.insert(command.stable_id.into(), crate::workspace::ShortcutBinding::DISABLED);
                                    self.save_workspace_preferences();
                                }
                                if ui.button(if self.shortcut_capture == Some(command.id) { "Waiting…" } else { "Bind…" }).clicked() {
                                    self.shortcut_capture = Some(command.id);
                                    self.shortcut_conflict = None;
                                }
                                ui.small(self.shortcut_label(command));
                            });
                        });
                        ui.separator();
                    }
                });
            });
        self.shortcut_editor_open = open;
        if !open {
            self.shortcut_capture = None;
            self.shortcut_conflict = None;
        }
        if reset_all {
            self.workspace_preferences.reset_shortcuts();
            self.save_workspace_preferences();
        }
    }

    fn presentation_refresh_recovery(&mut self) {
        match crate::composition::list_recovery_snapshots() {
            Ok(snapshots) => self.presentation_studio.recovery_snapshots = snapshots,
            Err(error) => {
                self.presentation_studio.error = Some(format!(
                    "Could not inspect editor recovery checkpoints: {error:#}"
                ));
            }
        }
        self.presentation_studio.recovery_checked = true;
    }

    fn presentation_autosave(&mut self, ctx: &egui::Context) {
        if !self.presentation_studio.autosave_pending
            || self.presentation_studio.last_autosave.elapsed() < Duration::from_secs(20)
            || self.presentation_studio.audio_capture.is_some()
            || self.presentation_studio.job.is_some()
        {
            if self.presentation_studio.autosave_pending {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
            return;
        }
        self.presentation_studio.last_autosave = Instant::now();
        match crate::composition::save_recovery_snapshot(
            &self.presentation_studio.project,
            self.presentation_studio.project_path.as_deref(),
        ) {
            Ok(_) => {
                self.presentation_studio.autosave_pending = false;
                self.presentation_studio.message =
                    "Recovery checkpoint saved. Use Save to update the project file.".into();
                self.presentation_studio.recovery_checked = false;
            }
            Err(error) => {
                self.presentation_studio.error = Some(format!(
                    "Could not save editor recovery checkpoint: {error:#}"
                ));
                ctx.request_repaint_after(Duration::from_secs(10));
            }
        }
    }

    fn presentation_close_prompt_ui(&mut self, ctx: &egui::Context) {
        if !self.presentation_close_prompt {
            return;
        }
        egui::Window::new("Unsaved Presentation Studio changes")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Save or checkpoint your composition before closing?");
                ui.horizontal(|ui| {
                    if ui.button("Keep Editing").clicked() {
                        self.presentation_close_prompt = false;
                        self.presentation_close_checkpointed = false;
                    }
                    if ui.button("Save Project and Close").clicked() {
                        self.presentation_save(false);
                        if !self.presentation_studio.dirty {
                            self.presentation_close_prompt = false;
                            self.presentation_close_checkpointed = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui.button("Save Recovery Checkpoint and Close").clicked() {
                        match crate::composition::save_recovery_snapshot(
                            &self.presentation_studio.project,
                            self.presentation_studio.project_path.as_deref(),
                        ) {
                            Ok(_) => {
                                self.presentation_studio.autosave_pending = false;
                                self.presentation_studio.recovery_checked = false;
                                self.presentation_studio.message =
                                    "Recovery checkpoint saved. Reopen Presentation Studio and choose Recover to restore these edits.".into();
                                self.presentation_close_prompt = false;
                                self.presentation_close_checkpointed = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!(
                                    "Could not checkpoint the unsaved composition; it remains open: {error:#}"
                                ));
                            }
                        }
                    }
                    if ui.button("Discard and Close").clicked() {
                        match crate::composition::remove_recovery_snapshot(
                            self.presentation_studio.project_path.as_deref(),
                        ) {
                            Ok(()) => {
                                self.presentation_studio.dirty = false;
                                self.presentation_studio.autosave_pending = false;
                                self.presentation_close_prompt = false;
                                self.presentation_close_checkpointed = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!(
                                    "Could not remove the stale recovery checkpoint; the composition remains open: {error:#}"
                                ));
                            }
                        }
                    }
                });
                if let Some(error) = &self.presentation_studio.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            });
    }

    fn presentation_start_audio(&mut self, record: bool) {
        if self.presentation_studio.audio_capture.is_some() {
            return;
        }
        let timeline_start = self
            .presentation_studio
            .playhead
            .clamp(0.0, self.presentation_studio.project.duration);
        let path = if record {
            let directory = crate::audio_input::recording_directory(
                self.presentation_studio.project_path.as_deref(),
            );
            match crate::audio_input::new_take_path(&directory) {
                Ok(path) => Some(path),
                Err(error) => {
                    self.presentation_studio.error = Some(format!("{error:#}"));
                    return;
                }
            }
        } else {
            None
        };
        match crate::audio_input::CaptureSession::start(
            self.presentation_studio
                .audio_preferences
                .selected_device_id
                .as_deref(),
            path.as_deref(),
            timeline_start,
        ) {
            Ok(capture) => {
                let selected_name = self
                    .presentation_studio
                    .audio_devices
                    .iter()
                    .find(|device| device.id == capture.device_id())
                    .map(|device| device.name.as_str())
                    .unwrap_or("system input");
                self.presentation_studio.message = if record {
                    format!("Recording voiceover from {selected_name} at {timeline_start:.3}s. Click Stop to finalize the WAV take.")
                } else {
                    format!("Input meter active for {selected_name}. No audio is being saved; click Stop Meter to close the input.")
                };
                self.presentation_studio.audio_capture = Some(capture);
                self.presentation_studio.error = None;
            }
            Err(error) => {
                self.presentation_studio.error = Some(format!("{error:#}"));
                self.presentation_studio.message = "Microphone capture did not start.".into();
            }
        }
    }

    fn presentation_stop_audio(&mut self) {
        let Some(capture) = self.presentation_studio.audio_capture.take() else {
            return;
        };
        let was_recording = capture.is_recording();
        match capture.stop() {
            Ok(Some(recording)) => {
                let previous = self.presentation_studio.project.clone();
                let timeline_start = recording.timeline_start_seconds;
                match self
                    .presentation_studio
                    .project
                    .add_recorded_source(&recording.path)
                {
                    Ok(source_id) => match self.presentation_studio.project.add_clip(
                        &source_id,
                        crate::composition::ClipKind::Audio,
                        timeline_start,
                    ) {
                        Ok(clip_id) => {
                            self.presentation_studio.selected_source = Some(source_id);
                            self.presentation_studio.selected_clip = Some(clip_id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.message = format!(
                                "Voiceover finalized as {} ({} Hz, {} channel{}, {:.2}s) and inserted at {timeline_start:.3}s with a waveform summary.",
                                recording.path.file_name().and_then(|name| name.to_str()).unwrap_or("recording.wav"),
                                recording.sample_rate,
                                recording.channels,
                                if recording.channels == 1 { "" } else { "s" },
                                recording.frames as f64 / f64::from(recording.sample_rate),
                            );
                            if let Some(warning) = recording.warning {
                                self.presentation_studio.error = Some(warning);
                            } else {
                                self.presentation_studio.error = None;
                            }
                        }
                        Err(error) => {
                            self.presentation_studio.error = Some(format!(
                                "The WAV was safely saved at {}, but could not be inserted at this playhead: {error:#}",
                                recording.path.display()
                            ));
                        }
                    },
                    Err(error) => {
                        self.presentation_studio.error = Some(format!(
                            "The WAV was safely saved at {}, but could not be imported: {error:#}",
                            recording.path.display()
                        ));
                    }
                }
                self.refresh_recoverable_recordings();
            }
            Ok(None) => {
                self.presentation_studio.message =
                    "Input meter stopped. No samples were saved.".into();
                self.presentation_studio.error = None;
            }
            Err(error) => {
                self.presentation_studio.error = Some(format!("{error:#}"));
                self.presentation_studio.message = if was_recording {
                    "The recording could not be finalized. Any checkpointed partial WAV remains available for recovery.".into()
                } else {
                    "Input meter stopped.".into()
                };
                self.refresh_recoverable_recordings();
            }
        }
    }

    fn presentation_recover_take(&mut self, index: usize) {
        let Some(take) = self
            .presentation_studio
            .recoverable_takes
            .get(index)
            .cloned()
        else {
            return;
        };
        match crate::audio_input::recover_partial(&take) {
            Ok(recording) => {
                let previous = self.presentation_studio.project.clone();
                match self
                    .presentation_studio
                    .project
                    .add_recorded_source(&recording.path)
                {
                    Ok(source_id) => match self.presentation_studio.project.add_clip(
                        &source_id,
                        crate::composition::ClipKind::Audio,
                        recording.timeline_start_seconds,
                    ) {
                        Ok(clip_id) => {
                            self.presentation_studio.selected_source = Some(source_id);
                            self.presentation_studio.selected_clip = Some(clip_id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.message = format!(
                                "Recovered partial voiceover {} and inserted it at {:.3}s. Review its final boundary.",
                                recording.path.display(), recording.timeline_start_seconds
                            );
                            self.presentation_studio.error = None;
                        }
                        Err(error) => {
                            self.presentation_studio.error = Some(format!(
                                "Recovered WAV remains at {}, but could not be placed on the timeline: {error:#}",
                                recording.path.display()
                            ));
                        }
                    },
                    Err(error) => {
                        self.presentation_studio.error = Some(format!(
                            "Recovered WAV remains at {}, but could not be imported: {error:#}",
                            recording.path.display()
                        ));
                    }
                }
            }
            Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
        }
        self.refresh_recoverable_recordings();
    }

    fn presentation_open(&mut self) {
        if self.presentation_studio.audio_capture.is_some()
            || self.presentation_studio.playback.is_some()
            || self.presentation_studio.playback_start.is_some()
        {
            return;
        }
        let dialog = rfd::FileDialog::new()
            .set_title("Open Composition")
            .add_filter("Sono-Renderer composition", &["sonocomp", "json"]);
        if let Some(path) = dialog.pick_file() {
            self.presentation_load_project(path);
        }
    }

    fn presentation_load_project(&mut self, path: PathBuf) {
        match crate::composition::CompositionProject::load(&path) {
            Ok(project) => {
                let output = path.with_extension("mp4");
                self.presentation_studio = PresentationStudioView {
                    project,
                    project_path: Some(path),
                    selected_source: None,
                    selected_clip: None,
                    output,
                    message: "Composition opened. Media remains local and works offline.".into(),
                    ..PresentationStudioView::default()
                };
            }
            Err(error) => {
                self.presentation_studio.error =
                    Some(format!("Could not open composition: {error:#}"));
            }
        }
    }

    fn presentation_save(&mut self, save_as: bool) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop and finalize the microphone take before saving the composition.".into());
            return;
        }
        let path = if !save_as {
            self.presentation_studio.project_path.clone()
        } else {
            None
        };
        let path = path.or_else(|| {
            rfd::FileDialog::new()
                .set_title("Save Composition")
                .add_filter("Sono-Renderer composition", &["sonocomp"])
                .set_file_name("composition.sonocomp")
                .save_file()
        });
        let Some(path) = path else { return };
        let previous_path = self.presentation_studio.project_path.clone();
        match self.presentation_studio.project.save(&path) {
            Ok(()) => {
                self.presentation_studio.project_path = Some(path.clone());
                self.presentation_studio.dirty = false;
                self.presentation_studio.autosave_pending = false;
                self.presentation_close_checkpointed = false;
                self.presentation_studio.error = None;
                if let Err(error) =
                    crate::composition::remove_recovery_snapshot(previous_path.as_deref())
                {
                    self.presentation_studio.error = Some(format!(
                        "Project saved, but its old recovery checkpoint could not be removed: {error:#}"
                    ));
                }
                if let Err(error) =
                    crate::composition::remove_recovery_snapshot(Some(path.as_path()))
                {
                    self.presentation_studio.error = Some(format!(
                        "Project saved, but an older checkpoint for this path could not be removed: {error:#}"
                    ));
                }
                self.presentation_studio.recovery_snapshots.clear();
                self.presentation_studio.recovery_checked = false;
                self.presentation_studio.message = format!("Saved {}", path.display());
            }
            Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
        }
    }

    fn presentation_import_media(&mut self, expected: crate::composition::ClipKind) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop the microphone take before changing composition media.".into());
            return;
        }
        let (title, filter, extensions) = match expected {
            crate::composition::ClipKind::Video => (
                "Import Video or Image",
                "Video / image",
                vec![
                    "mp4", "mov", "mkv", "webm", "avi", "m4v", "png", "jpg", "jpeg",
                ],
            ),
            crate::composition::ClipKind::Audio => (
                "Import Audio",
                "Audio",
                vec!["wav", "mp3", "flac", "ogg", "opus", "m4a", "aac"],
            ),
            crate::composition::ClipKind::Text => return,
            crate::composition::ClipKind::Adjustment => return,
        };
        if let Some(path) = rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter, &extensions)
            .pick_file()
        {
            let previous = self.presentation_studio.project.clone();
            match self.presentation_studio.project.add_source(&path) {
                Ok(id) => {
                    let source = self
                        .presentation_studio
                        .project
                        .sources
                        .iter()
                        .find(|source| source.id == id)
                        .unwrap();
                    if (expected == crate::composition::ClipKind::Video && !source.has_video)
                        || (expected == crate::composition::ClipKind::Audio && !source.has_audio)
                    {
                        self.presentation_studio
                            .project
                            .sources
                            .retain(|item| item.id != id);
                        self.presentation_studio.error = Some(format!(
                            "{} does not contain the requested {} stream.",
                            path.display(),
                            filter.to_ascii_lowercase()
                        ));
                        return;
                    }
                    self.presentation_studio.selected_source = Some(id);
                    self.presentation_remember_undo(previous);
                    self.presentation_studio.message = format!(
                        "Imported {}. Choose Add Video or Add Audio to place it on the timeline.",
                        path.display()
                    );
                    self.presentation_studio.error = None;
                }
                Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
            }
        }
    }

    fn presentation_import_dropped_files(&mut self, files: &[PathBuf]) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop the microphone take before changing composition media.".into());
            return;
        }
        let previous = self.presentation_studio.project.clone();
        let mut imported = Vec::new();
        let mut rejected = Vec::new();
        for path in files {
            if !path.is_file() {
                rejected.push(format!("{} is not a supported media file", path.display()));
                continue;
            }
            let is_composition = path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sonocomp"));
            let result = if is_composition {
                self.presentation_studio
                    .project
                    .add_nested_composition_source(path)
            } else {
                self.presentation_studio.project.add_source(path)
            };
            match result {
                Ok(id) => imported.push((id, path.clone())),
                Err(error) => rejected.push(format!("{}: {error:#}", path.display())),
            }
        }
        if imported.is_empty() {
            self.presentation_studio.error = Some(if rejected.is_empty() {
                "No media files were dropped.".into()
            } else {
                format!("No media was imported. {}", rejected.join("; "))
            });
            return;
        }
        self.presentation_studio.selected_source = imported.last().map(|(id, _)| id.clone());
        self.presentation_remember_undo(previous);
        self.presentation_studio.message = format!(
            "Imported {} media/composition source(s); ordinary media was stream-probed and child compositions were validated. Select each source and use its insertion control to place it on the timeline.",
            imported.len()
        );
        self.presentation_studio.error = if rejected.is_empty() {
            None
        } else {
            Some(format!(
                "Some dropped files were skipped: {}",
                rejected.join("; ")
            ))
        };
    }

    fn start_presentation_preview(&mut self, ctx: &egui::Context) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop microphone capture before rendering a preview.".into());
            return;
        }
        if self.presentation_studio.job.is_some() {
            return;
        }
        let project = self.presentation_studio.project.clone();
        let time = self
            .presentation_studio
            .playhead
            .clamp(0.0, project.duration);
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                crate::composition::preview_png(&project, time, &worker_control)
            })
            .map(|png| PresentationJobOutcome::Preview { time, png })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: None,
            worker,
            control,
        });
        self.presentation_studio.error = None;
        self.presentation_studio.message = format!("Rendering preview at {time:.3}s…");
    }

    fn start_presentation_source_preview(
        &mut self,
        ctx: &egui::Context,
        source_id: String,
        source_time: f64,
    ) {
        if self.presentation_studio.job.is_some() {
            return;
        }
        let project = self.presentation_studio.project.clone();
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                crate::composition::preview_source_png(
                    &project,
                    &source_id,
                    source_time,
                    &worker_control,
                )
            })
            .map(|png| PresentationJobOutcome::SourcePreview {
                time: source_time,
                png,
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: None,
            worker,
            control,
        });
        self.presentation_studio.error = None;
        self.presentation_studio.message = format!("Rendering source monitor at {source_time:.3}s");
    }

    fn start_presentation_playback(&mut self, ctx: &egui::Context, paused: bool) {
        if let Some(playback) = &self.presentation_studio.playback {
            if !playback.is_ended() {
                playback.set_playing(!paused);
                return;
            }
            self.presentation_studio.playback = None;
            self.presentation_studio.playhead = 0.0;
        }
        if self.presentation_studio.playback_start.is_some() {
            return;
        }
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop microphone capture before starting timeline playback.".into());
            return;
        }
        let project = self.presentation_studio.project.clone();
        let start = self
            .presentation_studio
            .playhead
            .clamp(0.0, project.duration);
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let playback =
                crate::timeline_playback::TimelinePlayback::start(&project, start, paused)
                    .map_err(|error| format!("{error:#}"));
            let _ = tx.send(playback);
            wake.request_repaint();
        });
        self.presentation_studio.playback_start =
            Some(PresentationPlaybackStart { result, worker });
        self.presentation_studio.error = None;
        self.presentation_studio.message = "Preparing continuous video and audio preview…".into();
    }

    fn seek_presentation_playback(&mut self, ctx: &egui::Context, time: f64) {
        let was_active = self.presentation_studio.playback.is_some();
        let was_playing = self
            .presentation_studio
            .playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing());
        self.presentation_studio.playback = None;
        self.presentation_studio.playhead =
            time.clamp(0.0, self.presentation_studio.project.duration);
        if was_active {
            self.start_presentation_playback(ctx, !was_playing);
        } else {
            self.start_presentation_preview(ctx);
        }
    }

    fn poll_presentation_playback(&mut self, ctx: &egui::Context) {
        let mut completed = None;
        if let Some(start) = &self.presentation_studio.playback_start {
            match start.result.try_recv() {
                Ok(result) => completed = Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    completed = Some(Err("playback startup ended unexpectedly".into()))
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(result) = completed {
            if let Some(start) = self.presentation_studio.playback_start.take() {
                let _ = start.worker.join();
            }
            match result {
                Ok(playback) => {
                    self.presentation_studio.playhead = playback.position_seconds();
                    self.presentation_studio.playback = Some(playback);
                    self.presentation_studio.message = "Playing composition preview.".into();
                    self.presentation_studio.error = None;
                    if self.presentation_studio.record_voiceover_after_playback {
                        self.presentation_studio.record_voiceover_after_playback = false;
                        self.presentation_start_audio(true);
                        if let Some(playback) = &self.presentation_studio.playback {
                            playback.set_playing(true);
                        }
                    }
                }
                Err(error) => {
                    self.presentation_studio.error = Some(error);
                    self.presentation_studio.message =
                        "Could not start composition playback.".into();
                    if self.presentation_studio.record_voiceover_after_playback {
                        self.presentation_studio.record_voiceover_after_playback = false;
                        self.presentation_start_audio(true);
                        self.presentation_studio.error = Some(
                            "Playback could not start; microphone capture is active without moving video preview.".into(),
                        );
                    }
                }
            }
        }

        let mut stop_playback = false;
        if let Some(playback) = &mut self.presentation_studio.playback {
            if !self.presentation_studio.scrubbing {
                self.presentation_studio.playhead = playback.position_seconds();
            }
            match playback.next_video_frame() {
                Ok(Some(frame)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width as usize, frame.height as usize],
                        &frame.rgba,
                    );
                    self.presentation_studio.preview_texture = Some(ctx.load_texture(
                        "composition-playback-frame",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    self.presentation_studio.error = Some(format!("{error:#}"));
                    stop_playback = true;
                }
            }
            if let Some(error) = playback.take_error() {
                self.presentation_studio.error = Some(error);
                stop_playback = true;
            } else if playback.is_ended() {
                self.presentation_studio.playhead = self.presentation_studio.project.duration;
                stop_playback = true;
                self.presentation_studio.message =
                    "Playback reached the end of the composition.".into();
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self.presentation_studio.playback_start.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if stop_playback {
            self.presentation_studio.playback = None;
        }
    }

    fn start_presentation_export(&mut self, ctx: &egui::Context) {
        if self.presentation_studio.audio_capture.is_some() {
            self.presentation_studio.error =
                Some("Stop microphone capture before exporting the composition.".into());
            return;
        }
        if self.presentation_studio.job.is_some() {
            return;
        }
        let output = self.presentation_studio.output.clone();
        if output.as_os_str().is_empty() {
            self.presentation_studio.error = Some("Choose an MP4 output path first.".into());
            return;
        }
        if output.exists() {
            self.presentation_studio.error = Some("That output already exists. Choose a different file; composition export never overwrites.".into());
            return;
        }
        if let Err(error) = self.presentation_studio.project.validate() {
            self.presentation_studio.error = Some(format!("{error:#}"));
            return;
        }
        let project = self.presentation_studio.project.clone();
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker_output = output.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                crate::composition::render(&project, &worker_output, &worker_control)
            })
            .map(|()| PresentationJobOutcome::Export {
                output: worker_output,
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: None,
            worker,
            control,
        });
        self.presentation_studio.error = None;
        self.presentation_studio.message = "Exporting standalone composition…".into();
    }

    fn start_presentation_proxy_generation(
        &mut self,
        ctx: &egui::Context,
        source_id: String,
        output: PathBuf,
    ) {
        if self.presentation_studio.job.is_some() {
            return;
        }
        let Some(source) = self
            .presentation_studio
            .project
            .sources
            .iter()
            .find(|source| source.id == source_id)
        else {
            self.presentation_studio.error = Some("Selected media source no longer exists.".into());
            return;
        };
        let source_path = match source.path.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                self.presentation_studio.error = Some(format!(
                    "Cannot generate a proxy for offline media {}: {error}",
                    source.display_name
                ));
                return;
            }
        };
        let worker_source_path = source_path.clone();
        let worker_source_id = source_id.clone();
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                crate::composition::generate_video_proxy(
                    &worker_source_path,
                    &output,
                    &worker_control,
                )
            })
            .map(|proxy_path| PresentationJobOutcome::ProxyGenerated {
                source_id: worker_source_id,
                source_path: worker_source_path,
                proxy_path,
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: None,
            worker,
            control,
        });
        self.presentation_studio.error = None;
        self.presentation_studio.message = "Generating a video-only preview proxy…".into();
    }

    fn start_speech_dependency_install(
        &mut self,
        ctx: &egui::Context,
        dependency: crate::subtitles::SpeechDependency,
    ) {
        if self.presentation_studio.job.is_some() {
            return;
        }
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let (progress_tx, progress) = mpsc::channel();
        let wake = ctx.clone();
        let worker_wake = wake.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                crate::subtitles::install_dependency(
                    dependency,
                    &worker_control,
                    |message, fraction| {
                        let _ = progress_tx.send(PresentationProgress { message, fraction });
                        worker_wake.request_repaint();
                    },
                )
                .map(|path| PresentationJobOutcome::SpeechDependencyInstalled { dependency, path })
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: Some(progress),
            worker,
            control,
        });
        self.presentation_studio.job_progress = None;
        self.presentation_studio.error = None;
        self.presentation_studio.message = match dependency {
            crate::subtitles::SpeechDependency::Runtime => {
                "Installing the optional local CPU speech recognizer…".into()
            }
            crate::subtitles::SpeechDependency::MultilingualBaseModel => {
                "Downloading the multilingual Base speech model…".into()
            }
        };
    }

    fn start_presentation_transcription(&mut self, ctx: &egui::Context, clip_id: String) {
        if self.presentation_studio.job.is_some() {
            return;
        }
        let project = self.presentation_studio.project.clone();
        let Some(clip) = project
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .cloned()
        else {
            self.presentation_studio.error =
                Some("Selected timeline clip no longer exists.".into());
            return;
        };
        let Some(source_id) = clip.source_id.clone() else {
            self.presentation_studio.error = Some("Selected clip has no media source.".into());
            return;
        };
        let Some(source) = project.sources.iter().find(|source| source.id == source_id) else {
            self.presentation_studio.error =
                Some("Selected clip's media source no longer exists.".into());
            return;
        };
        let source_name = source.display_name.clone();
        let worker_source_name = source_name.clone();
        let language = self.presentation_studio.transcription_language.clone();
        let preset_id = self.presentation_studio.selected_caption_preset.clone();
        let snapshot = match serde_json::to_vec(&project) {
            Ok(snapshot) => crate::content_hash::sha256_hex(&snapshot),
            Err(error) => {
                self.presentation_studio.error = Some(format!(
                    "Could not snapshot the project before transcription: {error}"
                ));
                return;
            }
        };
        let metadata = crate::composition::CaptionMetadata {
            origin: crate::composition::CaptionOrigin::Transcribed,
            source_id: Some(source_id),
            source_clip_id: Some(clip.id.clone()),
            language: Some(language.clone()),
            model_id: Some("whisper.cpp 1.9.2 / ggml-base".into()),
            ..Default::default()
        };
        let offset = clip.timeline_start;
        let end_limit = clip.timeline_start + clip.duration;
        let control = ExportControl::default();
        let worker_control = control.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let (progress_tx, progress) = mpsc::channel();
        let wake = ctx.clone();
        let worker_wake = wake.clone();
        let worker = std::thread::spawn(move || {
            let outcome = catch_export_panic(|| {
                let cues = crate::subtitles::transcribe_clip(
                    &project,
                    &clip_id,
                    &language,
                    &worker_control,
                    |message, fraction| {
                        let _ = progress_tx.send(PresentationProgress { message, fraction });
                        worker_wake.request_repaint();
                    },
                )?;
                worker_control.check()?;
                let mut completed_project = project.clone();
                let caption_ids = crate::subtitles::insert_cues(
                    &mut completed_project,
                    &cues,
                    offset,
                    Some(end_limit),
                    &preset_id,
                    &metadata,
                )?;
                Ok(PresentationJobOutcome::SubtitlesGenerated {
                    snapshot_digest: snapshot,
                    source_name: worker_source_name,
                    caption_ids,
                    project: completed_project,
                })
            })
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(outcome);
            wake.request_repaint();
        });
        self.presentation_studio.job = Some(PresentationJob {
            result,
            progress: Some(progress),
            worker,
            control,
        });
        self.presentation_studio.job_progress = None;
        self.presentation_studio.error = None;
        self.presentation_studio.message = format!(
            "Preparing to transcribe the selected clip '{}' locally…",
            source_name
        );
    }

    fn import_presentation_subtitles(&mut self, path: &Path) {
        let result = crate::subtitles::parse_subtitle_file(path).and_then(|(_, cues)| {
            let previous = self.presentation_studio.project.clone();
            let metadata = crate::composition::CaptionMetadata {
                origin: crate::composition::CaptionOrigin::Imported,
                ..Default::default()
            };
            let ids = crate::subtitles::insert_cues(
                &mut self.presentation_studio.project,
                &cues,
                self.presentation_studio.playhead,
                None,
                &self.presentation_studio.selected_caption_preset,
                &metadata,
            )?;
            self.presentation_studio.selected_clip = ids.last().cloned();
            self.presentation_studio.selected_clips.clear();
            self.presentation_studio
                .selected_clips
                .extend(ids.iter().cloned());
            self.presentation_remember_undo(previous);
            self.presentation_studio.dirty = true;
            self.presentation_studio.error = None;
            self.presentation_studio.message = format!(
                "Imported {} editable caption cues from {}.",
                ids.len(),
                path.display()
            );
            Ok(())
        });
        if let Err(error) = result {
            self.presentation_studio.error = Some(format!("{error:#}"));
        }
    }

    fn export_presentation_subtitles(
        &mut self,
        path: &Path,
        format: crate::subtitles::SubtitleFormat,
    ) {
        let result = (|| -> anyhow::Result<usize> {
            let cues = crate::subtitles::cues_from_project(&self.presentation_studio.project);
            if cues.is_empty() {
                anyhow::bail!(
                    "this composition has no imported or generated subtitle cues to export"
                );
            }
            let text = crate::subtitles::serialize_cues(&cues, format)?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let mut staged = tempfile::NamedTempFile::new_in(parent)
                .context("staging subtitle export beside its destination")?;
            use std::io::Write;
            staged
                .write_all(text.as_bytes())
                .context("writing subtitle export")?;
            staged
                .as_file()
                .sync_all()
                .context("flushing subtitle export")?;
            staged
                .persist(path)
                .map_err(|error| error.error)
                .context("atomically publishing subtitle export")?;
            Ok(cues.len())
        })();
        match result {
            Ok(count) => {
                self.presentation_studio.error = None;
                self.presentation_studio.message =
                    format!("Exported {count} subtitle cues to {}.", path.display());
            }
            Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
        }
    }

    fn poll_presentation_job(&mut self, ctx: &egui::Context) {
        let latest_progress = self
            .presentation_studio
            .job
            .as_ref()
            .and_then(|job| job.progress.as_ref())
            .and_then(|progress| progress.try_iter().last());
        if let Some(progress) = latest_progress {
            self.presentation_studio.message = progress.message.clone();
            self.presentation_studio.job_progress = Some(progress);
        }
        let result =
            self.presentation_studio
                .job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("composition worker ended without a result".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        let Some(result) = result else { return };
        let Some(job) = self.presentation_studio.job.take() else {
            return;
        };
        let _ = job.worker.join();
        self.presentation_studio.job_progress = None;
        match result {
            Ok(PresentationJobOutcome::Preview { time, png }) => {
                match image::load_from_memory(&png) {
                    Ok(image) => {
                        let rgba = image.to_rgba8();
                        let color = egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.as_raw(),
                        );
                        self.presentation_studio.preview_texture = Some(ctx.load_texture(
                            "composition-preview",
                            color,
                            egui::TextureOptions::LINEAR,
                        ));
                        self.presentation_studio.preview_png = Some((time, png));
                        self.presentation_studio.message =
                            format!("Preview rendered at {time:.3}s.");
                        self.presentation_studio.error = None;
                    }
                    Err(error) => {
                        self.presentation_studio.error =
                            Some(format!("decoding preview PNG: {error}"))
                    }
                }
            }
            Ok(PresentationJobOutcome::SourcePreview { time, png }) => {
                match image::load_from_memory(&png) {
                    Ok(image) => {
                        let rgba = image.to_rgba8();
                        let color = egui::ColorImage::from_rgba_unmultiplied(
                            [rgba.width() as usize, rgba.height() as usize],
                            rgba.as_raw(),
                        );
                        self.presentation_studio.source_preview_texture = Some(ctx.load_texture(
                            "composition-source-preview",
                            color,
                            egui::TextureOptions::LINEAR,
                        ));
                        self.presentation_studio.message =
                            format!("Source monitor rendered at {time:.3}s.");
                        self.presentation_studio.error = None;
                    }
                    Err(error) => {
                        self.presentation_studio.error =
                            Some(format!("decoding source monitor PNG: {error}"));
                    }
                }
            }
            Ok(PresentationJobOutcome::Export { output }) => {
                self.presentation_studio.message = format!("Export complete: {}", output.display());
                self.presentation_studio.error = None;
            }
            Ok(PresentationJobOutcome::ProxyGenerated {
                source_id,
                source_path,
                proxy_path,
            }) => {
                let source_matches = self
                    .presentation_studio
                    .project
                    .sources
                    .iter()
                    .find(|source| source.id == source_id)
                    .and_then(|source| source.path.canonicalize().ok())
                    .is_some_and(|path| path == source_path);
                if !source_matches {
                    let _ = std::fs::remove_file(&proxy_path);
                    self.presentation_studio.error = Some(
                        "The source changed while its proxy was being generated; the unattached generated file was removed.".into(),
                    );
                } else {
                    let previous = self.presentation_studio.project.clone();
                    match self
                        .presentation_studio
                        .project
                        .set_generated_source_proxy(&source_id, &proxy_path)
                    {
                        Ok(()) => {
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.message = format!(
                                "Proxy generated and attached: {}. Exports continue to use the original source.",
                                proxy_path.display()
                            );
                            self.presentation_studio.error = None;
                        }
                        Err(error) => {
                            self.presentation_studio.error =
                                Some(format!("Generated proxy could not be attached: {error:#}"));
                            self.presentation_studio.message = format!(
                                "Generated proxy remains at {} for inspection or manual cleanup.",
                                proxy_path.display()
                            );
                        }
                    }
                }
            }
            Ok(PresentationJobOutcome::SpeechDependencyInstalled { dependency, path }) => {
                self.presentation_studio.error = None;
                self.presentation_studio.message = match dependency {
                    crate::subtitles::SpeechDependency::Runtime => format!(
                        "Local whisper.cpp CPU recognizer installed at {}. Download the multilingual model separately to enable transcription.",
                        path.display()
                    ),
                    crate::subtitles::SpeechDependency::MultilingualBaseModel => format!(
                        "Multilingual Base speech model installed at {}.",
                        path.display()
                    ),
                };
            }
            Ok(PresentationJobOutcome::SubtitlesGenerated {
                snapshot_digest,
                source_name,
                caption_ids,
                project,
            }) => {
                let current_digest = serde_json::to_vec(&self.presentation_studio.project)
                    .ok()
                    .map(|bytes| crate::content_hash::sha256_hex(&bytes));
                if current_digest.as_deref() != Some(snapshot_digest.as_str()) {
                    self.presentation_studio.error = Some(
                        "Transcription finished after the composition changed. No captions were inserted; run Generate Subtitles again for the current clip timing.".into(),
                    );
                } else {
                    let previous = self.presentation_studio.project.clone();
                    self.presentation_studio.project = project;
                    self.presentation_studio.selected_clip = caption_ids.last().cloned();
                    self.presentation_studio.selected_clips.clear();
                    self.presentation_studio
                        .selected_clips
                        .extend(caption_ids.iter().cloned());
                    self.presentation_remember_undo(previous);
                    self.presentation_studio.dirty = true;
                    self.presentation_studio.error = None;
                    self.presentation_studio.message = format!(
                        "Created {} editable caption cues from {}. Review the text and timing before export.",
                        caption_ids.len(),
                        source_name
                    );
                }
            }
            Err(error) => {
                self.presentation_studio.error = Some(error);
                self.presentation_studio.message =
                    "Composition operation failed or was canceled.".into();
            }
        }
    }

    fn presentation_studio_ui(&mut self, ui: &mut egui::Ui) {
        if !self.presentation_studio.recovery_checked {
            self.presentation_refresh_recovery();
        }
        if !self.presentation_studio.audio_devices_loaded {
            self.presentation_refresh_audio_devices();
        }
        let connected_device = self
            .presentation_studio
            .audio_capture
            .as_ref()
            .map(|capture| capture.device_id().to_owned());
        if connected_device.is_some()
            && self.presentation_studio.audio_device_check.elapsed() >= Duration::from_secs(2)
        {
            self.presentation_refresh_audio_devices();
            let disconnected = connected_device.as_ref().is_some_and(|active| {
                !self
                    .presentation_studio
                    .audio_devices
                    .iter()
                    .any(|device| &device.id == active)
            });
            if disconnected {
                self.presentation_stop_audio();
                self.presentation_studio.error = Some(
                    "The microphone disconnected during capture. Sono-Renderer finalized the audio received before disconnection; inspect the take boundary before use.".into(),
                );
            }
        }
        if self
            .presentation_studio
            .audio_capture
            .as_ref()
            .is_some_and(|capture| capture.level().error.is_some())
        {
            self.presentation_stop_audio();
        }
        ui.heading("Presentation Studio");
        ui.small("Drop ordinary video, image, or audio files into the window to probe and add them to this project's media bin.");
        ui.label(self.app_text("presentation.description"));
        let mut recover_index = None;
        let mut clear_recovery_index = None;
        if !self.presentation_studio.recovery_snapshots.is_empty() {
            ui.group(|ui| {
                ui.label("Recovery checkpoints  Echoose one to restore; the last explicit project save is kept separately.");
                for (index, (_, recovery)) in self
                    .presentation_studio
                    .recovery_snapshots
                    .iter()
                    .enumerate()
                {
                    let name = recovery
                        .project_path
                        .as_deref()
                        .and_then(|path| path.file_name())
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Unsaved draft".into());
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let age_seconds = now_ms
                        .saturating_sub(recovery.checkpoint_unix_ms)
                        .saturating_div(1000);
                    ui.horizontal(|ui| {
                        ui.label(format!("{name}  Echeckpoint {age_seconds}s ago"));
                        if ui.small_button("Recover").clicked() {
                            recover_index = Some(index);
                        }
                        if ui.small_button("Remove").clicked() {
                            clear_recovery_index = Some(index);
                        }
                    });
                }
            });
        }
        if let Some(index) = recover_index {
            let (_, recovery) = self.presentation_studio.recovery_snapshots.remove(index);
            self.presentation_studio.project = recovery.project;
            self.presentation_studio.project_path = recovery.project_path;
            self.presentation_studio.output = self
                .presentation_studio
                .project_path
                .as_deref()
                .map(|path| path.with_extension("mp4"))
                .unwrap_or_else(|| PathBuf::from("output.mp4"));
            self.presentation_studio.playhead = 0.0;
            self.presentation_studio.selected_clip = None;
            self.presentation_studio.selected_clips.clear();
            self.presentation_studio.dirty = true;
            self.presentation_studio.autosave_pending = false;
            self.presentation_studio.last_autosave = Instant::now();
            self.presentation_studio.message =
                "Recovered the working composition. Save to update its project file.".into();
        }
        if let Some(index) = clear_recovery_index {
            if index < self.presentation_studio.recovery_snapshots.len() {
                let (path, _) = self.presentation_studio.recovery_snapshots.remove(index);
                if let Err(error) = std::fs::remove_file(path) {
                    self.presentation_studio.error =
                        Some(format!("Could not remove recovery checkpoint: {error:#}"));
                }
            }
        }
        let studio_idle = self.presentation_studio.job.is_none()
            && self.presentation_studio.audio_capture.is_none()
            && self.presentation_studio.playback.is_none()
            && self.presentation_studio.playback_start.is_none();
        ui.add_enabled_ui(studio_idle, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("New").clicked() {
                    self.dispatch_command(crate::workspace::CommandId::NewComposition, ui.ctx());
                }
                if ui.button("Open…").clicked() {
                    self.dispatch_command(crate::workspace::CommandId::OpenComposition, ui.ctx());
                }
                if ui.button("Save").clicked() {
                    self.dispatch_command(crate::workspace::CommandId::SaveComposition, ui.ctx());
                }
                if ui.button("Save As…").clicked() {
                    self.dispatch_command(crate::workspace::CommandId::SaveCompositionAs, ui.ctx());
                }
                if ui.button("Save Template…").clicked() {
                    let template_name = self.presentation_studio.project.name.clone();
                    let path = rfd::FileDialog::new()
                        .set_title("Save Presentation Template")
                        .add_filter("Sono-Renderer template", &["sonotemplate"])
                        .set_file_name("presentation.sonotemplate")
                        .save_file();
                    if let Some(path) = path {
                        match crate::composition::CompositionTemplate::from_project(
                            &self.presentation_studio.project,
                            &template_name,
                        )
                        .and_then(|template| template.save(&path))
                        {
                            Ok(()) => {
                                self.presentation_studio.error = None;
                                self.presentation_studio.message = format!(
                                    "Saved portable template to {}. Imported media is represented by source slots, and applied objects remain editable.",
                                    path.display()
                                );
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!("Could not save template: {error:#}"));
                            }
                        }
                    }
                }
                if ui.button("Apply Template…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Open Presentation Template")
                        .add_filter("Sono-Renderer template", &["sonotemplate"])
                        .pick_file()
                    {
                        match crate::composition::CompositionTemplate::load(&path) {
                            Ok(template) => {
                                let project = &self.presentation_studio.project;
                                let bindings = template
                                    .source_slots
                                    .iter()
                                    .filter_map(|slot| {
                                        let compatible = project.sources.iter().filter(|source| {
                                            (!slot.requires_video || source.has_video)
                                                && (!slot.requires_audio || source.has_audio)
                                                && (!slot.still_image || source.still_image)
                                        }).collect::<Vec<_>>();
                                        (compatible.len() == 1).then(|| (slot.id.clone(), compatible[0].id.clone()))
                                    })
                                    .collect();
                                self.presentation_studio.template_text_values = template
                                    .text_parameters
                                    .iter()
                                    .map(|parameter| (parameter.name.clone(), parameter.default_value.clone()))
                                    .collect();
                                self.presentation_studio.template_source_bindings = bindings;
                                self.presentation_studio.template_responsive = template.responsive;
                                self.presentation_studio.presentation_template = Some(template);
                                self.presentation_studio.error = None;
                                self.presentation_studio.message = format!(
                                    "Loaded template {}. Choose each media source slot, then apply it as ordinary editable timeline objects.",
                                    path.display()
                                );
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!("Could not open template: {error:#}"));
                            }
                        }
                    }
                }
                if ui.button("Import Video…").clicked() {
                    self.presentation_import_media(crate::composition::ClipKind::Video);
                }
                if ui.button("Import Audio…").clicked() {
                    self.presentation_import_media(crate::composition::ClipKind::Audio);
                }
                if ui.button("Add Nested Composition...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Add Nested Composition")
                        .add_filter("Sono Composition", &["sonocomp"])
                        .pick_file()
                    {
                        let previous = self.presentation_studio.project.clone();
                        match self
                            .presentation_studio
                            .project
                            .add_nested_composition_source(&path)
                        {
                            Ok(id) => {
                                self.presentation_studio.selected_source = Some(id);
                                self.presentation_remember_undo(previous);
                                self.presentation_studio.message =
                                    format!("Added live composition source {}.", path.display());
                                self.presentation_studio.error = None;
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!("{error:#}"));
                            }
                        }
                    }
                }
                if ui.button("Add Text").clicked() {
                    let previous = self.presentation_studio.project.clone();
                    match self.presentation_studio.project.add_text_clip(
                        "New caption",
                        self.presentation_studio.playhead,
                        5.0f64.min(
                            self.presentation_studio.project.duration
                                - self.presentation_studio.playhead,
                        ),
                    ) {
                        Ok(id) => {
                            self.presentation_studio.selected_clip = Some(id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
                if ui.button("Add Adjustment").clicked() {
                    let previous = self.presentation_studio.project.clone();
                    let fps = self.presentation_studio.project.fps;
                    let duration = self.presentation_studio.project.duration;
                    let frame = 1.0 / f64::from(fps);
                    let start = self
                        .presentation_studio
                        .playhead
                        .min((duration - frame).max(0.0));
                    let clip_duration = 5.0f64.min(duration - start).max(frame);
                    match self
                        .presentation_studio
                        .project
                        .add_adjustment_clip(start, clip_duration)
                    {
                        Ok(id) => {
                            self.presentation_studio.selected_clip = Some(id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => {
                            self.presentation_studio.error = Some(format!("{error:#}"));
                        }
                    }
                }
            });
            if let Some(template) = self.presentation_studio.presentation_template.clone() {
                let source_rows = template.source_slots.clone();
                let parameters = template.text_parameters.clone();
                let sources = self.presentation_studio.project.sources.clone();
                let mut apply_template = false;
                ui.collapsing(format!("Template: {}", template.name), |ui| {
                    ui.label("Templates copy ordinary clips, tracks, effects, masks, keyframes, and transitions into this composition. They do not replace the editor or load Sonolus runtime state.");
                    ui.checkbox(
                        &mut self.presentation_studio.template_responsive,
                        "Adapt normalized layout to this composition size",
                    );
                    for slot in &source_rows {
                        let selected = self
                            .presentation_studio
                            .template_source_bindings
                            .get(&slot.id)
                            .cloned()
                            .unwrap_or_default();
                        let selected_name = sources
                            .iter()
                            .find(|source| source.id == selected)
                            .map(|source| source.display_name.as_str())
                            .unwrap_or("Choose media source…");
                        egui::ComboBox::from_id_salt(("presentation-template-slot", &slot.id))
                            .selected_text(format!("{}: {selected_name}", slot.label))
                            .show_ui(ui, |ui| {
                                for source in sources.iter().filter(|source| {
                                    (!slot.requires_video || source.has_video)
                                        && (!slot.requires_audio || source.has_audio)
                                        && (!slot.still_image || source.still_image)
                                }) {
                                    ui.selectable_value(
                                        self.presentation_studio
                                            .template_source_bindings
                                            .entry(slot.id.clone())
                                            .or_default(),
                                        source.id.clone(),
                                        &source.display_name,
                                    );
                                }
                            });
                    }
                    for parameter in &parameters {
                        ui.horizontal(|ui| {
                            ui.label(&parameter.name);
                            let value = self
                                .presentation_studio
                                .template_text_values
                                .entry(parameter.name.clone())
                                .or_insert_with(|| parameter.default_value.clone());
                            ui.text_edit_singleline(value);
                        });
                    }
                    let bindings_complete = source_rows.iter().all(|slot| {
                        self.presentation_studio
                            .template_source_bindings
                            .get(&slot.id)
                            .is_some_and(|id| sources.iter().any(|source| source.id == *id))
                    });
                    if ui
                        .add_enabled(bindings_complete, egui::Button::new("Apply as editable layers"))
                        .clicked()
                    {
                        apply_template = true;
                    }
                    if !bindings_complete {
                        ui.small("Import/select media in the bin and bind every required source slot first.");
                    }
                });
                if apply_template {
                    let previous = self.presentation_studio.project.clone();
                    match template.apply_to_project(
                        &mut self.presentation_studio.project,
                        &self.presentation_studio.template_source_bindings,
                        &self.presentation_studio.template_text_values,
                        self.presentation_studio.template_responsive,
                    ) {
                        Ok(ids) => {
                            self.presentation_studio.selected_clip = ids.first().cloned();
                            self.presentation_studio.selected_clips.clear();
                            self.presentation_studio.selected_clips.extend(ids);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.dirty = true;
                            self.presentation_studio.error = None;
                            self.presentation_studio.message = "Template applied as native composition tracks and clips. Edit, keyframe, extend, and export these objects normally.".into();
                        }
                        Err(error) => {
                            self.presentation_studio.error = Some(format!("Could not apply template: {error:#}"));
                        }
                    }
                }
            }
            let title_presets: Vec<_> = self
                .presentation_studio
                .project
                .title_presets
                .iter()
                .map(|preset| (preset.id.clone(), preset.name.clone(), preset.built_in))
                .collect();
            if !title_presets
                .iter()
                .any(|(id, _, _)| id == &self.presentation_studio.selected_title_preset)
            {
                self.presentation_studio.selected_title_preset = title_presets
                    .first()
                    .map(|(id, _, _)| id.clone())
                    .unwrap_or_default();
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Title preset");
                let selected_name = title_presets
                    .iter()
                    .find(|(id, _, _)| id == &self.presentation_studio.selected_title_preset)
                    .map(|(_, name, _)| name.as_str())
                    .unwrap_or("No presets");
                egui::ComboBox::from_id_salt("presentation-title-preset")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        for (id, name, _) in &title_presets {
                            ui.selectable_value(
                                &mut self.presentation_studio.selected_title_preset,
                                id.clone(),
                                name,
                            );
                        }
                    });
                if ui.button("Add title").clicked() {
                    let previous = self.presentation_studio.project.clone();
                    let fps = self.presentation_studio.project.fps;
                    let duration = self.presentation_studio.project.duration;
                    let frame = 1.0 / f64::from(fps);
                    let start = self
                        .presentation_studio
                        .playhead
                        .min((duration - frame).max(0.0));
                    let clip_duration = 5.0f64.min(duration - start).max(frame);
                    match self.presentation_studio.project.add_title_from_preset(
                        &self.presentation_studio.selected_title_preset,
                        start,
                        clip_duration,
                    ) {
                        Ok(id) => {
                            self.presentation_studio.selected_clip = Some(id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.presentation_studio.title_preset_name)
                        .desired_width(130.0)
                        .hint_text("Preset name"),
                );
                if ui.button("Save selected style").clicked() {
                    if let Some(clip_id) = self.presentation_studio.selected_clip.clone() {
                        let previous = self.presentation_studio.project.clone();
                        match self
                            .presentation_studio
                            .project
                            .save_title_preset_from_clip(
                                &clip_id,
                                &self.presentation_studio.title_preset_name,
                            ) {
                            Ok(id) => {
                                self.presentation_studio.selected_title_preset = id;
                                self.presentation_studio.title_preset_name.clear();
                                self.presentation_remember_undo(previous);
                                self.presentation_studio.error = None;
                            }
                            Err(error) => {
                                self.presentation_studio.error = Some(format!("{error:#}"));
                            }
                        }
                    } else {
                        self.presentation_studio.error =
                            Some("Select a text clip before saving its style.".into());
                    }
                }
                if title_presets.iter().any(|(id, _, built_in)| {
                    id == &self.presentation_studio.selected_title_preset && !built_in
                }) && ui.button("Remove preset").clicked()
                {
                    let previous = self.presentation_studio.project.clone();
                    match self
                        .presentation_studio
                        .project
                        .remove_title_preset(&self.presentation_studio.selected_title_preset)
                    {
                        Ok(()) => {
                            self.presentation_studio.selected_title_preset =
                                "builtin-centered-title".into();
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
            });
            let mut add_shape_kind = None;
            ui.horizontal(|ui| {
                ui.label("Solid color");
                ui.add(
                    egui::TextEdit::singleline(&mut self.presentation_studio.solid_color_input)
                        .desired_width(72.0)
                        .hint_text("RRGGBB"),
                );
                if ui.button("Add color clip").clicked() {
                    let previous = self.presentation_studio.project.clone();
                    let result = self
                        .presentation_studio
                        .project
                        .add_solid_color_source(&self.presentation_studio.solid_color_input)
                        .and_then(|source_id| {
                            self.presentation_studio.project.add_clip(
                                &source_id,
                                crate::composition::ClipKind::Video,
                                self.presentation_studio.playhead,
                            )
                        });
                    match result {
                        Ok(id) => {
                            self.presentation_studio.selected_clip = Some(id);
                            self.presentation_remember_undo(previous);
                            self.presentation_studio.error = None;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
                if ui.button("Add rectangle").clicked() {
                    add_shape_kind = Some(crate::composition::GeneratedShapeKind::Rectangle);
                }
                if ui.button("Add ellipse").clicked() {
                    add_shape_kind = Some(crate::composition::GeneratedShapeKind::Ellipse);
                }
                if ui.button("Add line").clicked() {
                    add_shape_kind = Some(crate::composition::GeneratedShapeKind::Line);
                }
            });
            if let Some(kind) = add_shape_kind {
                let previous = self.presentation_studio.project.clone();
                let result = self
                    .presentation_studio
                    .project
                    .add_shape_source(kind, &self.presentation_studio.solid_color_input)
                    .and_then(|source_id| {
                        self.presentation_studio.project.add_clip(
                            &source_id,
                            crate::composition::ClipKind::Video,
                            self.presentation_studio.playhead,
                        )
                    });
                match result {
                    Ok(id) => {
                        self.presentation_studio.selected_clip = Some(id);
                        self.presentation_remember_undo(previous);
                        self.presentation_studio.error = None;
                    }
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
        });
        let mut undo_requested = false;
        let mut redo_requested = false;
        ui.horizontal(|ui| {
            undo_requested = ui
                .add_enabled(
                    self.presentation_studio.edit_history.can_undo()
                        && self.presentation_studio.job.is_none()
                        && self.presentation_studio.audio_capture.is_none(),
                    egui::Button::new(format!(
                        "Undo ({})",
                        self.shortcut_label(
                            crate::workspace::COMMANDS
                                .iter()
                                .find(|command| command.id == crate::workspace::CommandId::Undo)
                                .expect("registered command")
                        )
                    )),
                )
                .clicked();
            redo_requested = ui
                .add_enabled(
                    self.presentation_studio.edit_history.can_redo()
                        && self.presentation_studio.job.is_none()
                        && self.presentation_studio.audio_capture.is_none(),
                    egui::Button::new(format!(
                        "Redo ({})",
                        self.shortcut_label(
                            crate::workspace::COMMANDS
                                .iter()
                                .find(|command| command.id == crate::workspace::CommandId::Redo)
                                .expect("registered command")
                        )
                    )),
                )
                .clicked();
        });
        if undo_requested {
            self.dispatch_command(crate::workspace::CommandId::Undo, ui.ctx());
        }
        if redo_requested {
            self.dispatch_command(crate::workspace::CommandId::Redo, ui.ctx());
        }
        if let Some(path) = &self.presentation_studio.project_path {
            ui.small(format!("Project: {}", path.display()));
        }
        if let Some(error) = &self.presentation_studio.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.label(&self.presentation_studio.message);
        if self.presentation_studio.job.is_some() {
            ui.horizontal(|ui| {
                if let Some(progress) = &self.presentation_studio.job_progress {
                    if let Some(fraction) = progress.fraction {
                        ui.add(egui::ProgressBar::new(fraction).desired_width(220.0));
                    } else {
                        ui.spinner();
                    }
                } else {
                    ui.spinner();
                }
                if ui.small_button("Cancel task").clicked() {
                    if let Some(job) = &self.presentation_studio.job {
                        job.control.cancel();
                    }
                }
            });
        }

        let mut playback_action = None;
        let mut scrub_to = None;
        let is_playing = self
            .presentation_studio
            .playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing());
        ui.horizontal(|ui| {
            if self.presentation_studio.playback_start.is_some() {
                ui.spinner();
                ui.label("Preparing playback…");
                if ui.button("Cancel startup").clicked() {
                    playback_action = Some("cancel");
                }
            } else if self.presentation_studio.playback.is_some() {
                if ui
                    .button(if is_playing { "Pause" } else { "Resume" })
                    .clicked()
                {
                    playback_action = Some(if is_playing { "pause" } else { "resume" });
                }
                if ui.button("Stop").clicked() {
                    playback_action = Some("stop");
                }
                if ui.button("◀ Frame").clicked() {
                    playback_action = Some("previous-frame");
                }
                if ui.button("Frame ▶").clicked() {
                    playback_action = Some("next-frame");
                }
            } else if ui.button("Play").clicked() {
                playback_action = Some("play");
            }
            ui.label(format!(
                "{:02}:{:02}.{:03} / {:.2}s",
                (self.presentation_studio.playhead / 60.0).floor() as u64,
                (self.presentation_studio.playhead % 60.0).floor() as u64,
                ((self.presentation_studio.playhead.fract() * 1000.0).round() as u64).min(999),
                self.presentation_studio.project.duration
            ));
            if let Some(playback) = &self.presentation_studio.playback {
                let (rms, peak, clipping) = playback.master_meter();
                ui.add(
                    egui::ProgressBar::new(rms.clamp(0.0, 1.0))
                        .desired_width(112.0)
                        .text(format!("Master RMS {rms:.2}")),
                );
                ui.small(format!("Peak {peak:.2}"));
                if clipping {
                    ui.colored_label(egui::Color32::LIGHT_RED, "CLIP");
                }
                ui.small(format!(
                    "Audio output underflows: {}",
                    playback.underflow_count()
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        });
        let scrub = ui.add(
            egui::Slider::new(
                &mut self.presentation_studio.playhead,
                0.0..=self.presentation_studio.project.duration,
            )
            .show_value(false),
        );
        if scrub.dragged() {
            self.presentation_studio.scrubbing = true;
        }
        if scrub.drag_stopped() {
            self.presentation_studio.scrubbing = false;
            scrub_to = Some(self.presentation_studio.playhead);
        } else if scrub.changed() && !scrub.dragged() {
            scrub_to = Some(self.presentation_studio.playhead);
        }
        match playback_action {
            Some("play") | Some("resume") => {
                self.start_presentation_playback(ui.ctx(), false);
            }
            Some("pause") => {
                if let Some(playback) = &self.presentation_studio.playback {
                    playback.set_playing(false);
                }
            }
            Some("stop") => {
                self.presentation_studio.playback = None;
                self.presentation_studio.playhead = 0.0;
            }
            Some("previous-frame") => {
                let target = self.presentation_studio.playhead
                    - 1.0 / f64::from(self.presentation_studio.project.fps);
                self.seek_presentation_playback(ui.ctx(), target);
            }
            Some("next-frame") => {
                let target = self.presentation_studio.playhead
                    + 1.0 / f64::from(self.presentation_studio.project.fps);
                self.seek_presentation_playback(ui.ctx(), target);
            }
            Some("cancel") => {
                self.presentation_studio.playback_start = None;
                self.presentation_studio.record_voiceover_after_playback = false;
            }
            _ => {}
        }
        if let Some(time) = scrub_to {
            self.seek_presentation_playback(ui.ctx(), time);
        }

        let mut refresh_audio = false;
        let mut start_meter = false;
        let mut start_recording = false;
        let mut start_playback_then_record = false;
        let mut stop_capture = false;
        let mut recover_take_index = None;
        let mut selected_device = self
            .presentation_studio
            .audio_preferences
            .selected_device_id
            .clone()
            .unwrap_or_default();
        if self.workspace_preferences.panels.voiceover {
            ui.collapsing("Voiceover recording", |ui| {
            ui.label("Microphone capture starts only after Test Input or Record Voiceover. Speaker monitoring is off. Captured audio stays local.");
            ui.weak("Record Voiceover starts timeline playback first when stopped, then begins capture from the playhead. Preview may drop frames when composition rendering is slower than real time; the audio-device clock continues.");
            ui.horizontal_wrapped(|ui| {
                let selected_name = if selected_device.is_empty() {
                    "System Default Microphone".to_owned()
                } else {
                    self.presentation_studio
                        .audio_devices
                        .iter()
                        .find(|device| device.id == selected_device)
                        .map(|device| device.name.clone())
                        .unwrap_or_else(|| "Unavailable saved microphone".into())
                };
                egui::ComboBox::from_id_salt("presentation-microphone-device")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut selected_device,
                            String::new(),
                            "System Default Microphone",
                        );
                        if let Some(saved) = self
                            .presentation_studio
                            .audio_preferences
                            .selected_device_id
                            .as_ref()
                            .filter(|saved| {
                                !self
                                    .presentation_studio
                                    .audio_devices
                                    .iter()
                                    .any(|device| &device.id == *saved)
                            })
                        {
                            ui.selectable_value(
                                &mut selected_device,
                                saved.clone(),
                                "Unavailable saved microphone (fallback to default)",
                            );
                        }
                        for device in &self.presentation_studio.audio_devices {
                            let suffix = if device.is_default { " (default)" } else { "" };
                            ui.selectable_value(
                                &mut selected_device,
                                device.id.clone(),
                                format!("{}{suffix}", device.name),
                            );
                        }
                    });
                if ui.button("Refresh Inputs").clicked() {
                    refresh_audio = true;
                }
                if self.presentation_studio.audio_capture.is_none() {
                    if ui.button("Test Input / Meter").clicked() {
                        start_meter = true;
                    }
                    if ui.button("Record Voiceover").clicked() {
                        if self
                            .presentation_studio
                            .playback
                            .as_ref()
                            .is_some_and(|playback| !playback.is_ended())
                        {
                            start_recording = true;
                        } else {
                            start_playback_then_record = true;
                        }
                    }
                } else if let Some(capture) = &self.presentation_studio.audio_capture {
                    let stop_label = if capture.is_recording() {
                        "Stop Recording"
                    } else {
                        "Stop Meter"
                    };
                    if ui.button(stop_label).clicked() {
                        stop_capture = true;
                    }
                }
            });
            if let Some(device) = self
                .presentation_studio
                .audio_devices
                .iter()
                .find(|device| device.id == selected_device)
            {
                ui.small(format!(
                    "{} • {} Hz • {} channel{} • {}",
                    device.name,
                    device.sample_rate,
                    device.channels,
                    if device.channels == 1 { "" } else { "s" },
                    device.sample_format
                ));
            }
            if let Some(error) = &self.presentation_studio.audio_preferences_error {
                ui.colored_label(egui::Color32::LIGHT_YELLOW, error);
            }
            if let Some(capture) = &self.presentation_studio.audio_capture {
                let level = capture.level();
                let duration = level.captured_frames as f64 / f64::from(capture.sample_rate());
                let status = if capture.is_recording() {
                    format!(
                        "RECORDING • {duration:.2}s • {} Hz • {} channel{}",
                        capture.sample_rate(),
                        capture.channels(),
                        if capture.channels() == 1 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "INPUT METER • {} Hz • {} channel{}",
                        capture.sample_rate(),
                        capture.channels(),
                        if capture.channels() == 1 { "" } else { "s" }
                    )
                };
                ui.colored_label(
                    if capture.is_recording() {
                        egui::Color32::LIGHT_RED
                    } else {
                        egui::Color32::LIGHT_GREEN
                    },
                    status,
                );
                ui.add(
                    egui::ProgressBar::new(level.rms.clamp(0.0, 1.0))
                        .desired_width(ui.available_width())
                        .text(format!("RMS {:.3} • Peak {:.3}", level.rms, level.peak)),
                );
                if level.clipping {
                    ui.colored_label(egui::Color32::LIGHT_RED, "Clipping detected. Reduce microphone gain.");
                }
                if let Some(latency) = level.reported_input_latency_ms {
                    ui.small(format!("Driver-reported input callback delay: {latency:.1} ms; not compensated."));
                }
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            if !self.presentation_studio.recoverable_takes.is_empty() {
                ui.separator();
                ui.label("Interrupted takes");
                for (index, take) in self
                    .presentation_studio
                    .recoverable_takes
                    .iter()
                    .enumerate()
                {
                    ui.horizontal(|ui| {
                        ui.small(format!(
                            "{} • starts at {:.3}s • {} Hz / {} ch",
                            take.partial_path.display(),
                            take.timeline_start_seconds,
                            take.sample_rate,
                            take.channels
                        ));
                        if ui.small_button("Recover and insert").clicked() {
                            recover_take_index = Some(index);
                        }
                    });
                }
            }
        });
        }
        let selected_device = if selected_device.is_empty() {
            None
        } else {
            Some(selected_device)
        };
        if selected_device
            != self
                .presentation_studio
                .audio_preferences
                .selected_device_id
        {
            self.presentation_studio
                .audio_preferences
                .selected_device_id = selected_device;
            match crate::audio_input::save_preferences(&self.presentation_studio.audio_preferences)
            {
                Ok(()) => self.presentation_studio.audio_preferences_error = None,
                Err(error) => {
                    self.presentation_studio.audio_preferences_error =
                        Some(format!("Could not save microphone preference: {error:#}"));
                }
            }
        }
        if refresh_audio {
            self.presentation_refresh_audio_devices();
        }
        if start_meter {
            self.presentation_start_audio(false);
        }
        if start_recording {
            self.dispatch_command(crate::workspace::CommandId::RecordVoiceover, ui.ctx());
        }
        if start_playback_then_record {
            self.dispatch_command(crate::workspace::CommandId::RecordVoiceover, ui.ctx());
        }
        if stop_capture {
            self.dispatch_command(crate::workspace::CommandId::StopRecording, ui.ctx());
        }
        if let Some(index) = recover_take_index {
            self.presentation_recover_take(index);
        }

        while let Ok((key, source_id, result)) = self.presentation_studio.thumbnail_rx.try_recv() {
            self.presentation_studio.thumbnail_worker_active = false;
            self.presentation_studio.thumbnail_requested.remove(&key);
            let is_current = self
                .presentation_studio
                .project
                .sources
                .iter()
                .find(|source| source.id == source_id)
                .and_then(presentation_thumbnail_key)
                .is_some_and(|current| current == key);
            if !is_current {
                continue;
            }
            match result {
                Ok(bytes) => match image::load_from_memory(&bytes) {
                    Ok(image) => {
                        let image = image.thumbnail(160, 90).to_rgba8();
                        let color = egui::ColorImage::from_rgba_unmultiplied(
                            [image.width() as usize, image.height() as usize],
                            image.as_raw(),
                        );
                        let texture = ui.ctx().load_texture(
                            format!("presentation-media-{key}"),
                            color,
                            egui::TextureOptions::LINEAR,
                        );
                        if self.presentation_studio.thumbnail_cache.len() >= 64 {
                            while let Some(oldest) =
                                self.presentation_studio.thumbnail_lru.pop_front()
                            {
                                if self
                                    .presentation_studio
                                    .thumbnail_cache
                                    .remove(&oldest)
                                    .is_some()
                                {
                                    break;
                                }
                            }
                        }
                        self.presentation_studio
                            .thumbnail_lru
                            .push_back(key.clone());
                        self.presentation_studio
                            .thumbnail_cache
                            .insert(key, texture);
                    }
                    Err(_) => {
                        self.presentation_studio.thumbnail_failed.insert(key);
                    }
                },
                Err(_) => {
                    self.presentation_studio.thumbnail_failed.insert(key);
                }
            }
        }

        let before_frame_edit = self.presentation_studio.project.clone();
        let mut timeline_seek_after_edit = None;
        let mut source_preview_request = None;
        let mut pending_proxy_generation: Option<(String, PathBuf)> = None;
        let mut pending_speech_install = None;
        let mut pending_subtitle_transcription = None;
        let mut pending_subtitle_import: Option<PathBuf> = None;
        let mut pending_subtitle_export: Option<(PathBuf, crate::subtitles::SubtitleFormat)> = None;
        let mut pending_command_actions = Vec::new();
        ui.add_enabled_ui(self.presentation_studio.audio_capture.is_none(), |ui| {
        let project = &mut self.presentation_studio.project;
        if self.workspace_preferences.panels.composition {
        ui.collapsing("Composition", |ui| {
            ui.horizontal(|ui| {
                for (label, width, height) in [
                    ("Landscape 1080p", 1920, 1080),
                    ("Vertical 9:16", 1080, 1920),
                    ("Square", 1080, 1080),
                ] {
                    if ui
                        .selectable_label(project.width == width && project.height == height, label)
                        .clicked()
                    {
                        project.width = width;
                        project.height = height;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Size");
                ui.add(
                    egui::DragValue::new(&mut project.width)
                        .range(2..=8192)
                        .prefix("W "),
                );
                ui.label("Height");
                ui.add(
                    egui::DragValue::new(&mut project.height)
                        .range(2..=8192)
                        .prefix("H "),
                );
                ui.label("FPS");
                ui.add(egui::DragValue::new(&mut project.fps).range(1..=240));
                ui.label("Duration");
                ui.add(
                    egui::DragValue::new(&mut project.duration)
                        .range(0.1..=86_400.0)
                        .speed(0.1)
                        .suffix(" s"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Export quality");
                egui::ComboBox::from_id_salt("composition-export-preset")
                    .selected_text(project.export_preset.label())
                    .show_ui(ui, |ui| {
                        for preset in [
                            crate::composition::CompositionExportPreset::Draft,
                            crate::composition::CompositionExportPreset::Balanced,
                            crate::composition::CompositionExportPreset::HighQuality,
                        ] {
                            ui.selectable_value(
                                &mut project.export_preset,
                                preset,
                                preset.label(),
                            );
                        }
                    });
                ui.weak("H.264/AAC; faster presets trade size/quality for encode speed");
            });
            let mut use_export_range = project.export_range.is_some();
            if ui
                .checkbox(&mut use_export_range, "Export In/Out range")
                .changed()
            {
                if use_export_range {
                    if let Err(error) = project.set_export_range(0.0, project.duration) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                } else {
                    project.clear_export_range();
                }
            }
            if let Some(range) = project.export_range {
                let mut start = range.start;
                let mut end = range.end;
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("In");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut start)
                                .range(0.0..=project.duration)
                                .speed(0.05)
                                .suffix(" s"),
                        )
                        .changed();
                    if ui.small_button("Set In to playhead").clicked() {
                        start = self.presentation_studio.playhead;
                        changed = true;
                    }
                    ui.label("Out");
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut end)
                                .range(0.0..=project.duration)
                                .speed(0.05)
                                .suffix(" s"),
                        )
                        .changed();
                    if ui.small_button("Set Out to playhead").clicked() {
                        end = self.presentation_studio.playhead;
                        changed = true;
                    }
                });
                if changed {
                    if let Err(error) = project.set_export_range(start, end) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                }
                ui.small(format!("Export duration: {:.3}s", project.export_duration()));
            }
        });
        }

        let mut add_track_kind = None;
        let mut remove_track_id = None;
        let mut reorder_track = None;
        if self.workspace_preferences.panels.tracks {
        ui.collapsing(format!("Tracks ({})", project.tracks.len()), |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add Visual Track").clicked() {
                    add_track_kind = Some(crate::composition::TrackKind::Visual);
                }
                if ui.button("Add Audio Track").clicked() {
                    add_track_kind = Some(crate::composition::TrackKind::Audio);
                }
            });
            let mut tracks: Vec<_> = project.tracks.iter_mut().collect();
            tracks.sort_by_key(|track| (track.kind, track.order, track.id.clone()));
            for track in tracks {
                ui.horizontal(|ui| {
                    let occupied = project
                        .clips
                        .iter()
                        .any(|clip| clip.track_id.as_deref() == Some(track.id.as_str()));
                    ui.add_enabled_ui(!occupied, |ui| {
                        if ui.small_button("Up").clicked() {
                            reorder_track = Some((track.id.clone(), -1));
                        }
                    });
                    if ui.small_button("Down").clicked() {
                        reorder_track = Some((track.id.clone(), 1));
                    }
                    ui.text_edit_singleline(&mut track.name);
                    ui.small(format!("{:?}", track.kind));
                    ui.checkbox(&mut track.enabled, "Enabled");
                    if track.kind == crate::composition::TrackKind::Audio {
                        ui.checkbox(&mut track.muted, "Mute");
                        ui.checkbox(&mut track.solo, "Solo");
                        ui.add_enabled_ui(!track.locked, |ui| {
                            ui.label("Gain");
                            ui.add(
                                egui::DragValue::new(&mut track.gain)
                                    .range(0.0..=16.0)
                                    .speed(0.02)
                                    .suffix("×"),
                            );
                        });
                    }
                    if ui.small_button("Remove").clicked() {
                        remove_track_id = Some(track.id.clone());
                    }
                });
            }
        });
        }
        if let Some(kind) = add_track_kind {
            let count = project.tracks.iter().filter(|track| track.kind == kind).count() + 1;
            if let Err(error) = project.add_track(kind, format!("{kind:?} {count}")) {
                self.presentation_studio.error = Some(format!("{error:#}"));
            }
        }
        if let Some(track_id) = remove_track_id {
            if let Err(error) = project.remove_track(&track_id) {
                self.presentation_studio.error = Some(format!("{error:#}"));
            }
        }
        if let Some((track_id, direction)) = reorder_track {
            if let Some(index) = project.tracks.iter().position(|track| track.id == track_id) {
                let kind = project.tracks[index].kind;
                let neighbor = project
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| track.kind == kind)
                    .min_by_key(|(_, track)| {
                        (track.order - (project.tracks[index].order + direction)).abs()
                    })
                    .map(|(index, _)| index);
                if let Some(neighbor) = neighbor.filter(|neighbor| *neighbor != index) {
                    let order = project.tracks[index].order;
                    project.tracks[index].order = project.tracks[neighbor].order;
                    project.tracks[neighbor].order = order;
                }
            }
        }

        if self.presentation_studio.source_monitor_source
            != self.presentation_studio.selected_source
        {
            self.presentation_studio.source_monitor_source =
                self.presentation_studio.selected_source.clone();
            self.presentation_studio.source_monitor_time = 0.0;
            self.presentation_studio.source_in = 0.0;
            self.presentation_studio.source_preview_texture = None;
            self.presentation_studio.source_out = self
                .presentation_studio
                .selected_source
                .as_deref()
                .and_then(|id| project.sources.iter().find(|source| source.id == id))
                .map_or(5.0, |source| {
                    if source.still_image || source.solid_color.is_some() {
                        5.0f64.min(project.duration)
                    } else {
                        source.duration
                    }
                });
        }
        let mut pending_add_clip: Option<(String, crate::composition::ClipKind)> = None;
        let mut pending_add_linked: Option<String> = None;
        let mut pending_relink: Option<(String, PathBuf)> = None;
        let mut pending_source_insert: Option<SourceInsertMode> = None;
        let mut pending_proxy_removal: Option<(String, bool)> = None;
        let mut remove_unused_media = false;
        if self.workspace_preferences.panels.captions {
        ui.collapsing("Subtitles / captions", |ui| {
            ui.label("Captions become ordinary editable text clips on the composition timeline. Transcription runs locally and never uploads audio.");
            ui.horizontal(|ui| {
                ui.label("Language");
                ui.add(
                    egui::TextEdit::singleline(&mut self.presentation_studio.transcription_language)
                        .desired_width(92.0)
                        .hint_text("auto / en / ja"),
                );
                ui.small("`auto` or a Whisper language code");
            });
            egui::ComboBox::from_id_salt("presentation-caption-style")
                .selected_text(
                    project
                        .title_presets
                        .iter()
                        .find(|preset| preset.id == self.presentation_studio.selected_caption_preset)
                        .map(|preset| preset.name.as_str())
                        .unwrap_or("Short-form caption"),
                )
                .show_ui(ui, |ui| {
                    for preset in &project.title_presets {
                        ui.selectable_value(
                            &mut self.presentation_studio.selected_caption_preset,
                            preset.id.clone(),
                            &preset.name,
                        );
                    }
                });

            let speech_status = crate::subtitles::setup_status();
            ui.horizontal_wrapped(|ui| {
                if speech_status.runtime_installed {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "Local CPU recognizer installed");
                }
                if ui
                    .add_enabled(
                        self.presentation_studio.job.is_none(),
                        egui::Button::new(if speech_status.runtime_installed {
                            "Verify / repair recognizer"
                        } else {
                            "Install CPU recognizer (~8 MB)"
                        }),
                    )
                    .clicked()
                {
                    pending_speech_install = Some(crate::subtitles::SpeechDependency::Runtime);
                }
                if speech_status.model_installed {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "Multilingual Base model present");
                }
                if ui
                    .add_enabled(
                        self.presentation_studio.job.is_none(),
                        egui::Button::new(if speech_status.model_installed {
                            "Verify / repair Base model"
                        } else {
                            "Download Base model (~148 MB)"
                        }),
                    )
                    .on_hover_text("Verification is local. If repair is needed, the model is explicitly downloaded and stored for offline use in your application data; it is not part of every installer.")
                    .clicked()
                {
                    pending_speech_install = Some(
                        crate::subtitles::SpeechDependency::MultilingualBaseModel,
                    );
                }
            });
            let selected_clip = self
                .presentation_studio
                .selected_clip
                .as_deref()
                .and_then(|id| project.clips.iter().find(|clip| clip.id == id));
            let selected_audio_source = selected_clip.and_then(|clip| {
                project
                    .sources
                    .iter()
                    .find(|source| clip.source_id.as_deref() == Some(source.id.as_str()) && source.has_audio)
                    .map(|source| (clip, source))
            });
            if let Some((clip, source)) = selected_audio_source {
                ui.small(format!(
                    "Source: {} · selected clip only · source trim and forward speed applied · timeline start {:.2}s",
                    source.display_name,
                    clip.timeline_start
                ));
                ui.add_enabled_ui(
                    speech_status.ready()
                        && self.presentation_studio.job.is_none()
                        && clip.source_time_mode == crate::composition::SourceTimeMode::Forward,
                    |ui| {
                        if ui.button("Generate subtitles from selected clip").clicked() {
                            pending_subtitle_transcription = Some(clip.id.clone());
                        }
                    },
                );
                if clip.source_time_mode != crate::composition::SourceTimeMode::Forward {
                    ui.small("Reverse/freeze clips are not offered because recognition time would not map to normal speech reliably.");
                }
            } else {
                ui.small("Select an audio or video timeline clip with an audio stream to choose exactly what is transcribed.");
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.presentation_studio.job.is_none(), egui::Button::new("Import SRT / WebVTT…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Import Subtitles")
                        .add_filter("Subtitles", &["srt", "vtt"])
                        .pick_file()
                    {
                        pending_subtitle_import = Some(path);
                    }
                }
                if ui
                    .add_enabled(!crate::subtitles::cues_from_project(project).is_empty(), egui::Button::new("Export SRT…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Export SubRip subtitles")
                        .add_filter("SubRip subtitles", &["srt"])
                        .set_file_name("captions.srt")
                        .save_file()
                    {
                        pending_subtitle_export = Some((path, crate::subtitles::SubtitleFormat::Srt));
                    }
                }
                if ui
                    .add_enabled(!crate::subtitles::cues_from_project(project).is_empty(), egui::Button::new("Export WebVTT…"))
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Export WebVTT subtitles")
                        .add_filter("WebVTT subtitles", &["vtt"])
                        .set_file_name("captions.vtt")
                        .save_file()
                    {
                        pending_subtitle_export = Some((path, crate::subtitles::SubtitleFormat::WebVtt));
                    }
                }
            });
        });
        }
        if self.workspace_preferences.panels.media {
        ui.collapsing(format!("Media sources ({})", project.sources.len()), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.presentation_studio.media_search)
                        .hint_text("Search media name or path"),
                );
                ui.checkbox(
                    &mut project.use_proxies_for_preview,
                    "Use valid proxies for preview",
                )
                .on_hover_text("Final MP4 exports always use original media.");
                let unused_count = project
                    .sources
                    .iter()
                    .filter(|source| {
                        !project
                            .clips
                            .iter()
                            .any(|clip| clip.source_id.as_deref() == Some(source.id.as_str()))
                    })
                    .count();
                if ui
                    .add_enabled(
                        unused_count > 0,
                        egui::Button::new(format!("Remove unused ({unused_count})")),
                    )
                    .on_hover_text("Removes unused entries from this project only; source files stay on disk.")
                    .clicked()
                {
                    remove_unused_media = true;
                }
            });
            let query = self.presentation_studio.media_search.trim().to_lowercase();
            for source in project.sources.iter().filter(|source| {
                let path = source
                    .nested_composition_path
                    .as_deref()
                    .unwrap_or(source.path.as_path());
                query.is_empty()
                    || source.display_name.to_lowercase().contains(&query)
                    || path.to_string_lossy().to_lowercase().contains(&query)
            }) {
                ui.horizontal_wrapped(|ui| {
                    if source.has_video
                        && source.solid_color.is_none()
                        && source.nested_composition_path.is_none()
                    {
                        if let Some(key) = presentation_thumbnail_key(source) {
                            let texture = self
                                .presentation_studio
                                .thumbnail_cache
                                .get(&key)
                                .cloned();
                            if let Some(texture) = texture {
                                if let Some(index) = self
                                    .presentation_studio
                                    .thumbnail_lru
                                    .iter()
                                    .position(|item| item == &key)
                                {
                                    self.presentation_studio.thumbnail_lru.remove(index);
                                }
                                self.presentation_studio
                                    .thumbnail_lru
                                    .push_back(key.clone());
                                ui.image((texture.id(), egui::vec2(88.0, 50.0)));
                            } else {
                                if !self
                                    .presentation_studio
                                    .thumbnail_requested
                                    .contains(&key)
                                    && !self
                                        .presentation_studio
                                        .thumbnail_failed
                                        .contains(&key)
                                    && self.presentation_studio.thumbnail_queue.len() < 128
                                {
                                    self.presentation_studio
                                        .thumbnail_requested
                                        .insert(key.clone());
                                    self.presentation_studio.thumbnail_queue.push_back((
                                        key,
                                        source.id.clone(),
                                        source.path.clone(),
                                    ));
                                }
                                ui.add_sized(
                                    egui::vec2(88.0, 50.0),
                                    egui::Label::new(if source.path.is_file() {
                                        "Loading preview…"
                                    } else {
                                        "Offline"
                                    }),
                                );
                            }
                        }
                    }
                    let selected = ui.selectable_value(
                        &mut self.presentation_studio.selected_source,
                        Some(source.id.clone()),
                        &source.display_name,
                    );
                    ui.dnd_drag_source(
                        egui::Id::new(("presentation-media-source", source.id.as_str())),
                        PresentationMediaDrag {
                            source_id: source.id.clone(),
                        },
                        |ui| {
                            ui.small("⠿ Drag to timeline track");
                        },
                    );
                    if selected.changed() {
                        self.presentation_studio.source_preview_texture = None;
                    }
                    let media_kind = if let Some(shape) = source.generated_shape {
                        match shape.kind {
                            crate::composition::GeneratedShapeKind::Rectangle => "Rectangle",
                            crate::composition::GeneratedShapeKind::Ellipse => "Ellipse",
                            crate::composition::GeneratedShapeKind::Line => "Line",
                        }
                    } else if source.solid_color.is_some() {
                        "Solid"
                    } else if source.nested_composition_path.is_some() {
                        "Nested composition"
                    } else if source.still_image {
                        "Still image"
                    } else if source.has_video && source.has_audio {
                        "Video + audio"
                    } else if source.has_video {
                        "Video"
                    } else {
                        "Audio"
                    };
                    let media_duration = if source.still_image || source.solid_color.is_some() {
                        "still".to_owned()
                    } else {
                        format!("{:.2}s", source.duration)
                    };
                    let dimensions = source
                        .width
                        .zip(source.height)
                        .map(|(width, height)| format!("  {width}x{height}"))
                        .unwrap_or_default();
                    let usage = project
                        .clips
                        .iter()
                        .filter(|clip| clip.source_id.as_deref() == Some(source.id.as_str()))
                        .count();
                    ui.small(format!("{media_kind} | {media_duration}{dimensions} | {usage} clip(s)"));
                    if source.has_video
                        && !source.still_image
                        && source.solid_color.is_none()
                        && source.nested_composition_path.is_none()
                    {
                        let proxy_status = project.source_proxy_status(&source.id);
                        let status_text = match proxy_status {
                            None | Some(crate::composition::VideoProxyStatus::Missing) => {
                                "No proxy".to_owned()
                            }
                            Some(crate::composition::VideoProxyStatus::Ready) => source
                                .proxy_metadata
                                .as_ref()
                                .map(|proxy| format!(
                                    "Proxy ready · {}x{} {}",
                                    proxy.width, proxy.height, proxy.video_codec
                                ))
                                .unwrap_or_else(|| "Proxy ready".into()),
                            Some(crate::composition::VideoProxyStatus::ReadyOriginalOffline) => {
                                "Proxy ready · original offline (export unavailable)".into()
                            }
                            Some(crate::composition::VideoProxyStatus::ProxyOffline) => {
                                "Proxy file missing".into()
                            }
                            Some(crate::composition::VideoProxyStatus::Stale) => {
                                "Proxy stale · regenerate".into()
                            }
                            Some(crate::composition::VideoProxyStatus::MetadataMissing) => {
                                "Legacy proxy · regenerate to validate".into()
                            }
                        };
                        ui.small(status_text);
                        if self.presentation_studio.job.is_none() {
                            if source.path.is_file()
                                && ui.small_button("Generate / Replace proxy…").clicked()
                            {
                                let stem = source
                                    .path
                                    .file_stem()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("video");
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_title(format!("Generate proxy for {}", source.display_name))
                                    .add_filter("MP4 video proxy", &["mp4"])
                                    .set_file_name(format!("{stem}.proxy.mp4"))
                                    .save_file()
                                {
                                    pending_proxy_generation = Some((source.id.clone(), path));
                                }
                            }
                            if source.proxy_path.is_some() {
                                let owned = source
                                    .proxy_metadata
                                    .as_ref()
                                    .is_some_and(|proxy| proxy.generated_by_renderer);
                                if ui
                                    .small_button(if owned { "Delete generated proxy" } else { "Detach proxy" })
                                    .clicked()
                                {
                                    pending_proxy_removal = Some((source.id.clone(), owned));
                                }
                            }
                        }
                    }
                    let source_path = source
                        .nested_composition_path
                        .as_deref()
                        .unwrap_or(source.path.as_path());
                    if source.solid_color.is_none() && !source_path.is_file() {
                        ui.colored_label(egui::Color32::LIGHT_RED, "Missing media");
                        if ui.small_button("Relink…").clicked() {
                            let mut dialog = rfd::FileDialog::new().set_title(format!(
                                "Relink {}",
                                source.display_name
                            ));
                            if source.nested_composition_path.is_some() {
                                dialog = dialog.add_filter("Sono Composition", &["sonocomp"]);
                            } else if source.has_video && source.has_audio {
                                dialog = dialog.add_filter("Video with audio", &["mp4", "mov", "mkv", "webm"]);
                            } else if source.has_video {
                                dialog = dialog.add_filter("Video", &["mp4", "mov", "mkv", "webm"]);
                            } else {
                                dialog = dialog.add_filter("Audio", &["wav", "mp3", "m4a", "aac", "flac", "ogg"]);
                            }
                            if let Some(path) = dialog.pick_file() {
                                pending_relink = Some((source.id.clone(), path));
                            }
                        }
                    } else if source.nested_composition_path.is_some()
                        && ui.small_button("Replace composition...").clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(format!("Replace {}", source.display_name))
                            .add_filter("Sono Composition", &["sonocomp"])
                            .pick_file()
                        {
                            pending_relink = Some((source.id.clone(), path));
                        }
                    } else if source.nested_composition_path.is_none()
                        && source.solid_color.is_none()
                        && ui.small_button("Replace…").clicked()
                    {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title(format!("Replace {}", source.display_name));
                        if source.has_video && source.has_audio {
                            dialog = dialog.add_filter("Video with audio", &["mp4", "mov", "mkv", "webm"]);
                        } else if source.has_video {
                            dialog = dialog.add_filter("Video", &["mp4", "mov", "mkv", "webm"]);
                        } else {
                            dialog = dialog.add_filter("Audio", &["wav", "mp3", "m4a", "aac", "flac", "ogg"]);
                        }
                        if let Some(path) = dialog.pick_file() {
                            pending_relink = Some((source.id.clone(), path));
                        }
                    }
                    ui.add_enabled_ui(self.presentation_studio.job.is_none(), |ui| {
                        if source.has_video && ui.small_button("Add video clip").clicked() {
                            pending_add_clip =
                                Some((source.id.clone(), crate::composition::ClipKind::Video));
                        }
                        if source.has_audio && ui.small_button("Add audio clip").clicked() {
                            pending_add_clip =
                                Some((source.id.clone(), crate::composition::ClipKind::Audio));
                        }
                        if source.has_video
                            && source.has_audio
                            && ui.small_button("Add linked A/V").clicked()
                        {
                            pending_add_linked = Some(source.id.clone());
                        }
                    });
                });
            }
        });
        }
        if !self.presentation_studio.thumbnail_worker_active {
            while let Some((key, source_id, path)) =
                self.presentation_studio.thumbnail_queue.pop_front()
            {
                let still_current = project
                    .sources
                    .iter()
                    .find(|source| source.id == source_id)
                    .and_then(presentation_thumbnail_key)
                    .is_some_and(|current| current == key);
                if !still_current {
                    self.presentation_studio.thumbnail_requested.remove(&key);
                    continue;
                }
                self.presentation_studio.thumbnail_worker_active = true;
                let sender = self.presentation_studio.thumbnail_tx.clone();
                let context = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = crate::composition::media_thumbnail_png(&path, 160, 90)
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send((key, source_id, result));
                    context.request_repaint();
                });
                break;
            }
        }
        if remove_unused_media {
            let removed = project.remove_unused_sources();
            if removed > 0 {
                if self
                    .presentation_studio
                    .selected_source
                    .as_ref()
                    .is_some_and(|selected| !project.sources.iter().any(|source| &source.id == selected))
                {
                    self.presentation_studio.selected_source = None;
                    self.presentation_studio.source_monitor_source = None;
                    self.presentation_studio.source_preview_texture = None;
                }
                self.presentation_studio.message = format!(
                    "Removed {removed} unused media entr{} from the project. Source files were not deleted.",
                    if removed == 1 { "y" } else { "ies" }
                );
            }
        }
        if let Some((source_id, delete_file)) = pending_proxy_removal {
            let result = if delete_file {
                project
                    .delete_generated_source_proxy(&source_id)
                    .map(|deleted| {
                        if deleted {
                            "Deleted generated proxy and detached it from the media source."
                        } else {
                            "Generated proxy file was already missing; detached it from the media source."
                        }
                    })
            } else {
                project
                    .clear_source_proxy(&source_id)
                    .map(|()| "Detached the proxy reference; its file was left on disk.")
            };
            match result {
                Ok(message) => {
                    self.presentation_studio.message = message.into();
                    self.presentation_studio.error = None;
                }
                Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
            }
        }
        if self.workspace_preferences.panels.source_monitor {
        ui.collapsing("Source monitor", |ui| {
            if let Some(source) = self
                .presentation_studio
                .selected_source
                .as_deref()
                .and_then(|id| project.sources.iter().find(|source| source.id == id))
            {
                ui.label(format!("Source: {}", source.display_name));
                let frame = 1.0 / f64::from(project.fps);
                let still = source.still_image || source.solid_color.is_some();
                let source_limit = if still {
                    project.duration
                } else {
                    source.duration
                };
                self.presentation_studio.source_in = self
                    .presentation_studio
                    .source_in
                    .clamp(0.0, (source_limit - frame).max(0.0));
                let minimum_out = (self.presentation_studio.source_in + frame).min(source_limit);
                self.presentation_studio.source_out = self
                    .presentation_studio
                    .source_out
                    .clamp(minimum_out, source_limit);
                if source.has_video {
                    if still {
                        ui.label("Still source: source time is fixed at 0; Out sets the inserted duration.");
                    } else {
                        ui.add(
                            egui::Slider::new(
                                &mut self.presentation_studio.source_monitor_time,
                                0.0..=(source.duration - frame).max(0.0),
                            )
                            .text("Source time"),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label("In");
                        ui.add(
                            egui::DragValue::new(&mut self.presentation_studio.source_in)
                                .range(0.0..=(source_limit - frame).max(0.0))
                                .speed(0.05)
                                .suffix(" s"),
                        );
                        if ui.button("Set In").clicked() {
                            self.presentation_studio.source_in =
                                self.presentation_studio.source_monitor_time;
                        }
                        ui.label("Out");
                        ui.add(
                            egui::DragValue::new(&mut self.presentation_studio.source_out)
                                .range(
                                    minimum_out..=source_limit,
                                )
                                .speed(0.05)
                                .suffix(" s"),
                        );
                        if ui.button("Set Out").clicked() {
                            self.presentation_studio.source_out = self
                                .presentation_studio
                                .source_monitor_time
                                .max(minimum_out)
                                .min(source_limit);
                        }
                        if ui
                            .add_enabled(
                                self.presentation_studio.job.is_none(),
                                egui::Button::new("Preview Source"),
                            )
                            .clicked()
                        {
                            source_preview_request = Some((
                                source.id.clone(),
                                self.presentation_studio.source_monitor_time,
                            ));
                        }
                    });
                } else {
                    ui.label("Audio-only source. Use In/Out to insert a trimmed audio clip.");
                    ui.horizontal(|ui| {
                        ui.label("In");
                        ui.add(
                            egui::DragValue::new(&mut self.presentation_studio.source_in)
                                .range(0.0..=(source.duration - frame).max(0.0))
                                .speed(0.05)
                                .suffix(" s"),
                        );
                        ui.label("Out");
                        ui.add(
                            egui::DragValue::new(&mut self.presentation_studio.source_out)
                                .range(
                                    minimum_out..=source_limit,
                                )
                                .speed(0.05)
                                .suffix(" s"),
                        );
                    });
                }
                egui::ComboBox::from_id_salt("source-edit-operation")
                    .selected_text(match self.presentation_studio.source_edit_operation {
                        SourceEditOperation::PlaceAtPlayhead => "Place at playhead",
                        SourceEditOperation::InsertRipple => "Insert and ripple all tracks",
                        SourceEditOperation::OverwriteTrack => "Overwrite first unlocked track",
                        SourceEditOperation::Append => "Append at end",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.presentation_studio.source_edit_operation,
                            SourceEditOperation::PlaceAtPlayhead,
                            "Place at playhead",
                        );
                        ui.selectable_value(
                            &mut self.presentation_studio.source_edit_operation,
                            SourceEditOperation::InsertRipple,
                            "Insert and ripple all tracks",
                        );
                        ui.selectable_value(
                            &mut self.presentation_studio.source_edit_operation,
                            SourceEditOperation::OverwriteTrack,
                            "Overwrite first unlocked compatible track",
                        );
                        ui.selectable_value(
                            &mut self.presentation_studio.source_edit_operation,
                            SourceEditOperation::Append,
                            "Append at end",
                        );
                    });
                ui.horizontal_wrapped(|ui| {
                    if source.has_video && ui.button("Use video In/Out").clicked() {
                        pending_source_insert = Some(SourceInsertMode::Video);
                    }
                    if source.has_audio && ui.button("Use audio In/Out").clicked() {
                        pending_source_insert = Some(SourceInsertMode::Audio);
                    }
                    if source.has_video
                        && source.has_audio
                        && ui.button("Use linked A/V In/Out").clicked()
                    {
                        pending_source_insert = Some(SourceInsertMode::LinkedAv);
                    }
                });
                if let Some(texture) = &self.presentation_studio.source_preview_texture {
                    let max = egui::vec2(ui.available_width().max(1.0), 220.0);
                    let size = texture.size_vec2();
                    let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
                    ui.image((texture.id(), size * scale));
                } else if source.has_video {
                    ui.weak("Choose Preview Source to render the selected source at its own time position.");
                }
            } else {
                ui.weak("Select a source in the media bin.");
            }
        });
        }
        if let Some((source_id, path)) = pending_relink {
            let before = project.clone();
            let is_nested = project
                .sources
                .iter()
                .find(|source| source.id == source_id)
                .is_some_and(|source| source.nested_composition_path.is_some());
            let relink_result = if is_nested {
                project.relink_nested_composition_source(&source_id, &path)
            } else {
                project.relink_source(&source_id, &path)
            };
            match relink_result {
                Ok(()) => {
                    self.presentation_studio.edit_history.remember(before);
                    self.presentation_studio.message = format!(
                        "Relinked media source to {}. Every clip that uses this source is repaired.",
                        path.display()
                    );
                    self.presentation_studio.error = None;
                }
                Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
            }
        }
        if let Some((source_id, kind)) = pending_add_clip {
            match project.add_clip(&source_id, kind, self.presentation_studio.playhead) {
                Ok(id) => {
                    self.presentation_studio.selected_source = Some(source_id);
                    self.presentation_studio.selected_clip = Some(id);
                }
                Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
            }
        }
        if let Some(source_id) = pending_add_linked {
            match project.add_linked_av_clip(&source_id, self.presentation_studio.playhead) {
                Ok((video_id, _audio_id)) => {
                    self.presentation_studio.selected_source = Some(source_id);
                    self.presentation_studio.selected_clip = Some(video_id);
                    self.presentation_studio.error = None;
                }
                Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
            }
        }
        if let Some(mode) = pending_source_insert {
            if let Some(source_id) = self.presentation_studio.selected_source.clone() {
                let source_in = self.presentation_studio.source_in;
                let source_out = self.presentation_studio.source_out;
                let timeline_start = self.presentation_studio.playhead;
                let operation = self.presentation_studio.source_edit_operation;
                let result = match (operation, mode) {
                    (SourceEditOperation::PlaceAtPlayhead, SourceInsertMode::Video) => project
                        .add_clip_from_range(&source_id, crate::composition::ClipKind::Video, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::PlaceAtPlayhead, SourceInsertMode::Audio) => project
                        .add_clip_from_range(&source_id, crate::composition::ClipKind::Audio, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::PlaceAtPlayhead, SourceInsertMode::LinkedAv) => project
                        .add_linked_av_range(&source_id, timeline_start, source_in, source_out)
                        .map(|(video, audio)| (video, Some(audio))),
                    (SourceEditOperation::InsertRipple, SourceInsertMode::Video) => project
                        .insert_clip_from_range(&source_id, crate::composition::ClipKind::Video, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::InsertRipple, SourceInsertMode::Audio) => project
                        .insert_clip_from_range(&source_id, crate::composition::ClipKind::Audio, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::InsertRipple, SourceInsertMode::LinkedAv) => project
                        .insert_linked_av_range(&source_id, timeline_start, source_in, source_out)
                        .map(|(video, audio)| (video, Some(audio))),
                    (SourceEditOperation::OverwriteTrack, SourceInsertMode::Video) => project
                        .overwrite_clip_from_range(&source_id, crate::composition::ClipKind::Video, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::OverwriteTrack, SourceInsertMode::Audio) => project
                        .overwrite_clip_from_range(&source_id, crate::composition::ClipKind::Audio, timeline_start, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::OverwriteTrack, SourceInsertMode::LinkedAv) => project
                        .overwrite_linked_av_range(&source_id, timeline_start, source_in, source_out)
                        .map(|(video, audio)| (video, Some(audio))),
                    (SourceEditOperation::Append, SourceInsertMode::Video) => project
                        .append_clip_from_range(&source_id, crate::composition::ClipKind::Video, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::Append, SourceInsertMode::Audio) => project
                        .append_clip_from_range(&source_id, crate::composition::ClipKind::Audio, source_in, source_out, None)
                        .map(|id| (id, None)),
                    (SourceEditOperation::Append, SourceInsertMode::LinkedAv) => project
                        .append_linked_av_range(&source_id, source_in, source_out)
                        .map(|(video, audio)| (video, Some(audio))),
                };
                match result {
                    Ok((clip_id, _linked_audio_id)) => {
                        self.presentation_studio.selected_clip = Some(clip_id);
                        self.presentation_studio.error = None;
                    }
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
        }

        if self.workspace_preferences.panels.timeline {
        ui.separator();
        let mut request_preview = false;
        ui.horizontal(|ui| {
            ui.label("Playhead");
            ui.add(
                egui::Slider::new(
                    &mut self.presentation_studio.playhead,
                    0.0..=project.duration,
                )
                .show_value(true)
                .suffix(" s"),
            );
            if self.presentation_studio.job.is_none() && ui.button("Preview / Seek").clicked() {
                request_preview = true;
            }
        });
        if request_preview {
            // The project snapshot is captured before the asynchronous worker
            // starts; it never borrows live editor state.
            let project_snapshot = project.clone();
            let time = self
                .presentation_studio
                .playhead
                .clamp(0.0, project.duration);
            let control = ExportControl::default();
            let worker_control = control.clone();
            let (tx, result) = mpsc::sync_channel(1);
            let wake = ui.ctx().clone();
            let worker = std::thread::spawn(move || {
                let outcome = catch_export_panic(|| {
                    crate::composition::preview_png(&project_snapshot, time, &worker_control)
                })
                .map(|png| PresentationJobOutcome::Preview { time, png })
                .map_err(|error| format!("{error:#}"));
                let _ = tx.send(outcome);
                wake.request_repaint();
            });
            self.presentation_studio.job = Some(PresentationJob {
                result,
                progress: None,
                worker,
                control,
            });
            self.presentation_studio.error = None;
            self.presentation_studio.message = format!("Rendering preview at {time:.3}s…");
        }

        ui.label("Timeline  Eselect a clip, then split, duplicate, trim, transform, or edit its audio/text properties.");
        let timeline_playhead_before = self.presentation_studio.playhead;
        if let Some(error) = presentation_timeline(
            ui,
            project,
            &mut self.presentation_studio.selected_clip,
            &mut self.presentation_studio.selected_clips,
            &mut self.presentation_studio.playhead,
            &mut self.presentation_studio.timeline_zoom,
            &mut self.presentation_studio.timeline_track_height,
            &mut self.presentation_studio.timeline_snapping,
        ) {
            self.presentation_studio.error = Some(error);
        }
        if (self.presentation_studio.playhead - timeline_playhead_before).abs() > 0.000_001
            && self.presentation_studio.playback.is_some()
            && !self.presentation_studio.scrubbing
        {
            timeline_seek_after_edit = Some(self.presentation_studio.playhead);
        }
        }
        let selected_id = self.presentation_studio.selected_clip.clone();
        let mut duplicate = false;
        let mut split = false;
        let mut merge_caption_request = false;
        let mut ripple_delete = false;
        let mut slip_delta = 0.0;
        let mut raise = false;
        let mut lower = false;
        let mut animation_jump = None;
        let mut audio_crossfade_request = None;
        if self.workspace_preferences.panels.inspector {
        if let Some(index) = project
            .clips
            .iter()
            .position(|clip| Some(clip.id.as_str()) == selected_id.as_deref())
        {
            let mut clip = project.clips[index].clone();
            let selected_clip_id = clip.id.clone();
            let mut set_effect_keyframes = Vec::new();
            let mut edit_effect_keyframes = Vec::new();
            let mut remove_effect_keyframe = None;
            let link_id = clip.link_group_id.clone();
            let clip_locked = project
                .clips
                .iter()
                .filter(|candidate| {
                    candidate.id == clip.id
                        || link_id.as_deref().is_some_and(|id| {
                            candidate.link_group_id.as_deref() == Some(id)
                        })
                })
                .any(|candidate| {
                    project
                        .track_for_clip(candidate)
                        .is_some_and(|track| track.locked)
                });
            ui.add_enabled_ui(!clip_locked, |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading(format!("{} · {:?}", clip.name, clip.kind));
                duplicate = ui.button("Duplicate").clicked();
                split = ui.button("Split at playhead").clicked();
                ripple_delete = ui.button("Ripple delete").clicked();
                if ui.button("Slip -1 frame").clicked() {
                    slip_delta = -clip.playback_rate / f64::from(project.fps);
                }
                if ui.button("Slip +1 frame").clicked() {
                    slip_delta = clip.playback_rate / f64::from(project.fps);
                }
                lower = ui.button("Lower layer").clicked();
                raise = ui.button("Raise layer").clicked();
            });
            ui.horizontal(|ui| {
                ui.label("Timeline start");
                ui.add(
                    egui::DragValue::new(&mut clip.timeline_start)
                        .range(0.0..=project.duration)
                        .speed(0.05)
                        .suffix(" s"),
                );
                if clip.kind != crate::composition::ClipKind::Adjustment {
                    ui.label("Source in");
                    ui.add(
                        egui::DragValue::new(&mut clip.source_start)
                            .range(0.0..=86_400.0)
                            .speed(0.05)
                            .suffix(" s"),
                    );
                }
                ui.label("Duration");
                ui.add(
                    egui::DragValue::new(&mut clip.duration)
                        .range(0.01..=86_400.0)
                        .speed(0.05)
                        .suffix(" s"),
                );
            });
            if matches!(
                clip.kind,
                crate::composition::ClipKind::Video | crate::composition::ClipKind::Audio
            ) {
                let max_rate = clip
                    .source_id
                    .as_deref()
                    .and_then(|id| project.sources.iter().find(|source| source.id == id))
                    .filter(|source| !source.still_image && source.solid_color.is_none())
                    .map_or(8.0, |source| {
                        ((source.duration - clip.source_start) / clip.duration).clamp(0.1, 8.0)
                    });
                let mut source_mode = clip.source_time_mode;
                let mut playback_rate = clip.playback_rate;
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.label("Source time");
                    egui::ComboBox::from_id_salt(("source-time-mode", &clip.id))
                        .selected_text(match source_mode {
                            crate::composition::SourceTimeMode::Forward => "Forward",
                            crate::composition::SourceTimeMode::Reverse => "Reverse",
                            crate::composition::SourceTimeMode::Freeze => "Freeze frame",
                        })
                        .show_ui(ui, |ui| {
                            changed |= ui.selectable_value(
                                &mut source_mode,
                                crate::composition::SourceTimeMode::Forward,
                                "Forward",
                            ).changed();
                            changed |= ui.selectable_value(
                                &mut source_mode,
                                crate::composition::SourceTimeMode::Reverse,
                                "Reverse",
                            ).changed();
                            changed |= ui.selectable_value(
                                &mut source_mode,
                                crate::composition::SourceTimeMode::Freeze,
                                "Freeze frame",
                            ).changed();
                        });
                    ui.add_enabled_ui(source_mode != crate::composition::SourceTimeMode::Freeze, |ui| {
                        changed |= ui.add(
                            egui::Slider::new(&mut playback_rate, 0.1..=max_rate)
                                .text("Speed")
                                .suffix("x"),
                        ).changed();
                    });
                });
                if changed {
                    match project.set_clip_source_time(&clip.id, source_mode, playback_rate) {
                        Ok(()) => {
                            clip.source_time_mode = source_mode;
                            clip.playback_rate = playback_rate;
                        }
                        Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                    }
                }
                ui.small("Audio retiming changes pitch; reverse reverses audio, freeze silences it.");
            }
            match clip.kind {
                crate::composition::ClipKind::Video | crate::composition::ClipKind::Adjustment => {
                    if clip.kind == crate::composition::ClipKind::Video {
                    ui.horizontal(|ui| {
                        ui.label("Position");
                        ui.add(
                            egui::DragValue::new(&mut clip.x)
                                .speed(1.0)
                                .prefix("X ")
                                .fixed_decimals(1),
                        );
                        ui.add(
                            egui::DragValue::new(&mut clip.y)
                                .speed(1.0)
                                .prefix("Y ")
                                .fixed_decimals(1),
                        );
                        ui.label("Scale");
                        ui.add(
                            egui::DragValue::new(&mut clip.scale_x)
                                .range(0.01..=16.0)
                                .speed(0.01)
                                .prefix("X ")
                                .fixed_decimals(2),
                        );
                        ui.add(
                            egui::DragValue::new(&mut clip.scale_y)
                                .range(0.01..=16.0)
                                .speed(0.01)
                                .prefix("Y ")
                                .fixed_decimals(2),
                        );
                        ui.checkbox(&mut clip.include_audio, "Use source audio");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Crop L/T/R/B");
                        for value in [
                            &mut clip.crop_left,
                            &mut clip.crop_top,
                            &mut clip.crop_right,
                            &mut clip.crop_bottom,
                        ] {
                            ui.add(
                                egui::DragValue::new(value)
                                    .range(0.0..=0.95)
                                    .speed(0.005)
                                    .fixed_decimals(3),
                            );
                        }
                        ui.label("Blur");
                        ui.add(
                            egui::DragValue::new(&mut clip.blur)
                                .range(0.0..=100.0)
                                .speed(0.2)
                                .fixed_decimals(1),
                        );
                        ui.label("Opacity");
                        ui.add(egui::Slider::new(&mut clip.opacity, 0.0..=1.0));
                    });
                    ui.collapsing("Mask", |ui| {
                        if clip.mask.is_none() && ui.button("Add rectangle mask").clicked() {
                            clip.mask = Some(crate::composition::VisualMask::default());
                        }
                        let mut remove_mask = false;
                        if let Some(mask) = &mut clip.mask {
                            ui.horizontal(|ui| {
                                ui.label("Shape");
                                egui::ComboBox::from_id_salt(("mask-shape", &clip.id))
                                    .selected_text(match mask.shape {
                                        crate::composition::VisualMaskShape::Rectangle => "Rectangle",
                                        crate::composition::VisualMaskShape::Ellipse => "Ellipse",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut mask.shape,
                                            crate::composition::VisualMaskShape::Rectangle,
                                            "Rectangle",
                                        );
                                        ui.selectable_value(
                                            &mut mask.shape,
                                            crate::composition::VisualMaskShape::Ellipse,
                                            "Ellipse",
                                        );
                                    });
                                ui.checkbox(&mut mask.inverted, "Invert");
                                if ui.small_button("Remove mask").clicked() {
                                    remove_mask = true;
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label("X / Y");
                                ui.add(egui::Slider::new(
                                    &mut mask.x,
                                    0.0..=(1.0 - mask.width).max(0.0),
                                ));
                                ui.add(egui::Slider::new(
                                    &mut mask.y,
                                    0.0..=(1.0 - mask.height).max(0.0),
                                ));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Width / height");
                                ui.add(egui::Slider::new(
                                    &mut mask.width,
                                    0.01..=(1.0 - mask.x).max(0.01),
                                ));
                                ui.add(egui::Slider::new(
                                    &mut mask.height,
                                    0.01..=(1.0 - mask.y).max(0.01),
                                ));
                            });
                            ui.add(
                                egui::Slider::new(&mut mask.feather, 0.0..=0.5)
                                    .text("Feather (short-edge ratio)"),
                            );
                        }
                        if remove_mask {
                            clip.mask = None;
                        }
                    });
                    }
                    ui.collapsing(format!("Effects ({})", clip.effects.len()), |ui| {
                        if clip.kind == crate::composition::ClipKind::Adjustment {
                            ui.add(egui::Slider::new(&mut clip.opacity, 0.0..=1.0).text("Blend"));
                            ui.small("Effects process the composite from lower visual layers.");
                        }
                        let mut copy_stack = false;
                        let mut paste_stack = false;
                        let mut copy_effect = None;
                        let mut paste_effect = false;
                        ui.horizontal(|ui| {
                            copy_stack = ui.small_button("Copy stack").clicked();
                            let clipboard_stack_valid = self
                                .presentation_studio
                                .effect_stack_clipboard
                                .as_ref()
                                .is_some_and(|stack| {
                                    clip.kind == crate::composition::ClipKind::Video
                                        || stack.iter().all(|effect| {
                                            !matches!(
                                                effect.kind,
                                                crate::composition::VisualEffectKind::ChromaKey { .. }
                                            )
                                        })
                                });
                            paste_stack = ui
                                .add_enabled(clipboard_stack_valid, egui::Button::new("Paste stack"))
                                .clicked();
                            let clipboard_effect_valid = self
                                .presentation_studio
                                .effect_clipboard
                                .as_ref()
                                .is_some_and(|effect| {
                                    clip.kind == crate::composition::ClipKind::Video
                                        || !matches!(
                                            effect.kind,
                                            crate::composition::VisualEffectKind::ChromaKey { .. }
                                        )
                                });
                            if clipboard_effect_valid
                                && ui.small_button("Paste effect").clicked()
                            {
                                paste_effect = true;
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.small_button("+ Blur").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::GaussianBlur {
                                            sigma: 4.0,
                                        },
                                    ),
                                );
                            }
                            if ui.small_button("+ Brightness").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Brightness {
                                            amount: 0.0,
                                        },
                                    ),
                                );
                            }
                            if ui.small_button("+ Contrast").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Contrast {
                                            amount: 1.0,
                                        },
                                    ),
                                );
                            }
                            if ui.small_button("+ Saturation").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Saturation {
                                            amount: 1.0,
                                        },
                                    ),
                                );
                            }
                            if ui.small_button("+ Hue").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Hue { degrees: 0.0 },
                                    ),
                                );
                            }
                            if ui.small_button("+ Sharpen").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Sharpen { amount: 1.0 },
                                    ),
                                );
                            }
                            if ui.small_button("+ Gamma").clicked() {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::Gamma { amount: 1.0 },
                                    ),
                                );
                            }
                            if clip.kind == crate::composition::ClipKind::Video
                                && ui.small_button("+ Chroma key").clicked()
                            {
                                clip.effects.push(
                                    crate::composition::VisualEffectInstance::new(
                                        crate::composition::VisualEffectKind::ChromaKey {
                                            key_color: "00FF00".into(),
                                            tolerance: 0.2,
                                            softness: 0.1,
                                        },
                                    ),
                                );
                            }
                        });
                        let mut remove_effect = None;
                        let mut reorder_effect = None;
                        for effect in &mut clip.effects {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut effect.enabled, "");
                                ui.label(effect.kind.label());
                                if ui.small_button("↑").clicked() {
                                    reorder_effect = Some((effect.id.clone(), -1));
                                }
                                if ui.small_button("↓").clicked() {
                                    reorder_effect = Some((effect.id.clone(), 1));
                                }
                                if ui.small_button("Remove").clicked() {
                                    remove_effect = Some(effect.id.clone());
                                }
                                if ui.small_button("Copy").clicked() {
                                    copy_effect = Some(effect.clone());
                                }
                                match &mut effect.kind {
                                    crate::composition::VisualEffectKind::GaussianBlur { sigma } => {
                                        ui.add(egui::DragValue::new(sigma).range(0.0..=100.0).speed(0.2).prefix("Sigma "));
                                    }
                                    crate::composition::VisualEffectKind::Brightness { amount } => {
                                        ui.add(egui::Slider::new(amount, -1.0..=1.0).text("Amount"));
                                    }
                                    crate::composition::VisualEffectKind::Contrast { amount }
                                    | crate::composition::VisualEffectKind::Saturation { amount } => {
                                        ui.add(egui::Slider::new(amount, 0.0..=4.0).text("Amount"));
                                    }
                                    crate::composition::VisualEffectKind::Hue { degrees } => {
                                        ui.add(egui::Slider::new(degrees, -180.0..=180.0).text("Degrees"));
                                    }
                                    crate::composition::VisualEffectKind::Sharpen { amount } => {
                                        ui.add(egui::Slider::new(amount, 0.0..=5.0).text("Amount"));
                                    }
                                    crate::composition::VisualEffectKind::Gamma { amount } => {
                                        ui.add(egui::Slider::new(amount, 0.1..=10.0).text("Gamma"));
                                    }
                                    crate::composition::VisualEffectKind::ChromaKey { key_color, tolerance, softness } => {
                                        ui.label("Key RGB");
                                        ui.add(egui::TextEdit::singleline(key_color).desired_width(72.0));
                                        ui.add(egui::Slider::new(tolerance, 0.0..=1.0).text("Tolerance"));
                                        ui.add(egui::Slider::new(softness, 0.0..=1.0).text("Softness"));
                                    }
                                }
                            });
                            let animated_parameter = match effect.kind {
                                crate::composition::VisualEffectKind::Brightness { .. }
                                | crate::composition::VisualEffectKind::Contrast { .. }
                                | crate::composition::VisualEffectKind::Saturation { .. }
                                | crate::composition::VisualEffectKind::Gamma { .. } => {
                                    Some(crate::composition::EffectParameter::Amount)
                                }
                                crate::composition::VisualEffectKind::Hue { .. } => {
                                    Some(crate::composition::EffectParameter::HueDegrees)
                                }
                                _ => None,
                            };
                            if let Some(parameter) = animated_parameter {
                                let current_value = match (&effect.kind, parameter) {
                                    (
                                        crate::composition::VisualEffectKind::Brightness { amount }
                                        | crate::composition::VisualEffectKind::Contrast { amount }
                                        | crate::composition::VisualEffectKind::Saturation { amount }
                                        | crate::composition::VisualEffectKind::Gamma { amount },
                                        crate::composition::EffectParameter::Amount,
                                    ) => *amount,
                                    (
                                        crate::composition::VisualEffectKind::Hue { degrees },
                                        crate::composition::EffectParameter::HueDegrees,
                                    ) => *degrees,
                                    _ => 0.0,
                                };
                                let has_key_at_playhead = effect
                                    .animated_parameters
                                    .get(&parameter)
                                    .is_some_and(|keys| {
                                        keys.iter().any(|key| {
                                            (key.time - self.presentation_studio.playhead).abs()
                                                < 0.000_001
                                        })
                                    });
                                ui.horizontal(|ui| {
                                    let key_count = effect
                                        .animated_parameters
                                        .get(&parameter)
                                        .map_or(0, Vec::len);
                                    ui.small(format!("{key_count} {parameter:?} keys"));
                                    if ui
                                        .small_button(format!(
                                            "Set key at {:.2}s",
                                            self.presentation_studio.playhead
                                        ))
                                        .clicked()
                                    {
                                        set_effect_keyframes.push((
                                            effect.id.clone(),
                                            parameter,
                                            self.presentation_studio.playhead,
                                            current_value,
                                            self.presentation_studio.pending_keyframe_easing,
                                        ));
                                    }
                                    if has_key_at_playhead
                                        && ui.small_button("Remove key here").clicked()
                                    {
                                        remove_effect_keyframe = Some((
                                            effect.id.clone(),
                                            parameter,
                                            self.presentation_studio.playhead,
                                        ));
                                    }
                                });
                                let (minimum, maximum) =
                                    crate::composition::effect_parameter_range(
                                        &effect.kind,
                                        parameter,
                                    )
                                    .expect("only animatable effect parameters reach this editor");
                                if let Some(keys) = effect.animated_parameters.get(&parameter) {
                                    for (key_index, key) in keys.iter().enumerate() {
                                        let mut time = key.time;
                                        let mut value = key.value;
                                        let mut easing = key.easing_to_next;
                                        let mut remove_this_key = false;
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::DragValue::new(&mut time)
                                                    .range(0.0..=project.duration)
                                                    .speed(0.02)
                                                    .suffix(" s"),
                                            );
                                            ui.add(
                                                egui::DragValue::new(&mut value)
                                                    .range(minimum..=maximum)
                                                    .speed(((maximum - minimum) / 500.0).max(0.001))
                                                    .fixed_decimals(3),
                                            );
                                            egui::ComboBox::from_id_salt((
                                                "effect-key-easing",
                                                &effect.id,
                                                parameter,
                                                key_index,
                                            ))
                                            .selected_text(format!("{:?}", easing))
                                            .show_ui(ui, |ui| {
                                                for (option, label) in [
                                                    (crate::composition::Easing::Linear, "Linear"),
                                                    (crate::composition::Easing::Hold, "Hold"),
                                                    (crate::composition::Easing::EaseIn, "Ease in"),
                                                    (crate::composition::Easing::EaseOut, "Ease out"),
                                                    (crate::composition::Easing::EaseInOut, "Ease in/out"),
                                                ] {
                                                    ui.selectable_value(&mut easing, option, label);
                                                }
                                            });
                                            if ui.small_button("Go").clicked() {
                                                animation_jump = Some(time);
                                            }
                                            if ui.small_button("Remove").clicked() {
                                                remove_this_key = true;
                                                remove_effect_keyframe = Some((
                                                    effect.id.clone(),
                                                    parameter,
                                                    key.time,
                                                ));
                                            }
                                        });
                                        if !remove_this_key
                                            && (time != key.time
                                            || value != key.value
                                            || easing != key.easing_to_next)
                                        {
                                            edit_effect_keyframes.push((
                                                effect.id.clone(),
                                                parameter,
                                                key.time,
                                                time,
                                                value,
                                                easing,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(effect_id) = remove_effect {
                            clip.effects.retain(|effect| effect.id != effect_id);
                        }
                        if let Some((effect_id, direction)) = reorder_effect {
                            if let Some(index) = clip.effects.iter().position(|effect| effect.id == effect_id) {
                                let target = index as isize + direction;
                                if (0..clip.effects.len() as isize).contains(&target) {
                                    clip.effects.swap(index, target as usize);
                                }
                            }
                        }
                        if copy_stack {
                            self.presentation_studio.effect_stack_clipboard =
                                Some(clip.effects.clone());
                        }
                        if paste_stack {
                            if let Some(stack) =
                                self.presentation_studio.effect_stack_clipboard.clone()
                            {
                                clip.effects = stack
                                    .into_iter()
                                    .map(|effect| {
                                        let mut pasted =
                                            crate::composition::VisualEffectInstance::new(
                                                effect.kind,
                                            );
                                        pasted.enabled = effect.enabled;
                                        pasted
                                    })
                                    .collect();
                            }
                        }
                        if paste_effect {
                            if let Some(effect) =
                                self.presentation_studio.effect_clipboard.clone()
                            {
                                let mut pasted = crate::composition::VisualEffectInstance::new(
                                    effect.kind,
                                );
                                pasted.enabled = effect.enabled;
                                clip.effects.push(pasted);
                            }
                        }
                        if let Some(effect) = copy_effect {
                            self.presentation_studio.effect_clipboard = Some(effect);
                        }
                    });
                    if clip.kind == crate::composition::ClipKind::Video {
                    ui.collapsing("Transitions", |ui| {
                        let candidate = project
                            .clips
                            .iter()
                            .filter(|other| {
                                let overlap = (clip.timeline_start + clip.duration)
                                    .min(other.timeline_start + other.duration)
                                    - clip.timeline_start.max(other.timeline_start);
                                other.id != clip.id
                                    && other.kind == crate::composition::ClipKind::Video
                                    && other.track_id == clip.track_id
                                    && !project
                                        .track_for_clip(other)
                                        .is_some_and(|track| track.locked)
                                    && overlap > 1.0 / f64::from(project.fps)
                                    && !project.transitions.iter().any(|transition| {
                                        (transition.from_clip_id == clip.id
                                            && transition.to_clip_id == other.id)
                                            || (transition.to_clip_id == clip.id
                                                && transition.from_clip_id == other.id)
                                    })
                                })
                            .max_by(|left, right| {
                                let left_overlap = (clip.timeline_start + clip.duration)
                                    .min(left.timeline_start + left.duration)
                                    - clip.timeline_start.max(left.timeline_start);
                                let right_overlap = (clip.timeline_start + clip.duration)
                                    .min(right.timeline_start + right.duration)
                                    - clip.timeline_start.max(right.timeline_start);
                                left_overlap.total_cmp(&right_overlap)
                            })
                            .map(|other| {
                                let pair = if clip.z_order <= other.z_order {
                                    (clip.id.clone(), other.id.clone())
                                } else {
                                    (other.id.clone(), clip.id.clone())
                                };
                                let overlap = (clip.timeline_start + clip.duration)
                                    .min(other.timeline_start + other.duration)
                                    - clip.timeline_start.max(other.timeline_start);
                                (pair, other.name.clone(), overlap)
                            });
                        let mut requested_kind = None;
                        if let Some(((from_id, to_id), partner_name, overlap)) = candidate {
                            ui.label(format!("Overlapping clip: {partner_name}"));
                            let default_duration = overlap.min(0.5).max(1.0 / f64::from(project.fps));
                            ui.horizontal_wrapped(|ui| {
                                for (kind, label) in [
                                    (crate::composition::TransitionKind::CrossDissolve, "Add dissolve"),
                                    (crate::composition::TransitionKind::DipToBlack, "Add dip to black"),
                                    (crate::composition::TransitionKind::DipToWhite, "Add dip to white"),
                                ] {
                                    if ui.small_button(label).clicked() {
                                        requested_kind = Some(kind);
                                    }
                                }
                            });
                            if let Some(kind) = requested_kind {
                                if let Err(error) = project.add_transition_between(
                                    &from_id,
                                    &to_id,
                                    kind,
                                    default_duration,
                                ) {
                                    self.presentation_studio.error = Some(format!("{error:#}"));
                                }
                            }
                        } else {
                            ui.weak("Add a transition after overlapping two visual clips on one track.");
                        }

                        let attached = project
                            .transitions
                            .iter()
                            .filter(|transition| {
                                transition.from_clip_id == clip.id
                                    || transition.to_clip_id == clip.id
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        for mut transition in attached {
                            let mut changed = false;
                            let mut remove = false;
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt(("transition-kind", &transition.id))
                                    .selected_text(match transition.kind {
                                        crate::composition::TransitionKind::CrossDissolve => "Cross dissolve",
                                        crate::composition::TransitionKind::DipToBlack => "Dip to black",
                                        crate::composition::TransitionKind::DipToWhite => "Dip to white",
                                    })
                                    .show_ui(ui, |ui| {
                                        for (kind, label) in [
                                            (crate::composition::TransitionKind::CrossDissolve, "Cross dissolve"),
                                            (crate::composition::TransitionKind::DipToBlack, "Dip to black"),
                                            (crate::composition::TransitionKind::DipToWhite, "Dip to white"),
                                        ] {
                                            changed |= ui.selectable_value(&mut transition.kind, kind, label).changed();
                                        }
                                    });
                                egui::ComboBox::from_id_salt(("transition-easing", &transition.id))
                                    .selected_text(transition.easing.label())
                                    .show_ui(ui, |ui| {
                                        for easing in [
                                            crate::composition::Easing::Linear,
                                            crate::composition::Easing::Hold,
                                            crate::composition::Easing::EaseIn,
                                            crate::composition::Easing::EaseOut,
                                            crate::composition::Easing::EaseInOut,
                                        ] {
                                            changed |= ui.selectable_value(
                                                &mut transition.easing,
                                                easing,
                                                easing.label(),
                                            ).changed();
                                        }
                                    });
                                ui.label("Duration");
                                changed |= ui.add(
                                    egui::DragValue::new(&mut transition.duration)
                                        .range((1.0 / f64::from(project.fps))..=project.duration)
                                        .speed(0.05)
                                        .suffix(" s"),
                                ).changed();
                                ui.label(format!("at {:.2}s", transition.start_time));
                                remove = ui.small_button("Remove").clicked();
                            });
                            if remove {
                                if let Err(error) = project.remove_transition(&transition.id) {
                                    self.presentation_studio.error = Some(format!("{error:#}"));
                                }
                            } else if changed {
                                if let Err(error) = project.update_transition(
                                    &transition.id,
                                    transition.kind,
                                    transition.duration,
                                ) {
                                    self.presentation_studio.error = Some(format!("{error:#}"));
                                } else if let Err(error) = project.set_transition_easing(
                                    &transition.id,
                                    transition.easing,
                                ) {
                                    self.presentation_studio.error = Some(format!("{error:#}"));
                                }
                            }
                        }
                    });
                    }
                }
                crate::composition::ClipKind::Audio => {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut clip.muted, "Mute");
                        ui.checkbox(&mut clip.solo, "Solo");
                        ui.label("Gain");
                        ui.add(egui::Slider::new(&mut clip.gain, 0.0..=4.0));
                        ui.label("Pan");
                        ui.add(egui::Slider::new(&mut clip.pan, -1.0..=1.0));
                        ui.label("Fade in");
                        ui.add(
                            egui::DragValue::new(&mut clip.fade_in)
                                .range(0.0..=clip.duration)
                                .speed(0.05),
                        );
                        ui.label("Fade out");
                        ui.add(
                            egui::DragValue::new(&mut clip.fade_out)
                                .range(0.0..=clip.duration)
                                .speed(0.05),
                        );
                    });
                    let overlaps = project
                        .clips
                        .iter()
                        .filter(|other| {
                            other.id != clip.id
                                && other.kind == crate::composition::ClipKind::Audio
                                && other.track_id == clip.track_id
                                && other.timeline_start < clip.timeline_start + clip.duration
                                && clip.timeline_start < other.timeline_start + other.duration
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    for other in overlaps {
                        let (outgoing, incoming) = if clip.timeline_start <= other.timeline_start {
                            (&clip, &other)
                        } else {
                            (&other, &clip)
                        };
                        let overlap = outgoing.timeline_start + outgoing.duration
                            - incoming.timeline_start;
                        if overlap > 0.0
                            && incoming.timeline_start + incoming.duration
                                + 0.5 / f64::from(project.fps)
                                >= outgoing.timeline_start + outgoing.duration
                        {
                            if ui
                                .button(format!("Crossfade with {} ({overlap:.2}s)", other.name))
                                .clicked()
                            {
                                audio_crossfade_request =
                                    Some((outgoing.id.clone(), incoming.id.clone()));
                            }
                        }
                    }
                }
                crate::composition::ClipKind::Text => {
                    if let Some(caption) = &clip.caption {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(egui::Color32::LIGHT_BLUE, "Timed caption");
                            ui.small(match &caption.origin {
                                crate::composition::CaptionOrigin::Imported => "Imported subtitle".to_owned(),
                                crate::composition::CaptionOrigin::Transcribed => format!(
                                    "Transcribed · {} · {}",
                                    caption.model_id.as_deref().unwrap_or("local model"),
                                    caption.language.as_deref().unwrap_or("auto")
                                ),
                            });
                            if self.presentation_studio.selected_clips.len() == 2
                                && ui.button("Merge selected captions").clicked()
                            {
                                merge_caption_request = true;
                            }
                        });
                    }
                    ui.add(
                        egui::TextEdit::multiline(&mut clip.text)
                            .desired_rows(2)
                            .hint_text("Caption text"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Position");
                        ui.add(egui::DragValue::new(&mut clip.x).speed(1.0).prefix("X "));
                        ui.add(egui::DragValue::new(&mut clip.y).speed(1.0).prefix("Y "));
                        ui.label("Size");
                        ui.add(egui::DragValue::new(&mut clip.font_size).range(1..=512));
                        ui.label("Fill RGB");
                        ui.text_edit_singleline(&mut clip.text_color);
                        ui.label("Outline RGB");
                        ui.text_edit_singleline(&mut clip.outline_color);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Outline");
                        ui.add(
                            egui::DragValue::new(&mut clip.outline_width)
                                .range(0.0..=32.0)
                                .speed(0.2),
                        );
                        ui.label("Shadow X/Y");
                        ui.add(egui::DragValue::new(&mut clip.shadow_x).speed(0.5));
                        ui.add(egui::DragValue::new(&mut clip.shadow_y).speed(0.5));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Alignment");
                        egui::ComboBox::from_id_salt(("text-alignment", &clip.id))
                            .selected_text(format!("{:?}", clip.text_alignment))
                            .show_ui(ui, |ui| {
                                for alignment in [
                                    crate::composition::TextAlignment::Left,
                                    crate::composition::TextAlignment::Center,
                                    crate::composition::TextAlignment::Right,
                                ] {
                                    ui.selectable_value(
                                        &mut clip.text_alignment,
                                        alignment,
                                        format!("{alignment:?}"),
                                    );
                                }
                            });
                        if ui.button("Choose font…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_title("Choose Text Font")
                                .add_filter("Font", &["ttf", "otf", "ttc"])
                                .pick_file()
                            {
                                clip.font_path = Some(path);
                            }
                        }
                        if ui.button("System default font").clicked() {
                            clip.font_path = None;
                        }
                        ui.small(
                            clip.font_path
                                .as_deref()
                                .and_then(|path| path.file_name())
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "System default".into()),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut clip.text_background_enabled, "Background box");
                        if clip.text_background_enabled {
                            ui.label("RGB");
                            ui.text_edit_singleline(&mut clip.text_background_color);
                            ui.label("Opacity");
                            ui.add(
                                egui::Slider::new(&mut clip.text_background_opacity, 0.0..=1.0)
                                    .show_value(true),
                            );
                            ui.label("Padding");
                            ui.add(
                                egui::DragValue::new(&mut clip.text_background_padding)
                                    .range(0..=256),
                            );
                        }
                    });
                }
            }
            let mut animated_properties: Vec<crate::composition::AnimatedProperty> = match clip.kind {
                crate::composition::ClipKind::Video => vec![
                    crate::composition::AnimatedProperty::X,
                    crate::composition::AnimatedProperty::Y,
                    crate::composition::AnimatedProperty::ScaleX,
                    crate::composition::AnimatedProperty::ScaleY,
                    crate::composition::AnimatedProperty::Rotation,
                    crate::composition::AnimatedProperty::Opacity,
                    crate::composition::AnimatedProperty::CropLeft,
                    crate::composition::AnimatedProperty::CropTop,
                    crate::composition::AnimatedProperty::CropRight,
                    crate::composition::AnimatedProperty::CropBottom,
                    crate::composition::AnimatedProperty::Blur,
                ],
                crate::composition::ClipKind::Audio => vec![
                    crate::composition::AnimatedProperty::Gain,
                    crate::composition::AnimatedProperty::Pan,
                ],
                crate::composition::ClipKind::Text => vec![
                    crate::composition::AnimatedProperty::X,
                    crate::composition::AnimatedProperty::Y,
                    crate::composition::AnimatedProperty::FontSize,
                    crate::composition::AnimatedProperty::Opacity,
                ],
                crate::composition::ClipKind::Adjustment => vec![
                    crate::composition::AnimatedProperty::Opacity,
                ],
            };
            if clip.mask.is_some() {
                animated_properties.extend([
                    crate::composition::AnimatedProperty::MaskX,
                    crate::composition::AnimatedProperty::MaskY,
                    crate::composition::AnimatedProperty::MaskWidth,
                    crate::composition::AnimatedProperty::MaskHeight,
                    crate::composition::AnimatedProperty::MaskFeather,
                ]);
            }
            if !animated_properties.contains(&self.presentation_studio.selected_animated_property) {
                self.presentation_studio.selected_animated_property = animated_properties[0];
            }
            let property = self.presentation_studio.selected_animated_property;
            ui.horizontal(|ui| {
                ui.label("Animate");
                egui::ComboBox::from_id_salt(("animated-property", &clip.id))
                    .selected_text(property.label())
                    .show_ui(ui, |ui| {
                        for option in &animated_properties {
                            ui.selectable_value(
                                &mut self.presentation_studio.selected_animated_property,
                                *option,
                                option.label(),
                            );
                        }
                    });
                if clip.has_animation(property) {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "◁Eanimated");
                }
                let mut value = clip.animated_value(property, self.presentation_studio.playhead);
                ui.add(
                    egui::DragValue::new(&mut value)
                        .range(-32_768.0..=32_768.0)
                        .speed(0.5)
                        .fixed_decimals(2),
                );
                if ui.button("Set at playhead").clicked() {
                    if let Err(error) = clip.set_property_at(
                        property,
                        self.presentation_studio.playhead,
                        value,
                    ) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                }
                egui::ComboBox::from_id_salt(("pending-key-easing", &clip.id))
                    .selected_text(format!("{:?}", self.presentation_studio.pending_keyframe_easing))
                    .show_ui(ui, |ui| {
                        for (easing, label) in [
                            (crate::composition::Easing::Linear, "Linear"),
                            (crate::composition::Easing::Hold, "Hold"),
                            (crate::composition::Easing::EaseIn, "Ease in"),
                            (crate::composition::Easing::EaseOut, "Ease out"),
                            (crate::composition::Easing::EaseInOut, "Ease in/out"),
                        ] {
                            ui.selectable_value(
                                &mut self.presentation_studio.pending_keyframe_easing,
                                easing,
                                label,
                            );
                        }
                    });
                if ui.button("Key at playhead").clicked() {
                    self.pending_keyframe_value = Some(value);
                    pending_command_actions.push(crate::workspace::CommandId::AddKeyframe);
                }
            });
            let mut remove_key_at = None;
            if let Some(keys) = clip.animated_properties.get_mut(&property) {
                for (index, key) in keys.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} key", property.label()));
                        ui.add(
                            egui::DragValue::new(&mut key.time)
                                .range(0.0..=project.duration)
                                .speed(0.05)
                                .suffix(" s"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut key.value)
                                .range(-32_768.0..=32_768.0)
                                .speed(0.5)
                                .fixed_decimals(2),
                        );
                        egui::ComboBox::from_id_salt(("key-ease", &clip.id, index))
                            .selected_text(format!("{:?}", key.easing_to_next))
                            .show_ui(ui, |ui| {
                                for (easing, label) in [
                                    (crate::composition::Easing::Linear, "Linear"),
                                    (crate::composition::Easing::Hold, "Hold"),
                                    (crate::composition::Easing::EaseIn, "Ease in"),
                                    (crate::composition::Easing::EaseOut, "Ease out"),
                                    (crate::composition::Easing::EaseInOut, "Ease in/out"),
                                ] {
                                    ui.selectable_value(&mut key.easing_to_next, easing, label);
                                }
                            });
                        if ui.small_button("Go").clicked() {
                            animation_jump = Some(key.time);
                        }
                        if ui.small_button("Remove key").clicked() {
                            remove_key_at = Some(key.time);
                        }
                    });
                }
                keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
            let mut paste_curve_keys = None;
            ui.collapsing("Keyframe curve", |ui| {
                let (minimum, maximum) =
                    crate::composition::animated_property_range(property);
                let duration = project.duration.max(1.0 / f64::from(project.fps));
                let width = ui.available_width().clamp(220.0, 560.0);
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(width, 132.0),
                    egui::Sense::click_and_drag(),
                );
                let graph = response.rect.shrink2(egui::vec2(20.0, 12.0));
                painter.rect_filled(response.rect, 3.0, egui::Color32::from_gray(25));
                painter.line_segment(
                    [graph.left_top(), graph.left_bottom()],
                    egui::Stroke::new(1.0, egui::Color32::GRAY),
                );
                painter.line_segment(
                    [graph.left_bottom(), graph.right_bottom()],
                    egui::Stroke::new(1.0, egui::Color32::GRAY),
                );
                for row in 1..4 {
                    let y = graph.top() + graph.height() * row as f32 / 4.0;
                    painter.line_segment(
                        [egui::pos2(graph.left(), y), egui::pos2(graph.right(), y)],
                        egui::Stroke::new(0.5, egui::Color32::from_gray(58)),
                    );
                }
                let keys = clip.animated_properties.entry(property).or_default();
                let point_for = |key: &crate::composition::ScalarKeyframe| {
                    let time_fraction = (key.time / duration).clamp(0.0, 1.0) as f32;
                    let value_fraction = ((key.value - minimum) / (maximum - minimum))
                        .clamp(0.0, 1.0) as f32;
                    egui::pos2(
                        graph.left() + graph.width() * time_fraction,
                        graph.bottom() - graph.height() * value_fraction,
                    )
                };
                for pair in keys.windows(2) {
                    painter.line_segment(
                        [point_for(&pair[0]), point_for(&pair[1])],
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 190, 235)),
                    );
                }
                let mut selected_indices = self
                    .presentation_studio
                    .selected_curve_keys
                    .as_ref()
                    .filter(|(clip_id, selected_property, _)| {
                        clip_id == &clip.id && *selected_property == property
                    })
                    .map(|(_, _, indices)| indices.clone())
                    .unwrap_or_default();
                selected_indices.retain(|index| *index < keys.len());
                for (index, key) in keys.iter().enumerate() {
                    let point = point_for(key);
                    let color = if selected_indices.contains(&index) {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::from_rgb(235, 150, 80)
                    };
                    painter.circle_filled(point, 5.0, color);
                }
                if response.clicked() || response.drag_started() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let nearest = keys
                            .iter()
                            .enumerate()
                            .map(|(index, key)| (index, point_for(key).distance(pointer)))
                            .filter(|(_, distance)| *distance <= 14.0)
                            .min_by(|left, right| left.1.total_cmp(&right.1))
                            .map(|(index, _)| index);
                        if let Some(index) = nearest {
                            let multi_select = ui.input(|input| {
                                input.modifiers.ctrl || input.modifiers.command
                            });
                            if multi_select {
                                if !selected_indices.insert(index) {
                                    selected_indices.remove(&index);
                                }
                            } else {
                                selected_indices.clear();
                                selected_indices.insert(index);
                            }
                            self.presentation_studio.selected_curve_keys = Some((
                                clip.id.clone(),
                                property,
                                selected_indices.clone(),
                            ));
                        }
                    }
                }
                if response.drag_started() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let selected = self
                            .presentation_studio
                            .selected_curve_keys
                            .as_ref()
                            .filter(|(clip_id, selected_property, _)| {
                                clip_id == &clip.id && *selected_property == property
                            })
                            .map(|(_, _, indices)| indices.clone())
                            .unwrap_or_default();
                        let origins = keys
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| selected.contains(index))
                            .map(|(index, key)| (index, key.time, key.value))
                            .collect::<Vec<_>>();
                        self.presentation_studio.curve_drag_origin = (!origins.is_empty())
                            .then_some((clip.id.clone(), property, pointer, origins));
                    }
                }
                if response.dragged() {
                    if let (
                        Some(pointer),
                        Some((clip_id, selected_property, origin_pointer, origins)),
                    ) = (
                        response.interact_pointer_pos(),
                        self.presentation_studio.curve_drag_origin.clone(),
                    ) {
                        if clip_id == clip.id && selected_property == property {
                            let selected = origins
                                .iter()
                                .map(|(index, _, _)| *index)
                                .collect::<HashSet<_>>();
                            let mut min_delta_time = -duration;
                            let mut max_delta_time = duration;
                            let mut min_delta_value = minimum - maximum;
                            let mut max_delta_value = maximum - minimum;
                            for (index, time, value) in &origins {
                                if let Some(previous) = (0..*index)
                                    .rev()
                                    .find(|candidate| !selected.contains(candidate))
                                {
                                    min_delta_time = min_delta_time
                                        .max(keys[previous].time + 0.0001 - *time);
                                }
                                if let Some(next) = (*index + 1..keys.len())
                                    .find(|candidate| !selected.contains(candidate))
                                {
                                    max_delta_time = max_delta_time
                                        .min(keys[next].time - 0.0001 - *time);
                                }
                                min_delta_time = min_delta_time.max(-*time);
                                max_delta_time = max_delta_time.min(duration - *time);
                                min_delta_value = min_delta_value.max(minimum - *value);
                                max_delta_value = max_delta_value.min(maximum - *value);
                            }
                            let delta_time = (pointer.x - origin_pointer.x) as f64
                                / graph.width().max(1.0) as f64
                                * duration;
                            let delta_value = -(pointer.y - origin_pointer.y) as f64
                                / graph.height().max(1.0) as f64
                                * (maximum - minimum);
                            let delta_time = delta_time.clamp(
                                min_delta_time,
                                max_delta_time.max(min_delta_time),
                            );
                            let delta_value = delta_value.clamp(
                                min_delta_value,
                                max_delta_value.max(min_delta_value),
                            );
                            for (index, time, value) in origins {
                                keys[index].time = time + delta_time;
                                keys[index].value = value + delta_value;
                            }
                        }
                    }
                }
                if response.drag_stopped() {
                    self.presentation_studio.curve_drag_origin = None;
                }
                ui.horizontal(|ui| {
                    if let Some(index) = selected_indices.iter().min().copied() {
                        ui.label(format!(
                            "Key {:.3}s · {:.3} · {:?}",
                            keys[index].time, keys[index].value, keys[index].easing_to_next
                        ));
                        if ui.small_button("Copy selected keys").clicked() {
                            self.presentation_studio.keyframe_clipboard =
                                Some((
                                    property,
                                    selected_indices
                                        .iter()
                                        .filter_map(|index| keys.get(*index).cloned())
                                        .collect(),
                                ));
                        }
                    } else {
                        ui.weak("Click a key to select it; Ctrl/Command-click to select more; drag to move.");
                    }
                    let copied = self
                        .presentation_studio
                        .keyframe_clipboard
                        .as_ref()
                        .filter(|(copied_property, _)| *copied_property == property)
                        .map(|(_, keys)| keys.clone());
                    if let Some(keys) = copied.filter(|keys| !keys.is_empty()) {
                        if ui.small_button("Paste at playhead").clicked() {
                            paste_curve_keys = Some(keys);
                        }
                    }
                });
            });
            if let Some(keys) = paste_curve_keys {
                let anchor = keys
                    .iter()
                    .map(|key| key.time)
                    .fold(f64::INFINITY, f64::min);
                let first_index = clip
                    .animated_properties
                    .get(&property)
                    .map_or(0, Vec::len);
                let mut candidate = clip.clone();
                let result = keys.iter().try_for_each(|key| {
                    candidate.add_keyframe(
                        property,
                        self.presentation_studio.playhead + key.time - anchor,
                        key.value,
                        key.easing_to_next,
                    )
                });
                match result {
                    Ok(()) => {
                        clip = candidate;
                        let selected = (first_index..first_index + keys.len()).collect();
                        self.presentation_studio.selected_curve_keys =
                            Some((clip.id.clone(), property, selected));
                    }
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
            if let Some(time) = remove_key_at {
                clip.remove_keyframe(property, time);
                self.presentation_studio.selected_curve_keys = None;
            }
            });
            if clip_locked {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "This clip or its linked media is on a locked track.",
                );
            } else {
                project.clips[index] = clip;
                for (effect_id, parameter, time, value, easing) in set_effect_keyframes {
                    if let Err(error) = project.set_effect_keyframe(
                        &selected_clip_id,
                        &effect_id,
                        parameter,
                        time,
                        value,
                        easing,
                    ) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                }
                for (effect_id, parameter, original_time, time, value, easing) in
                    edit_effect_keyframes
                {
                    if let Err(error) = project.edit_effect_keyframe(
                        &selected_clip_id,
                        &effect_id,
                        parameter,
                        original_time,
                        time,
                        value,
                        easing,
                    ) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                }
                if let Some((effect_id, parameter, time)) = remove_effect_keyframe {
                    if let Err(error) = project.remove_effect_keyframe(
                        &selected_clip_id,
                        &effect_id,
                        parameter,
                        time,
                    ) {
                        self.presentation_studio.error = Some(format!("{error:#}"));
                    }
                }
            }
        }
        if let Some(time) = animation_jump {
            self.presentation_studio.playhead = time.clamp(0.0, project.duration);
        }
        if let Some(id) = selected_id {
            if ripple_delete {
                pending_command_actions.push(crate::workspace::CommandId::RippleDeleteClip);
            }
            if slip_delta != 0.0 {
                if let Err(error) = project.slip_clip_by(&id, slip_delta) {
                    self.presentation_studio.error = Some(format!("{error:#}"));
                }
            }
            if duplicate {
                pending_command_actions.push(crate::workspace::CommandId::DuplicateClip);
            }
            if split {
                pending_command_actions.push(crate::workspace::CommandId::SplitClip);
            }
            if raise || lower {
                if let Some(index) = project.clips.iter().position(|clip| clip.id == id) {
                    project.clips[index].z_order += if raise { 1 } else { -1 };
                }
            }
        }
        if merge_caption_request {
            let ids = self
                .presentation_studio
                .selected_clips
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            if ids.len() == 2 {
                match crate::subtitles::merge_captions(project, &ids[0], &ids[1]) {
                    Ok(()) => {
                        self.presentation_studio.selected_clip = Some(ids[0].clone());
                        self.presentation_studio.selected_clips.clear();
                        self.presentation_studio.selected_clips.insert(ids[0].clone());
                        self.presentation_studio.error = None;
                    }
                    Err(error) => self.presentation_studio.error = Some(format!("{error:#}")),
                }
            }
        }
        if let Some((outgoing, incoming)) = audio_crossfade_request {
            if let Err(error) = project.add_audio_crossfade(&outgoing, &incoming) {
                self.presentation_studio.error = Some(format!("{error:#}"));
            }
        }
        }

        let export_range_enabled = project.export_range.is_some();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Output");
            let mut path = self
                .presentation_studio
                .output
                .to_string_lossy()
                .into_owned();
            if ui
                .add(egui::TextEdit::singleline(&mut path).desired_width(300.0))
                .changed()
            {
                self.presentation_studio.output = PathBuf::from(path);
            }
            if ui.button("Browse…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Export Composition")
                    .add_filter("MP4 video", &["mp4"])
                    .set_file_name("composition.mp4")
                    .save_file()
                {
                    self.presentation_studio.output = path;
                }
            }
            if self.presentation_studio.job.is_none() {
                if ui.button("Render Preview").clicked() {
                    self.start_presentation_preview(ui.ctx());
                }
                if let Some((preview_time, preview_png)) =
                    self.presentation_studio.preview_png.as_ref()
                {
                    if ui
                        .button(format!("Save displayed PNG ({preview_time:.2}s)…"))
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title("Save Displayed Composition Frame")
                            .add_filter("PNG image", &["png"])
                            .set_file_name(format!("frame-{preview_time:.3}.png"))
                            .save_file()
                        {
                            match save_composition_preview_png(&path, preview_png) {
                                Ok(()) => {
                                    self.presentation_studio.message = format!(
                                        "Saved displayed composition frame at {preview_time:.3}s to {}.",
                                        path.display()
                                    );
                                    self.presentation_studio.error = None;
                                }
                                Err(error) => {
                                    self.presentation_studio.error = Some(format!("{error:#}"));
                                }
                            }
                        }
                    }
                }
                if ui
                    .button(if export_range_enabled {
                        "Export MP4 range"
                    } else {
                        "Export MP4"
                    })
                    .clicked()
                {
                    self.dispatch_command(crate::workspace::CommandId::ExportComposition, ui.ctx());
                }
            } else if ui.button("Cancel").clicked() {
                if let Some(job) = &self.presentation_studio.job {
                    job.control.cancel();
                }
            }
        });
        });
        if let Some((source_id, output)) = pending_proxy_generation {
            self.start_presentation_proxy_generation(ui.ctx(), source_id, output);
        }
        if let Some((source_id, source_time)) = source_preview_request {
            self.start_presentation_source_preview(ui.ctx(), source_id, source_time);
        }
        if let Some(target) = timeline_seek_after_edit {
            self.seek_presentation_playback(ui.ctx(), target);
        }
        if let Some(texture) = &self.presentation_studio.preview_texture {
            let max = egui::vec2(ui.available_width().max(1.0), 440.0);
            let tex_size = texture.size_vec2();
            let scale = (max.x / tex_size.x).min(max.y / tex_size.y).min(1.0);
            ui.image((texture.id(), tex_size * scale));
        }
        let changed = self.presentation_studio.project != before_frame_edit;
        let pointer_down = ui.input(|input| input.pointer.primary_down());
        if changed && pointer_down {
            if self.presentation_studio.timeline_drag_before.is_none() {
                self.presentation_studio.timeline_drag_before = Some(before_frame_edit);
            }
        } else if !pointer_down {
            if let Some(previous) = self.presentation_studio.timeline_drag_before.take() {
                self.presentation_remember_undo(previous);
            } else if changed {
                self.presentation_remember_undo(before_frame_edit);
            }
        }
        if changed {
            self.presentation_studio.preview_png = None;
            self.presentation_studio.dirty = true;
            self.presentation_studio.autosave_pending = true;
            self.presentation_close_checkpointed = false;
        }
        for command in std::mem::take(&mut pending_command_actions) {
            self.dispatch_command(command, ui.ctx());
        }
        if self.presentation_studio.dirty {
            ui.small("Unsaved changes");
        }
        if let Some(dependency) = pending_speech_install {
            self.start_speech_dependency_install(ui.ctx(), dependency);
        }
        if let Some(clip_id) = pending_subtitle_transcription {
            if self.presentation_studio.selected_clip.as_deref() == Some(clip_id.as_str()) {
                self.dispatch_command(crate::workspace::CommandId::GenerateSubtitles, ui.ctx());
            }
        }
        if let Some(path) = pending_subtitle_import {
            self.import_presentation_subtitles(&path);
        }
        if let Some((path, format)) = pending_subtitle_export {
            self.export_presentation_subtitles(&path, format);
        }
        let stored_zoom = self.workspace_preferences.timeline_zoom;
        let stored_height = self.workspace_preferences.timeline_track_height;
        if stored_zoom
            != self
                .presentation_studio
                .timeline_zoom
                .round()
                .clamp(10.0, 180.0) as u16
            || stored_height
                != self
                    .presentation_studio
                    .timeline_track_height
                    .round()
                    .clamp(24.0, 120.0) as u16
        {
            self.save_workspace_preferences();
        }
    }

    fn inputs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Render inputs");
        ui.label("Engine, level, and the engine resource package are required. Music is required only while BGM is enabled. Separate resource packages and MV are optional.");
        ui.add_space(12.0);
        egui::Grid::new("inputs")
            .spacing([12.0, 14.0])
            .show(ui, |ui| {
                path_row(
                    ui,
                    "Engine ZIP",
                    &mut self.config.engine,
                    &["zip"],
                    false,
                    self.parent.as_deref(),
                );
                path_row(
                    ui,
                    "Level",
                    &mut self.config.level,
                    &["gz", "data", "json", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                path_row(
                    ui,
                    "Skin / SCP",
                    &mut self.config.resources,
                    &["scp", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Particle SCP (optional)",
                    &mut self.config.resource_overrides.particles,
                    &["scp", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Music / BGM",
                    &mut self.config.music,
                    &["mp3", "ogg", "wav", "flac", "m4a"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "MV (optional)",
                    &mut self.config.mv,
                    &["mp4", "webm", "mkv", "mov"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Output MP4",
                    &mut self.config.output,
                    &["mp4"],
                    true,
                    self.parent.as_deref(),
                );
            });
        self.media_acquisition_ui(ui);
        if self.project.is_some() {
            ui.add_space(8.0);
            let mut cover = self
                .project
                .as_ref()
                .and_then(|project| project.inputs.cover.clone())
                .unwrap_or_default();
            egui::Grid::new("project-cover-input").show(ui, |ui| {
                path_row(
                    ui,
                    "Project cover (reference)",
                    &mut cover,
                    &["png"],
                    false,
                    self.parent.as_deref(),
                );
            });
            if let Some(project) = &mut self.project {
                project.inputs.cover = (!cover.as_os_str().is_empty()).then_some(cover);
            }
        }
        ui.add_space(12.0);
        ui.collapsing("Resource names / separate audio and background packages", |ui| {
            ui.label("Defaults come from the engine. Resource names are SCP entry names, not filenames.");
            egui::Grid::new("resources").show(ui, |ui| {
                optional_path_row(ui,"SFX SCP (optional)",&mut self.config.resource_overrides.effects,&["scp","zip"],false, self.parent.as_deref());
                optional_path_row(ui,"Background SCP (optional)",&mut self.config.resource_overrides.background,&["scp","zip"],false, self.parent.as_deref());
            });
            for (label,value) in [("Skin resource name",&mut self.config.skin),("Particle resource name",&mut self.config.resource_overrides.particle_name)] { ui.horizontal(|ui| {ui.label(label); let mut text=value.clone().unwrap_or_default(); if ui.text_edit_singleline(&mut text).changed() {*value=(!text.is_empty()).then_some(text);} }); }
        });
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Render range");
        ui.horizontal(|ui| {
            ui.label("Start (Watch seconds)");
            ui.add(
                egui::DragValue::new(&mut self.config.start_time)
                    .speed(0.1)
                    .range(0.0..=86400.0),
            );
            ui.checkbox(&mut self.config.whole_chart, "Whole Chart");
        });
        ui.add_enabled_ui(!self.config.whole_chart, |ui| {
            ui.horizontal(|ui| {
                ui.label("Duration (seconds)");
                ui.add(
                    egui::DragValue::new(&mut self.config.duration)
                        .speed(0.1)
                        .range(0.01..=86400.0),
                );
            });
        });
        if self.config.whole_chart {
            ui.label("Uses the existing chart-end inference. Manual duration is disabled.");
        }
        ui.add_space(16.0);
        ui.heading("Video");
        ui.horizontal(|ui| {
            ui.label("Watch execution").on_hover_text("Sono VM / Interpreter is the default compatibility path. Sono-GCC optionally compiles eligible regions; unsupported regions continue in the interpreter. A speedup is not guaranteed.");
            ui.selectable_value(
                &mut self.config.execution_mode,
                crate::sono_gcc::WatchExecutionMode::Interpreter,
                "Sono VM / Interpreter",
            );
            ui.selectable_value(
                &mut self.config.execution_mode,
                crate::sono_gcc::WatchExecutionMode::SonoGcc,
                "Sono-GCC",
            );
        });
        ui.horizontal(|ui| {
            ui.label("Approximate Watch operations")
                .on_hover_text("Stack operations, Random/RandomInteger, and StreamSet use bounded compatibility behavior whose exact Sonolus-client parity is unverified.");
            egui::ComboBox::from_id_salt("watch_compatibility_policy")
                .selected_text(match self.config.watch_compatibility_policy {
                    crate::runtime::WatchCompatibilityPolicy::Permissive => "Permissive",
                    crate::runtime::WatchCompatibilityPolicy::Strict => "Strict",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.watch_compatibility_policy,
                        crate::runtime::WatchCompatibilityPolicy::Permissive,
                        "Permissive (warn once per render)",
                    );
                    ui.selectable_value(
                        &mut self.config.watch_compatibility_policy,
                        crate::runtime::WatchCompatibilityPolicy::Strict,
                        "Strict (fail if used)",
                    );
                });
        });
        ui.label(match self.config.watch_compatibility_policy {
            crate::runtime::WatchCompatibilityPolicy::Permissive => {
                "Permissive uses deterministic bounded behavior and records a compatibility warning in the render diagnostics."
            }
            crate::runtime::WatchCompatibilityPolicy::Strict => {
                "Strict rejects these operations before their arguments execute."
            }
        });
        if self.config.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc {
            ui.label("Sono-GCC compiles eligible scalar regions; other paths use Sono VM.");
            ui.checkbox(
                &mut self.config.compile_sono_gcc_on_demand,
                "Allow compilation on demand during export",
            );
            ui.label("Off keeps export responsive while background preparation is incomplete; unsupported paths still use Sono VM.");
        }
        ui.checkbox(
            &mut self.config.parallel_watch_updates,
            "Parallel Watch callback scheduling",
        );
        ui.horizontal(|ui| {
            let mut limited = self.config.watch_workers.is_some();
            ui.checkbox(&mut limited, "Limit Watch workers");
            if limited {
                let mut workers = self.config.watch_workers.unwrap_or(4).clamp(1, 256);
                ui.add(egui::DragValue::new(&mut workers).range(1..=256));
                self.config.watch_workers = Some(workers);
            } else {
                self.config.watch_workers = None;
            }
        });
        if !self.config.parallel_watch_updates {
            ui.label("Uses ordered single-thread Watch execution for compatibility/debugging.");
        }
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.config.width).range(2..=8192));
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.config.height).range(2..=8192));
            ui.label("FPS");
            ui.add(egui::DragValue::new(&mut self.config.fps).range(1..=240));
        });
        ui.heading("MP4 encoding");
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut self.config.video_crf, 0..=51)
                    .text("H.264 quality (CRF; lower is higher quality)"),
            );
            egui::ComboBox::from_label("Encoder speed / compression")
                .selected_text(self.config.video_preset.ffmpeg_name())
                .show_ui(ui, |ui| {
                    for preset in [
                        crate::render::VideoPreset::Ultrafast,
                        crate::render::VideoPreset::Superfast,
                        crate::render::VideoPreset::Veryfast,
                        crate::render::VideoPreset::Faster,
                        crate::render::VideoPreset::Fast,
                        crate::render::VideoPreset::Medium,
                        crate::render::VideoPreset::Slow,
                        crate::render::VideoPreset::Slower,
                        crate::render::VideoPreset::Veryslow,
                    ] {
                        ui.selectable_value(
                            &mut self.config.video_preset,
                            preset,
                            preset.ffmpeg_name(),
                        );
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.config.backend, RenderBackend::Wgpu, "WGPU");
            ui.selectable_value(&mut self.config.backend, RenderBackend::Cpu, "CPU");
            ui.checkbox(&mut self.config.frame_pipeline, "Frame pipeline");
        });
        ui.add_space(16.0);
        ui.heading("MV background");
        ui.add_enabled(
            self.config.mv.is_some(),
            egui::Checkbox::new(&mut self.config.mv_enabled, "Use selected MV"),
        );
        ui.add_enabled(
            self.config.mv.is_some() && self.config.mv_enabled,
            egui::Slider::new(&mut self.config.mv_background, 0.0..=100.0).text("Background %"),
        );
        if ui
            .add_enabled(
                self.config.mv.is_some() && self.config.mv_enabled,
                egui::Checkbox::new(
                    &mut self.config.apply_engine_background_rotation,
                    "Apply engine-driven background rotation",
                ),
            )
            .changed()
        {
            if let Some(project) = &mut self.project {
                project.overrides.apply_engine_background_rotation =
                    Some(self.config.apply_engine_background_rotation);
            }
        }
        ui.label(
            "Presentation brightness is applied before gameplay. When enabled, the engine's Runtime Background transform also affects selected MV footage; gameplay, HUD, and watermark stay in screen space.",
        );
        ui.add_space(16.0);
        ui.heading("Layers");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.layers.particles, "Particles");
            ui.checkbox(&mut self.config.layers.sfx, "SFX");
            ui.checkbox(&mut self.config.layers.bgm, "BGM");
        });
        ui.collapsing("Validation", |ui| {
            ui.checkbox(
                &mut self.config.validate_determinism,
                "Full determinism preflight (additional Watch playback)",
            );
            ui.checkbox(
                &mut self.config.profile,
                "Collect pipeline and workload diagnostics",
            );
            ui.add_enabled_ui(self.config.profile, |ui| {
                ui.checkbox(&mut self.config.profile_frames, "Log each profiled frame");
            });
        });
        ui.collapsing("Compiler cache", |ui| {
            ui.label("Sono-GCC's persistent cache is reusable compiled output. Clearing it does not change render results; later use recompiles it.");
            ui.label("For entry details, integrity validation, size retention, and process-memory caches, use the Cache manager tab.");
            if ui.add_enabled(
                self.job.is_none() && self.engine_job.is_none(),
                egui::Button::new("Clear Sono-GCC cache..."),
            ).clicked() {
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Clear Sono-GCC cache")
                    .set_description("Remove Sono-Renderer's cached compiled engine artifacts? They will be rebuilt when needed.")
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if confirmed == rfd::MessageDialogResult::Yes {
                    match crate::sono_gcc::clear_persistent_cache() {
                        Ok(true) => {
                            self.summary = "Sono-GCC persistent cache cleared.".into();
                            self.error = false;
                        }
                        Ok(false) => {
                            self.summary = "Sono-GCC persistent cache was already empty.".into();
                            self.error = false;
                        }
                        Err(error) => {
                            self.summary = format!("Could not clear Sono-GCC cache: {error:#}");
                            self.error = true;
                        }
                    }
                }
            }
        });
    }
    fn cache_manager_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Generated cache manager");
        ui.label("This manager only touches regenerable Sono-GCC files and process-local decode/texture/program caches.");
        ui.label("Profiles, settings, queued-job history, source assets, rendered videos, Cargo build output, and artifacts/ evidence are not managed or deleted here.");
        ui.horizontal(|ui| {
            if ui.button("Refresh inventory").clicked() {
                match crate::cache_manager::inventory() {
                    Ok(inventory) => {
                        self.cache_inventory = Some(inventory);
                        self.cache_manager_error = None;
                    }
                    Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                }
            }
            if ui.button("Validate all disk entries").clicked() {
                match crate::cache_manager::validate_all() {
                    Ok(entries) => {
                        let validation_error = entries
                            .iter()
                            .find(|entry| {
                                entry.status != crate::cache_manager::CacheEntryStatus::Valid
                            })
                            .map(|entry| format!("{}: {}", entry.id, entry.diagnostic.as_deref().unwrap_or("not valid")));
                        self.cache_manager_error = validation_error;
                        match crate::cache_manager::inventory() {
                            Ok(mut inventory) => {
                                inventory.disk_entries = entries;
                                self.cache_inventory = Some(inventory);
                            }
                            Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                        }
                    }
                    Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                }
            }
            let selected = self.cache_selection.clone();
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("Remove selected..."))
                .clicked()
            {
                let id = selected.unwrap();
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Remove generated cache entry")
                    .set_description(format!("Remove only cache entry {id}? It will be regenerated when needed. User files and render outputs are not included."))
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if confirmed == rfd::MessageDialogResult::Yes {
                    match crate::cache_manager::remove(&id) {
                        Ok(_) => {
                            self.cache_selection = None;
                            self.cache_manager_error = None;
                            self.cache_inventory = crate::cache_manager::inventory().ok();
                        }
                        Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                    }
                }
            }
            if ui.button("Clear inactive caches...").clicked() {
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Clear generated caches")
                    .set_description("Remove inactive Sono-GCC and chart tempo-analysis cache entries, and clear safe process-local cache references. Profiles, settings, job history, source assets, rendered videos, and retained artifacts are not touched.")
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if confirmed == rfd::MessageDialogResult::Yes {
                    match crate::cache_manager::clear(true) {
                        Ok(report) => {
                            self.cache_manager_error = (report.skipped_active > 0).then(|| format!("Cleared {} entries; {} active entries were left intact.", report.removed_entries, report.skipped_active));
                            self.cache_inventory = crate::cache_manager::inventory().ok();
                        }
                        Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Manual cache size target (GiB)");
            ui.add(egui::DragValue::new(&mut self.cache_limit_gib).range(1..=4096));
            if ui.button("Prune oldest-use entries...").clicked() {
                let confirmed = rfd::MessageDialog::new()
                    .set_title("Prune generated cache entries")
                    .set_description(format!("Remove oldest-use inactive cache entries until the managed disk cache is at most {} GiB. Active entries are preserved; no automatic pruning is enabled.", self.cache_limit_gib))
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show();
                if confirmed == rfd::MessageDialogResult::Yes {
                    let target = self.cache_limit_gib.saturating_mul(1024 * 1024 * 1024);
                    match crate::cache_manager::prune_to(target) {
                        Ok(report) => {
                            self.cache_manager_error = (report.skipped_active > 0).then(|| format!("Pruned {} entries; {} active entries were preserved.", report.removed_entries, report.skipped_active));
                            self.cache_inventory = crate::cache_manager::inventory().ok();
                        }
                        Err(error) => self.cache_manager_error = Some(format!("{error:#}")),
                    }
                }
            }
        });
        if let Some(error) = &self.cache_manager_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let Some(inventory) = &self.cache_inventory else {
            ui.label("Select Refresh inventory to inspect this application's cache. No scan runs during startup or rendering.");
            return;
        };
        ui.separator();
        ui.label(format!(
            "Managed disk cache: {} across {} entries",
            format_cache_bytes(inventory.total_disk_bytes),
            inventory
                .regenerable_disk
                .iter()
                .map(|cache| cache.entries)
                .sum::<usize>()
        ));
        for cache in &inventory.regenerable_disk {
            ui.small(format!(
                "{}: {} entries, {}",
                cache.category,
                cache.entries,
                format_cache_bytes(cache.bytes)
            ));
        }
        if inventory.disk_entries.is_empty() {
            ui.label("No persistent Sono-GCC entries were found for this platform/user.");
        }
        let selection = self.cache_selection.clone();
        for entry in inventory.disk_entries.clone() {
            let label = format!(
                "{}  |  {}  |  {}  |  schema {}  |  {:?}",
                entry.id,
                entry.associated_data,
                format_cache_bytes(entry.size_bytes),
                entry
                    .schema
                    .map_or_else(|| "unknown".into(), |schema| schema.to_string()),
                entry.status
            );
            if ui
                .selectable_label(selection.as_deref() == Some(entry.id.as_str()), label)
                .clicked()
            {
                self.cache_selection = Some(entry.id.clone());
            }
            ui.indent(entry.id.clone(), |ui| {
                ui.label(format!(
                    "Created: {} | Last used: {} | Target: {}",
                    entry
                        .created_unix_ms
                        .map_or_else(|| "unknown".into(), format_cache_time),
                    entry
                        .last_used_unix_ms
                        .map_or_else(|| "not tracked".into(), format_cache_time),
                    entry.target.as_deref().unwrap_or("unknown")
                ));
                if let Some(diagnostic) = &entry.diagnostic {
                    ui.label(diagnostic);
                }
            });
        }
        ui.separator();
        ui.heading("Process-local cache memory");
        for item in &inventory.memory {
            ui.label(format!(
                "{}: {} entries, {}{}  E{}",
                item.category,
                item.entries,
                item.retained_bytes
                    .map_or_else(|| "size unknown".into(), format_cache_bytes),
                item.limit_bytes.map_or_else(String::new, |limit| format!(
                    " / {} cap",
                    format_cache_bytes(limit)
                )),
                item.notes
            ));
        }
    }
    fn compatibility_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Engine / project compatibility");
        ui.label("Static findings describe package structure and implemented code. A listed Watch operation is not proof that its node is reachable or executes.");
        ui.checkbox(
            &mut self.compatibility_dynamic,
            "Run bounded dynamic Watch preprocess probe",
        );
        ui.label("The optional probe uses the Sono VM, single-threaded, and exercises preprocess only. It skips inputs above fixed graph/entity limits; it does not render or export.");
        if ui
            .button("Analyze current engine, resources, and level")
            .clicked()
        {
            match crate::compatibility::analyze_with_options(
                &self.config.engine,
                &self.config.resources,
                &self.config.level,
                self.compatibility_dynamic,
            ) {
                Ok(report) => {
                    self.compatibility_report = Some(report);
                    self.compatibility_error = None;
                }
                Err(error) => {
                    self.compatibility_report = None;
                    self.compatibility_error = Some(format!("{error:#}"));
                }
            }
        }
        if let Some(error) = &self.compatibility_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let Some(report) = &self.compatibility_report else {
            return;
        };
        ui.separator();
        ui.label(format!(
            "{} distinct Watch operation names · {} level entities · report schema {}",
            report.operations.len(),
            report.level_entity_count,
            report.schema_version
        ));
        for assessment in &report.assessments {
            let status = assessment.status;
            let color = match status {
                crate::compatibility::CompatibilityStatus::Supported
                | crate::compatibility::CompatibilityStatus::DynamicallyVerified => {
                    egui::Color32::LIGHT_GREEN
                }
                crate::compatibility::CompatibilityStatus::Unsupported => egui::Color32::LIGHT_RED,
                crate::compatibility::CompatibilityStatus::Partial
                | crate::compatibility::CompatibilityStatus::FallbackRequired => {
                    egui::Color32::YELLOW
                }
                crate::compatibility::CompatibilityStatus::UnknownRuntimeDependent => {
                    egui::Color32::LIGHT_GRAY
                }
            };
            ui.collapsing(
                format!("{}  E{}", assessment.category, status.label()),
                |ui| {
                    ui.colored_label(color, status.label());
                    for evidence in &assessment.evidence {
                        ui.label(format!("Evidence: {evidence}"));
                    }
                    for warning in &assessment.warnings {
                        ui.colored_label(egui::Color32::YELLOW, format!("Warning: {warning}"));
                    }
                    if let Some(fallback) = &assessment.fallback {
                        ui.label(format!("Fallback: {fallback}"));
                    }
                    for limitation in &assessment.limitations {
                        ui.label(format!("Limitation: {limitation}"));
                    }
                },
            );
        }
        ui.collapsing("Operation coverage", |ui| {
            for operation in &report.operations {
                ui.label(format!("{}  EVM {:?}; Sono-GCC eligible {}; VM-routed (support classified separately) {}; {} graph occurrence(s), example nodes {:?}", operation.function, operation.vm_implementation, operation.gcc_eligible_occurrences, operation.vm_fallback_occurrences, operation.occurrence_count, operation.example_node_indices));
            }
        });
        ui.collapsing("Selected resource validation", |ui| {
            for resource in &report.required_resource_status {
                ui.label(format!(
                    "{}/{}  E{:?}{}",
                    resource.category,
                    resource.name,
                    resource.integrity,
                    resource
                        .detail
                        .as_ref()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                ));
            }
            for issue in &report.resource_integrity_issues {
                ui.colored_label(egui::Color32::YELLOW, issue);
            }
        });
        ui.collapsing("Structural findings", |ui| {
            if report.structural_findings.is_empty() {
                ui.label("No static structural issues found.");
            }
            for finding in &report.structural_findings {
                let color = match finding.severity {
                    crate::compatibility::FindingSeverity::Error => egui::Color32::LIGHT_RED,
                    crate::compatibility::FindingSeverity::Warning => egui::Color32::YELLOW,
                    crate::compatibility::FindingSeverity::Information => egui::Color32::LIGHT_GRAY,
                };
                ui.colored_label(color, format!("{}: {}", finding.code, finding.message));
            }
        });
        if let Some(dynamic) = &report.dynamic_validation {
            ui.collapsing("Dynamic validation scope", |ui| {
                ui.label(format!(
                    "Outcome: {:?}; {} entities, {} Watch nodes",
                    dynamic.outcome, dynamic.level_entities, dynamic.watch_graph_nodes
                ));
                ui.label(format!(
                    "Safety caps: {} runtime entities; {} evaluations per callback",
                    dynamic.runtime_entity_limit, dynamic.callback_evaluation_limit
                ));
                for step in &dynamic.exercised {
                    ui.label(format!("Exercised: {step}"));
                }
                if let Some(detail) = &dynamic.detail {
                    ui.colored_label(egui::Color32::YELLOW, detail);
                }
            });
        }
    }

    fn render_diagnostics_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Render diagnostics");
        ui.label("This view analyzes the saved performance record and job events. Exclusive phases partition wall time; concurrent lane attributions overlap and are shown separately.");
        let jobs = self
            .job_manager
            .as_ref()
            .and_then(|manager| manager.jobs().ok())
            .unwrap_or_default();
        if !jobs.is_empty() {
            if self
                .selected_job
                .as_ref()
                .is_none_or(|selected| !jobs.iter().any(|job| &job.id == selected))
            {
                self.selected_job = jobs
                    .iter()
                    .max_by_key(|job| {
                        job.finished_unix_time_ms
                            .unwrap_or(job.submitted_unix_time_ms)
                    })
                    .map(|job| job.id.clone());
            }
            egui::ComboBox::from_label("Render job")
                .selected_text(self.selected_job.as_deref().unwrap_or("Select a job"))
                .show_ui(ui, |ui| {
                    for job in &jobs {
                        ui.selectable_value(
                            &mut self.selected_job,
                            Some(job.id.clone()),
                            format!("{} · {:?} · attempt {}", job.id, job.state, job.attempt),
                        );
                    }
                });
        }

        let selected_job = self
            .selected_job
            .as_deref()
            .and_then(|id| jobs.iter().find(|job| job.id == id));
        let report = selected_job.and_then(|job| {
            job.diagnostic_report.clone().or_else(|| {
                let performance_id = job
                    .outcome
                    .as_ref()
                    .and_then(|outcome| outcome.performance_record_id.as_deref())?;
                let store = self.performance_store.as_ref()?;
                store.get(performance_id).ok().map(|record| {
                    crate::render_diagnostics::RenderDiagnosticRecord::from_performance(&record)
                })
            })
        });
        let report = report.or_else(|| self.last_diagnostic.clone()).or_else(|| {
            if jobs.is_empty() {
                self.performance_records.first().map(|record| {
                    self.diagnostic_store
                        .as_ref()
                        .and_then(|store| store.get_for_performance(&record.id).ok())
                        .unwrap_or_else(|| {
                            crate::render_diagnostics::RenderDiagnosticRecord::from_performance(
                                record,
                            )
                        })
                })
            } else {
                None
            }
        });

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    report.is_some(),
                    egui::Button::new("Export diagnostic JSON"),
                )
                .clicked()
            {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Export redacted render diagnostic")
                    .add_filter("JSON report", &["json"])
                    .set_file_name("sono-renderer-diagnostic.json");
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let (Some(path), Some(report)) = (dialog.save_file(), report.as_ref()) {
                    match crate::render_diagnostics::save_diagnostic_atomic(&path, report) {
                        Ok(()) => self.performance_error = None,
                        Err(error) => self.performance_error = Some(format!("{error:#}")),
                    }
                }
            }
            if ui.button("Refresh jobs and performance records").clicked() {
                self.refresh_performance_history();
            }
        });
        if let Some(error) = &self.performance_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(report) = &report {
            render_diagnostic_record_ui(ui, report);
        } else if selected_job.is_some() {
            ui.label("This job has not completed or does not have a diagnostic record yet.");
        } else {
            ui.label("No completed renders are available yet. Completed jobs and performance records appear here.");
        }
    }

    fn start_media_acquisition_work<F>(
        &mut self,
        ctx: &egui::Context,
        control: crate::media_acquisition::AcquisitionControl,
        work: F,
    ) where
        F: FnOnce() -> Result<MediaAcquisitionOutcome> + Send + 'static,
    {
        if self.media_acquisition_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = catch_export_panic(work).map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.media_acquisition_job = Some(MediaAcquisitionJob {
            result: rx,
            worker,
            control,
        });
        self.media_acquisition_error = None;
    }

    fn inspect_media_url(&mut self, ctx: &egui::Context) {
        let url = self.media_acquisition_url.clone();
        let control = crate::media_acquisition::AcquisitionControl::default();
        let worker_control = control.clone();
        let progress = self.media_acquisition_progress.clone();
        *progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            crate::media_acquisition::AcquisitionProgress {
                stage: crate::media_acquisition::AcquisitionStage::Inspecting,
                detail: "Inspecting source metadata".into(),
                ..Default::default()
            };
        self.media_acquisition_inspected_url = None;
        self.media_acquisition_info = None;
        self.start_media_acquisition_work(ctx, control, move || {
            let info = crate::media_acquisition::inspect_url(&url, &worker_control, &progress)?;
            Ok(MediaAcquisitionOutcome::Inspected(info))
        });
    }

    fn download_media_url(&mut self, ctx: &egui::Context) {
        let url = self.media_acquisition_url.clone();
        let mode = self.media_acquisition_mode;
        let destination = self
            .project_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|parent| parent.join("media"))
            .unwrap_or_else(crate::media_acquisition::default_media_directory);
        let control = crate::media_acquisition::AcquisitionControl::default();
        let worker_control = control.clone();
        let progress = self.media_acquisition_progress.clone();
        *progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            crate::media_acquisition::AcquisitionProgress {
                stage: crate::media_acquisition::AcquisitionStage::Waiting,
                detail: "Preparing one media item".into(),
                ..Default::default()
            };
        self.start_media_acquisition_work(ctx, control, move || {
            let media = crate::media_acquisition::acquire(
                &url,
                mode,
                &destination,
                &worker_control,
                &progress,
            )?;
            Ok(MediaAcquisitionOutcome::Downloaded { mode, media })
        });
    }

    fn poll_media_acquisition(&mut self) {
        let result =
            self.media_acquisition_job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("media acquisition worker ended without a result".into()))
                    }
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        let Some(result) = result else {
            return;
        };
        let Some(job) = self.media_acquisition_job.take() else {
            return;
        };
        let canceled = job.control.is_cancelled();
        let _ = job.worker.join();
        match result {
            Ok(MediaAcquisitionOutcome::Inspected(info)) => {
                self.media_acquisition_info = Some(info);
                self.media_acquisition_inspected_url =
                    Some(self.media_acquisition_url.trim().into());
                self.media_acquisition_error = None;
                let mut progress = self
                    .media_acquisition_progress
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                progress.stage = crate::media_acquisition::AcquisitionStage::Complete;
                progress.detail = "Source is a single supported item".into();
            }
            Ok(MediaAcquisitionOutcome::Downloaded { mode, media }) => {
                if mode.has_audio() {
                    self.config.music = Some(media.path.clone());
                }
                if mode.has_video() {
                    self.config.mv = Some(media.path.clone());
                }
                if self.project.is_some() {
                    self.sync_project_from_config();
                    if let Some(path) = self.project_path.clone() {
                        self.save_project_as(path);
                    } else {
                        self.project_error = Some("Acquired media is assigned to this project in memory; save the project to persist its new file references.".into());
                    }
                }
                self.media_acquisition_error = None;
                let mut progress = self
                    .media_acquisition_progress
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                progress.stage = crate::media_acquisition::AcquisitionStage::Complete;
                progress.detail =
                    format!("Ready: {} (SHA-256 {})", media.path.display(), media.sha256);
            }
            Err(error) => {
                let stage = if canceled {
                    crate::media_acquisition::AcquisitionStage::Canceled
                } else {
                    crate::media_acquisition::AcquisitionStage::Failed
                };
                let mut progress = self
                    .media_acquisition_progress
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                progress.stage = stage;
                progress.detail = if canceled {
                    "Acquisition canceled; staging files were discarded".into()
                } else {
                    "Acquisition failed".into()
                };
                self.media_acquisition_error = Some(error);
            }
        }
    }

    fn start_manifest_work<F>(&mut self, ctx: &egui::Context, work: F)
    where
        F: FnOnce() -> Result<ManifestJobOutcome> + Send + 'static,
    {
        if self.manifest_job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::spawn(move || {
            let result = catch_export_panic(work).map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.manifest_job = Some(ManifestJob { result: rx, worker });
        self.manifest_error = None;
    }

    fn open_manifest_dialog(&mut self, ctx: &egui::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open render manifest")
            .add_filter("Sono-Renderer manifest", &["json", "sonomanifest"]);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        self.manifest = None;
        self.manifest_report = None;
        self.manifest_comparison = None;
        self.manifest_asset_overrides.clear();
        self.manifest_path = Some(path.clone());
        self.start_manifest_work(ctx, move || {
            let manifest = crate::render_manifest::RenderManifest::load(&path)?;
            let report = manifest.verify(&Default::default())?;
            Ok(ManifestJobOutcome::Loaded {
                path,
                manifest: Box::new(manifest),
                report: Box::new(report),
            })
        });
    }

    fn verify_open_manifest(&mut self, ctx: &egui::Context) {
        let (Some(manifest), overrides) =
            (self.manifest.clone(), self.manifest_asset_overrides.clone())
        else {
            return;
        };
        self.start_manifest_work(ctx, move || {
            Ok(ManifestJobOutcome::Verified(manifest.verify(&overrides)?))
        });
    }

    fn export_manifest_dialog(
        &mut self,
        ctx: &egui::Context,
        entry: crate::render_history::RenderHistoryRecord,
    ) {
        let filename = format!("{}.sonomanifest.json", entry.id);
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export reproducible render manifest")
            .add_filter("JSON manifest", &["json"])
            .set_file_name(filename);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let performance_store = self.performance_store.clone();
        self.start_manifest_work(ctx, move || {
            let performance = entry
                .performance_record_id
                .as_deref()
                .and_then(|id| performance_store.as_ref()?.get(id).ok());
            let manifest = crate::render_manifest::RenderManifest::from_history_with_performance(
                &entry,
                performance.as_ref(),
            )?;
            let sha256 = manifest.manifest_sha256.clone();
            manifest.save_atomic(&path)?;
            Ok(ManifestJobOutcome::Exported { path, sha256 })
        });
    }

    fn reproduce_manifest_to(&mut self, ctx: &egui::Context, output: PathBuf) {
        let Some(manifest) = self.manifest.clone() else {
            self.manifest_error = Some("Manifest or render queue is unavailable.".into());
            return;
        };
        if self.job_manager.is_none() {
            self.manifest_error = Some("Manifest or render queue is unavailable.".into());
            return;
        }
        let overrides = self.manifest_asset_overrides.clone();
        // Input hashing and reconstruction can be slow, so prepare it in the
        // background. Submission itself goes through the app-owned manager in
        // `poll`; a temporary manager would cancel its workers when dropped.
        self.start_manifest_work(ctx, move || {
            let config = manifest.build_reproduction_config(&overrides, &output)?;
            Ok(ManifestJobOutcome::ReproductionPrepared {
                config: Box::new(config),
                manifest: Box::new(manifest),
            })
        });
    }

    fn compare_history_to_manifest(
        &mut self,
        ctx: &egui::Context,
        entry: crate::render_history::RenderHistoryRecord,
    ) {
        let Some(manifest) = self.manifest.clone() else {
            return;
        };
        let performance_store = self.performance_store.clone();
        self.start_manifest_work(ctx, move || {
            let performance = entry
                .performance_record_id
                .as_deref()
                .and_then(|id| performance_store.as_ref()?.get(id).ok());
            let current = crate::render_manifest::RenderManifest::from_history_with_performance(
                &entry,
                performance.as_ref(),
            )?;
            Ok(ManifestJobOutcome::Compared(
                crate::render_manifest::compare(&manifest, &current)?,
            ))
        });
    }

    fn render_manifest_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Render manifest");
        ui.label("A manifest records the resolved execution and hashes its source assets. It does not package those assets or depend on the mutable project/profile/cache.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.manifest_job.is_none(),
                    egui::Button::new("Open Manifest…"),
                )
                .clicked()
            {
                self.open_manifest_dialog(ui.ctx());
            }
            if ui
                .add_enabled(
                    self.manifest.is_some() && self.manifest_job.is_none(),
                    egui::Button::new("Verify Local Inputs"),
                )
                .clicked()
            {
                self.verify_open_manifest(ui.ctx());
            }
            if self.manifest_job.is_some() {
                ui.spinner();
                ui.label("Hashing or preparing manifest operation…");
            }
            if let Some(path) = &self.manifest_path {
                ui.small(path.display().to_string());
            }
        });
        if let Some(error) = &self.manifest_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let Some(manifest) = self.manifest.clone() else {
            ui.separator();
            return;
        };
        ui.label(format!(
            "Manifest {} | renderer {} | {} requested / {} effective",
            manifest.manifest_sha256,
            manifest.renderer.version,
            manifest.execution.requested_backend,
            manifest.execution.effective_backend
        ));
        ui.label(format!(
            "Source history {} | profile {} | target {} {}",
            manifest
                .provenance
                .history_id
                .as_deref()
                .unwrap_or("unknown"),
            manifest
                .provenance
                .profile_name
                .as_deref()
                .unwrap_or("none"),
            manifest.execution.target_os,
            manifest.execution.target_architecture
        ));
        if let Some(report) = self.manifest_report.clone() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Inputs/config: {} | exact backend: {} | reference: {}",
                    if report.can_reproduce_semantic_inputs {
                        "MATCH"
                    } else {
                        "NEEDS ATTENTION"
                    },
                    if report.can_reproduce_exact_backend {
                        "available"
                    } else {
                        "unavailable"
                    },
                    report.strongest_stored_reference
                ));
                if ui
                    .add_enabled(
                        report.can_reproduce_exact_backend && self.manifest_job.is_none(),
                        egui::Button::new("Reproduce…"),
                    )
                    .clicked()
                {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Choose new output for manifest reproduction")
                        .add_filter("MP4 video", &["mp4"])
                        .set_file_name("reproduction.mp4");
                    if let Some(parent) = self.parent.as_deref() {
                        dialog = dialog.set_parent(parent);
                    }
                    if let Some(output) = dialog.save_file() {
                        self.reproduce_manifest_to(ui.ctx(), output);
                    }
                }
            });
        }
        let inputs = manifest.inputs.clone();
        egui::Grid::new("manifest-input-identities")
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Asset");
                ui.strong("Identity / local candidate");
                ui.strong("Status");
                ui.end_row();
                for input in inputs.iter().filter(|input| input.active) {
                    let status = self
                        .manifest_report
                        .as_ref()
                        .and_then(|report| {
                            report
                                .items
                                .iter()
                                .find(|item| item.component == input.role)
                        })
                        .map(|item| item.status);
                    ui.label(&input.role);
                    ui.vertical(|ui| {
                        ui.small(
                            input
                                .sha256
                                .as_deref()
                                .unwrap_or("identity was not captured"),
                        );
                        if let Some(path) = self
                            .manifest_asset_overrides
                            .get(&input.id)
                            .or(input.path_hint.as_ref())
                        {
                            ui.small(path.display().to_string());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(format!("{status:?}"));
                        if matches!(
                            status,
                            Some(
                                crate::render_manifest::VerificationStatus::Missing
                                    | crate::render_manifest::VerificationStatus::Mismatch
                            )
                        ) && ui.button("Locate…").clicked()
                        {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title(format!("Locate matching {}", input.role));
                            if let Some(parent) = self.parent.as_deref() {
                                dialog = dialog.set_parent(parent);
                            }
                            if let Some(path) = dialog.pick_file() {
                                self.manifest_asset_overrides.insert(input.id.clone(), path);
                                self.verify_open_manifest(ui.ctx());
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        ui.collapsing("Effective configuration snapshot", |ui| {
            if let Ok(json) = serde_json::to_string_pretty(&manifest.effective_config) {
                ui.monospace(json);
            }
        });
        if let Some(comparison) = &self.manifest_comparison {
            ui.collapsing("Last comparison with selected history entry", |ui| {
                ui.label(format!(
                    "Inputs match: {} | effective config match: {} | RGB sequence: {:?} | audio PCM: {:?}",
                    comparison.input_content_matches,
                    comparison.semantic_config_matches,
                    comparison.raw_frame_reference_matches,
                    comparison.audio_reference_matches
                ));
                for difference in &comparison.differences {
                    ui.label(format!("{:?}: {}", difference.category, difference.component));
                }
            });
        }
        ui.separator();
    }

    fn refresh_render_history(&mut self) {
        let Some(store) = self.history_store.clone() else {
            return;
        };
        let page = store.list(&crate::render_history::HistoryQuery {
            text: (!self.history_search.trim().is_empty()).then(|| self.history_search.clone()),
            status: self.history_status,
            engine: (!self.history_engine_filter.trim().is_empty())
                .then(|| self.history_engine_filter.clone()),
            project: (!self.history_project_filter.trim().is_empty())
                .then(|| self.history_project_filter.clone()),
            backend: (!self.history_backend_filter.trim().is_empty())
                .then(|| self.history_backend_filter.clone()),
            limit: Some(100),
            ..Default::default()
        });
        self.history_entries = page.entries;
        self.history_error = page
            .issues
            .first()
            .map(|issue| format!("{}: {}", issue.path.display(), issue.message));
    }

    fn render_history_ui(&mut self, ui: &mut egui::Ui) {
        use crate::render_history::HistoryStatus;
        self.render_manifest_ui(ui);
        ui.heading("Render history");
        ui.label("Durable attempt records are independent of the active queue. Render Again creates a new job from the saved effective configuration.");
        if let Some(store) = &self.history_store {
            ui.small(format!("History: {}", store.root().display()));
        }
        let old_search = self.history_search.clone();
        let old_status = self.history_status;
        let old_filters = (
            self.history_engine_filter.clone(),
            self.history_project_filter.clone(),
            self.history_backend_filter.clone(),
        );
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add_sized(
                [260.0, 24.0],
                egui::TextEdit::singleline(&mut self.history_search)
                    .hint_text("job, profile, engine, output"),
            );
            ui.add_sized(
                [130.0, 24.0],
                egui::TextEdit::singleline(&mut self.history_engine_filter)
                    .hint_text("engine path"),
            );
            ui.add_sized(
                [130.0, 24.0],
                egui::TextEdit::singleline(&mut self.history_project_filter)
                    .hint_text("project path"),
            );
            ui.add_sized(
                [100.0, 24.0],
                egui::TextEdit::singleline(&mut self.history_backend_filter).hint_text("backend"),
            );
            egui::ComboBox::from_id_salt("history-status-filter")
                .selected_text(
                    self.history_status
                        .map_or("All statuses", |status| match status {
                            HistoryStatus::Completed => "Completed",
                            HistoryStatus::Failed => "Failed",
                            HistoryStatus::Cancelled => "Cancelled",
                        }),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.history_status, None, "All statuses");
                    ui.selectable_value(
                        &mut self.history_status,
                        Some(HistoryStatus::Completed),
                        "Completed",
                    );
                    ui.selectable_value(
                        &mut self.history_status,
                        Some(HistoryStatus::Failed),
                        "Failed",
                    );
                    ui.selectable_value(
                        &mut self.history_status,
                        Some(HistoryStatus::Cancelled),
                        "Cancelled",
                    );
                });
            if ui.button("Refresh").clicked() {
                self.refresh_render_history();
            }
        });
        if old_search != self.history_search
            || old_status != self.history_status
            || old_filters
                != (
                    self.history_engine_filter.clone(),
                    self.history_project_filter.clone(),
                    self.history_backend_filter.clone(),
                )
        {
            self.refresh_render_history();
        }
        if let Some(error) = &self.history_error {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("History note: {error}"));
        }
        let store = self.history_store.clone();
        let entries = self.history_entries.clone();
        let mut selected = self.selected_history_id.clone();
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                if entries.is_empty() {
                    ui.label("No matching render attempts.");
                }
                for entry in &entries {
                    let output = entry
                        .output
                        .path
                        .as_ref()
                        .and_then(|path| path.file_name())
                        .and_then(|name| name.to_str())
                        .unwrap_or("no output");
                    let label = format!(
                        "{:?} | attempt {} | {} | {} | {}",
                        entry.status,
                        entry.attempt,
                        entry.execution_backend,
                        entry.profile_name.as_deref().unwrap_or("no profile"),
                        output
                    );
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut selected, Some(entry.id.clone()), label);
                        if entry.pinned {
                            ui.strong("Pinned");
                        }
                    });
                }
            });
        self.selected_history_id = selected;
        let selected_entry = self
            .selected_history_id
            .as_deref()
            .and_then(|id| entries.iter().find(|entry| entry.id == id))
            .cloned();
        if let Some(entry) = selected_entry {
            ui.separator();
            ui.heading(format!("{:?}  E{}", entry.status, entry.id));
            ui.label(format!(
                "Job {} • attempt {} • completed Unix ms {}",
                entry.job_id, entry.attempt, entry.completed_unix_time_ms
            ));
            ui.label(format!(
                "Profile: {} ({})",
                entry.profile_name.as_deref().unwrap_or("none"),
                entry.profile_id.as_deref().unwrap_or("none")
            ));
            ui.label(format!(
                "Renderer {} • {} • {} • {}",
                entry.renderer_version,
                entry.operating_system,
                entry.architecture,
                entry.execution_backend
            ));
            if let Some(path) = &entry.project_path {
                ui.label(format!("Project: {}", path.display()));
            }
            if let Some(path) = &entry.output.path {
                ui.label(format!("Output: {}", path.display()));
            }
            ui.horizontal(|ui| {
                if ui.button("Render Again").clicked() {
                    self.prepare_history_rerender(entry.clone());
                }
                if ui
                    .add_enabled(
                        self.manifest_job.is_none(),
                        egui::Button::new("Export Render Manifest…"),
                    )
                    .clicked()
                {
                    self.export_manifest_dialog(ui.ctx(), entry.clone());
                }
                if ui
                    .add_enabled(
                        self.manifest.is_some() && self.manifest_job.is_none(),
                        egui::Button::new("Compare to Open Manifest"),
                    )
                    .clicked()
                {
                    self.compare_history_to_manifest(ui.ctx(), entry.clone());
                }
                if ui
                    .add_enabled(
                        entry.output.path.as_ref().is_some_and(|path| path.exists()),
                        egui::Button::new("Open output"),
                    )
                    .clicked()
                {
                    if let Some(path) = &entry.output.path {
                        open_file(path);
                    }
                }
                if ui
                    .add_enabled(
                        entry.output.path.as_ref().is_some_and(|path| path.exists()),
                        egui::Button::new("Reveal output"),
                    )
                    .clicked()
                {
                    if let Some(parent) = entry.output.path.as_ref().and_then(|path| path.parent())
                    {
                        open_file(parent);
                    }
                }
                if let Some(store) = &store {
                    if ui
                        .button(if entry.pinned { "Unpin" } else { "Pin" })
                        .clicked()
                    {
                        match store.pin(&entry.id, !entry.pinned) {
                            Ok(_) => {
                                self.refresh_render_history();
                                self.history_error = None;
                            }
                            Err(error) => {
                                self.history_error =
                                    Some(format!("Could not update pin: {error:#}"))
                            }
                        }
                    }
                    if ui.button("Delete entry").clicked() {
                        match store.delete(&entry.id) {
                            Ok(()) => {
                                self.selected_history_id = None;
                                self.refresh_render_history();
                            }
                            Err(error) => {
                                self.history_error =
                                    Some(format!("Could not delete entry: {error:#}"))
                            }
                        }
                    }
                }
                if ui.button("Export JSON...").clicked() {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Export render history entry")
                        .add_filter("JSON", &["json"])
                        .set_file_name(format!("{}.json", entry.id));
                    if let Some(parent) = self.parent.as_deref() {
                        dialog = dialog.set_parent(parent);
                    }
                    if let Some(path) = dialog.save_file() {
                        if let Err(error) =
                            crate::render_history::save_history_export_atomic(&path, &entry)
                        {
                            self.history_error =
                                Some(format!("Could not export history entry: {error:#}"));
                        }
                    }
                }
            });
            if let Some(error) = &entry.error {
                ui.colored_label(egui::Color32::LIGHT_RED, format!("Final error: {error}"));
            }
            if let Some(cache) = &entry.cache_state {
                ui.label(format!("Cache state: {cache}"));
            }
            if let Some(fallback) = &entry.fallback {
                ui.colored_label(egui::Color32::YELLOW, fallback);
            }
            for warning in &entry.warnings {
                ui.colored_label(egui::Color32::YELLOW, warning);
            }
            ui.collapsing("Inputs and output metadata", |ui| {
                for input in &entry.input_identities {
                    ui.label(format!(
                        "{}  |  {}{}",
                        input.role,
                        input.path.display(),
                        if input.exists { "" } else { " [missing]" }
                    ));
                }
                ui.label(format!(
                    "Output exists: {} • size {:?} bytes",
                    entry.output.exists, entry.output.size_bytes
                ));
            });
            ui.collapsing(
                "Effective configuration snapshot (secrets redacted)",
                |ui| {
                    if let Ok(json) = serde_json::to_string_pretty(&entry.config_snapshot) {
                        ui.monospace(json);
                    }
                },
            );
            if let Some(id) = &entry.performance_record_id {
                ui.collapsing(format!("Performance record: {id}"), |ui| {
                    if let Some(record) = self
                        .performance_store
                        .as_ref()
                        .and_then(|store| store.get(id).ok())
                    {
                        ui.label(format!(
                            "Execution {:.2} ms • queue wait {} • backend {}",
                            record.execution_time_ms,
                            record
                                .queue_wait_time_ms
                                .map_or_else(|| "n/a".into(), |value| format!("{value:.2} ms")),
                            record.effective_backend_status
                        ));
                        for stage in &record.stage_timings {
                            ui.label(format!(
                                "{}: {:.2} ms ({:?})",
                                stage.name, stage.duration_ms, stage.semantics
                            ));
                        }
                        if let Some(other_entry) = entries.iter().find(|other| {
                            other.id != entry.id && other.performance_record_id.is_some()
                        }) {
                            if let (Some(other_id), Some(performance_store)) = (
                                other_entry.performance_record_id.as_deref(),
                                self.performance_store.as_ref(),
                            ) {
                                if let Ok(other_record) = performance_store.get(other_id) {
                                    let comparison =
                                        crate::performance::compare_records(&other_record, &record);
                                    ui.separator();
                                    ui.label(format!(
                                        "Compared with {}: {} ({:+.2} ms, {})",
                                        other_entry.id,
                                        comparison.classification,
                                        comparison.absolute_difference_ms,
                                        comparison.percentage_difference.map_or_else(
                                            || "percentage unavailable".into(),
                                            |percent| format!("{percent:+.2}%")
                                        )
                                    ));
                                    if !comparison.comparable {
                                        ui.colored_label(
                                            egui::Color32::YELLOW,
                                            format!(
                                                "Not comparable: {}",
                                                comparison.differences.join(", ")
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        ui.label("The linked performance record is unavailable or expired.");
                    }
                });
            }
            if let Some(id) = &entry.diagnostic_record_id {
                ui.collapsing(format!("Diagnostic record: {id}"), |ui| {
                    if let Some(report) = self
                        .diagnostic_store
                        .as_ref()
                        .and_then(|store| store.get(id).ok())
                    {
                        render_diagnostic_record_ui(ui, &report);
                    } else {
                        ui.label("The linked diagnostic record is unavailable or expired.");
                    }
                });
            }
        }
        if let (Some(config), Some(source_id)) = (
            self.history_rerender_config.clone(),
            self.history_rerender_source.clone(),
        ) {
            ui.separator();
            ui.heading("Repair inputs for Render Again");
            let pending_missing = crate::render_history::missing_inputs(&config);
            for (role, path) in &pending_missing {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!("Missing {role}: {}", path.display()),
                    );
                    if ui.button("Choose replacement...").clicked() {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title(format!("Select replacement for {role}"));
                        if let Some(parent) = self.parent.as_deref() {
                            dialog = dialog.set_parent(parent);
                        }
                        if let Some(selected) = dialog.pick_file() {
                            if replace_history_input(
                                &mut self.history_rerender_config.as_mut().unwrap(),
                                &role,
                                selected,
                            )
                            .is_err()
                            {
                                self.history_error =
                                    Some(format!("Could not map replacement to {role}"));
                            }
                        }
                    }
                });
            }
            if !pending_missing.is_empty()
                && ui
                    .button("Locate moved project and resolve its relative inputs...")
                    .clicked()
            {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Locate the original Sono-Renderer project")
                    .add_filter("Sono-Renderer project", &["sonoproject", "json"]);
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(project_path) = dialog.pick_file() {
                    match crate::project::RenderProject::load(&project_path) {
                        Ok(project) => {
                            let inputs = project.resolved_inputs(
                                project_path.parent().unwrap_or(std::path::Path::new(".")),
                            );
                            if let Some(config) = self.history_rerender_config.as_mut() {
                                for (role, old_path) in &pending_missing {
                                    let candidate = match role.as_str() {
                                        "engine" => inputs.engine.as_ref(),
                                        "resources" => inputs.resources.as_ref(),
                                        "level" => inputs.level.as_ref(),
                                        "music" => inputs.music.as_ref(),
                                        "MV" => inputs.mv.as_ref(),
                                        "particle resources" => inputs.particle_resources.as_ref(),
                                        "effects resources" => inputs.effects_resources.as_ref(),
                                        "background resources" => {
                                            inputs.background_resources.as_ref()
                                        }
                                        _ => None,
                                    };
                                    if let Some(candidate) =
                                        candidate.filter(|candidate| candidate.is_file())
                                    {
                                        let _ =
                                            replace_history_input(config, role, candidate.clone());
                                    } else {
                                        self.history_error = Some(format!("The relocated project has no available {role} reference for {}", old_path.display()));
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            self.history_error =
                                Some(format!("Could not load relocated project: {error:#}"))
                        }
                    }
                }
            }
            let missing = crate::render_history::missing_inputs(&config);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        missing.is_empty(),
                        egui::Button::new("Queue repaired historical render"),
                    )
                    .clicked()
                {
                    if let Some(entry) = store.as_ref().and_then(|store| store.get(&source_id).ok())
                    {
                        self.submit_history_rerender(entry, config.clone());
                    } else {
                        self.history_error = Some("Original history entry is unavailable".into());
                    }
                }
                if ui.button("Cancel repair").clicked() {
                    self.history_rerender_config = None;
                    self.history_rerender_source = None;
                }
            });
        }
        if let Some(error) = &self.history_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Maximum entries");
            ui.add(egui::DragValue::new(&mut self.history_retention_max).range(1..=100_000));
            ui.checkbox(&mut self.history_keep_failures, "Keep failures");
            ui.checkbox(&mut self.history_keep_pinned, "Keep pinned");
            ui.checkbox(&mut self.history_retention_use_age, "Age limit days");
            if self.history_retention_use_age {
                ui.add(egui::DragValue::new(&mut self.history_retention_age_days).range(1..=3650));
            }
            if ui.button("Save retention / prune").clicked() {
                if let Some(store) = &store {
                    let mut policy = store.retention().unwrap_or_default();
                    policy.max_entries = self.history_retention_max;
                    policy.max_age_days = self
                        .history_retention_use_age
                        .then_some(self.history_retention_age_days);
                    policy.keep_failures = self.history_keep_failures;
                    policy.keep_pinned = self.history_keep_pinned;
                    match store.set_retention(&policy) {
                        Ok(removed) => {
                            self.history_error = None;
                            self.refresh_render_history();
                            self.summary = format!(
                                "History retention saved; removed {removed} eligible entries."
                            );
                        }
                        Err(error) => {
                            self.history_error =
                                Some(format!("Could not save retention: {error:#}"))
                        }
                    }
                }
            }
            if ui.button("Clear unprotected history").clicked() {
                if let Some(store) = &store {
                    match store.clear(true) {
                        Ok(removed) => {
                            self.refresh_render_history();
                            self.summary = format!(
                                "Removed {removed} entries; failures and pinned entries were kept."
                            );
                        }
                        Err(error) => {
                            self.history_error = Some(format!("Could not clear history: {error:#}"))
                        }
                    }
                }
            }
        });
    }

    fn prepare_history_rerender(&mut self, entry: crate::render_history::RenderHistoryRecord) {
        let config = crate::render_history::sanitize_config_snapshot(&entry.config_snapshot);
        if !crate::render_history::missing_inputs(&config).is_empty() {
            self.history_rerender_config = Some(config);
            self.history_rerender_source = Some(entry.id);
            self.history_error = Some(
                "Select replacements for the exact missing inputs before re-rendering.".into(),
            );
            return;
        }
        self.submit_history_rerender(entry, config);
    }

    fn submit_history_rerender(
        &mut self,
        entry: crate::render_history::RenderHistoryRecord,
        mut config: RenderConfig,
    ) {
        if !crate::render_history::missing_inputs(&config).is_empty() {
            self.history_error = Some("Some historical input files are still missing.".into());
            return;
        }
        let output = config.output.clone();
        let needs_output = output.as_ref().is_none_or(|path| {
            path.exists() || !path.parent().unwrap_or(std::path::Path::new(".")).is_dir()
        });
        if needs_output {
            let filename = output
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map(|name| format!("rerender-{name}"))
                .unwrap_or_else(|| "rerender.mp4".into());
            let mut dialog = rfd::FileDialog::new()
                .set_title("Choose output for historical rerender")
                .add_filter("MP4 video", &["mp4"])
                .set_file_name(filename);
            if let Some(parent) = self.parent.as_deref() {
                dialog = dialog.set_parent(parent);
            }
            let Some(path) = dialog.save_file() else {
                return;
            };
            config.output = Some(path);
        }
        let result = (|| -> Result<String> {
            config.validate_export_inputs()?;
            let manager = self
                .job_manager
                .as_ref()
                .context("render queue is unavailable")?;
            manager.enqueue_with_metadata(
                config,
                None,
                entry.profile_id.clone().zip(entry.profile_name.clone()),
                crate::job_manager::RenderJobMetadata {
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
            )
        })();
        match result {
            Ok(job_id) => {
                self.summary = format!("Queued new job {job_id} from history {}.", entry.id);
                self.error = false;
                self.history_error = None;
                self.history_rerender_config = None;
                self.history_rerender_source = None;
                self.selected_job = Some(job_id);
                if let Some(manager) = &self.job_manager {
                    let _ = manager.start();
                }
            }
            Err(error) => {
                self.history_error =
                    Some(format!("Could not create historical rerender: {error:#}"))
            }
        }
    }

    fn performance_lab_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Performance laboratory");
        ui.label("Benchmarks execute the selected renderer inputs into temporary MP4 outputs. Warm-up runs are excluded from measured distributions; Sono-GCC cache state is recorded as observed.");
        let queue_active = self.job_manager.as_ref().is_some_and(|manager| {
            manager
                .jobs()
                .is_ok_and(|jobs| jobs.iter().any(|job| job.state.is_active()))
        });
        let can_benchmark =
            self.performance_job.is_none() && self.engine_job.is_none() && !queue_active;
        ui.horizontal(|ui| {
            ui.label("Warm-ups");
            ui.add(egui::DragValue::new(&mut self.performance_warmups).range(0..=10));
            ui.label("Measured runs");
            ui.add(egui::DragValue::new(&mut self.performance_runs).range(1..=20));
            ui.checkbox(
                &mut self.performance_compare_modes,
                "Compare Sono VM and Sono-GCC",
            );
            ui.checkbox(
                &mut self.performance_measure_overhead,
                "Estimate detailed profiling overhead",
            );
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_benchmark, egui::Button::new("Benchmark current inputs"))
                .clicked()
            {
                self.start_performance_benchmark(ui.ctx());
            }
            if ui.button("Refresh history").clicked() {
                self.refresh_performance_history();
            }
            if let Some(store) = &self.performance_store {
                ui.label(format!("History: {}", store.root().display()));
            }
            if ui
                .add_enabled(
                    self.performance_latest_report.is_some(),
                    egui::Button::new("Export latest benchmark JSON"),
                )
                .clicked()
            {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Export benchmark report")
                    .add_filter("JSON report", &["json"])
                    .set_file_name("sono-renderer-benchmark.json");
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.save_file() {
                    if let Some(report) = self.performance_latest_report.as_ref() {
                        match crate::performance::save_benchmark_report_atomic(&path, report) {
                            Ok(()) => {
                                self.performance_error = None;
                                self.performance_summary =
                                    format!("Benchmark report exported to {}.", path.display());
                            }
                            Err(error) => {
                                self.performance_error = Some(format!("{error:#}"));
                            }
                        }
                    }
                }
            }
        });
        if self.performance_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&self.performance_summary);
            });
        } else {
            ui.label(&self.performance_summary);
        }
        if let Some(error) = &self.performance_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.performance_records.is_empty() {
            ui.label("No completed performance records are available yet.");
            return;
        }
        if self.performance_records.len() >= 2 {
            let newest = &self.performance_records[0];
            let previous = &self.performance_records[1];
            let comparison = crate::performance::compare_records(previous, newest);
            ui.separator();
            ui.heading("Latest comparison");
            ui.label(format!(
                "{} ↁE{}: {} ({:+.2}ms, {})",
                comparison.left_id,
                comparison.right_id,
                comparison.classification,
                comparison.absolute_difference_ms,
                comparison.percentage_difference.map_or_else(
                    || "percentage unavailable".into(),
                    |value| format!("{value:+.2}%")
                )
            ));
            if !comparison.comparable {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("Not comparable: {}", comparison.differences.join(", ")),
                );
            }
        }
        ui.separator();
        ui.heading("Recent records");
        for record in self.performance_records.iter().take(20) {
            let label = format!(
                "{} | {} | {:.3}s | {} frames | {}x{} | cache {}",
                &record.id,
                record.workload.execution_mode,
                record.execution_time_ms / 1000.0,
                record.frames,
                record.workload.resolution[0],
                record.workload.resolution[1],
                record.gcc_cache_state.as_deref().unwrap_or("n/a")
            );
            ui.collapsing(label, |ui| {
                ui.label(format!(
                    "Queue wait: {} | user latency: {} | backend: {}",
                    record.queue_wait_time_ms.map_or_else(|| "not queued".into(), |ms| format!("{ms:.1}ms")),
                    record.total_user_latency_ms.map_or_else(|| "not queued".into(), |ms| format!("{ms:.1}ms")),
                    record.effective_backend_status
                ));
                ui.label(format!(
                    "Renderer {} | {} {} | identity {} / config {}",
                    record.environment.renderer_version,
                    record.environment.operating_system,
                    record.environment.architecture,
                    record.workload.level_fingerprint,
                    record.workload.effective_config_fingerprint
                ));
                ui.label("Exclusive phase spans (these rows partition execution time):");
                for stage in record.stage_timings.iter().filter(|stage| {
                    stage.semantics == crate::performance::StageSemantics::ExclusiveWallSpan
                }) {
                    ui.label(format!("{}: {:.2}ms", stage.name, stage.duration_ms));
                }
                if let Some(pipeline) = &record.pipeline_overlap {
                    ui.label(format!(
                        "Pipeline wall {:.2}ms; overlapping Watch {:.2}ms, render {:.2}ms, output {:.2}ms; queue {}/{}; consumer wait {:.2}ms",
                        pipeline.wall_span_ms,
                        pipeline.producer_watch_ms,
                        pipeline.render_lane_ms,
                        pipeline.output_lane_ms,
                        pipeline.queue_high_water,
                        pipeline.queue_capacity,
                        pipeline.consumer_waiting_ms
                    ));
                }
                if let Some(gcc) = &record.sono_gcc_compilation {
                    ui.label(format!(
                        "Sono-GCC: {} cache lookup {:.2}ms, compile {:.2}ms, native load {:.2}ms, cache write {:.2}ms",
                        gcc.cache_state,
                        gcc.cache_lookup_ms,
                        gcc.compilation_ms,
                        gcc.native_load_ms,
                        gcc.cache_write_ms
                    ));
                }
                let attribution: Vec<_> = record
                    .stage_timings
                    .iter()
                    .filter(|stage| stage.semantics == crate::performance::StageSemantics::OverlappingAttribution)
                    .collect();
                if !attribution.is_empty() {
                    ui.collapsing("Advanced per-frame attribution (overlaps; do not add to phase spans)", |ui| {
                        for stage in attribution {
                            ui.label(format!("{}: {:.2}ms across {} sample(s)", stage.name, stage.duration_ms, stage.samples));
                        }
                    });
                }
            });
        }
    }

    fn regression_lab_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Visual / audio / semantic regression lab");
        ui.label("Regression capture runs the real Watch-to-pixels renderer only when requested. Reference changes use an explicit update action; ordinary runs never bless their output.");
        ui.horizontal(|ui| {
            ui.label("Fixture ID");
            ui.add_sized(
                [220.0, 24.0],
                egui::TextEdit::singleline(&mut self.regression_fixture_id),
            );
            ui.label("Checkpoint time (s)");
            ui.add(
                egui::DragValue::new(&mut self.regression_checkpoint_time)
                    .speed(0.05)
                    .range(0.0..=86400.0),
            );
            ui.label("Timing samples");
            ui.add(egui::DragValue::new(&mut self.regression_timing_samples).range(1..=5));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.regression_compare_vm_gcc, "Capture VM + GCC");
            ui.checkbox(
                &mut self.regression_include_media,
                "Include short media/audio pass (max 120s)",
            );
        });
        ui.horizontal(|ui| {
            ui.label("Reference directory");
            let mut path = self.regression_reference.to_string_lossy().into_owned();
            if ui
                .add_sized([400.0, 24.0], egui::TextEdit::singleline(&mut path))
                .changed()
            {
                self.regression_reference = PathBuf::from(path);
            }
            if ui.button("Browse...").clicked() {
                let mut dialog =
                    rfd::FileDialog::new().set_title("Select regression reference directory");
                if let Some(parent) = self.parent.as_deref() {
                    dialog = dialog.set_parent(parent);
                }
                if let Some(path) = dialog.pick_folder() {
                    self.regression_reference = path;
                }
            }
        });
        let available = self.regression_job.is_none()
            && self.job.is_none()
            && self.performance_job.is_none()
            && self.engine_job.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(available, egui::Button::new("Run against reference")).clicked() {
                self.start_regression(ui.ctx(), false, false);
            }
            if ui.add_enabled(available, egui::Button::new("Update reference...")).clicked() {
                let replace = self.regression_reference.exists();
                let confirmed = if replace {
                    rfd::MessageDialog::new()
                        .set_title("Explicitly update regression reference")
                        .set_description("This will render the current configuration, print the changed layers, and replace the selected marked reference. Inspect the new PNGs before accepting the new baseline.")
                        .set_level(rfd::MessageLevel::Warning)
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show() == rfd::MessageDialogResult::Yes
                } else {
                    true
                };
                if confirmed {
                    self.start_regression(ui.ctx(), true, replace);
                }
            }
            if ui.button("Open reference folder").clicked() && self.regression_reference.is_dir() {
                open_folder(&self.regression_reference);
            }
        });
        if self.regression_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&self.regression_summary);
            });
        } else {
            ui.label(&self.regression_summary);
        }
        if let Some(error) = &self.regression_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(report) = &self.regression_latest_report {
            ui.separator();
            ui.label(format!(
                "{} frame comparison(s); workload comparable: {}. Coverage differences: {}; environment differences: {}",
                report.frames.len(),
                report.comparable,
                if report.coverage_differences.is_empty() { "none".into() } else { report.coverage_differences.join(", ") },
                if report.environment_differences.is_empty() {
                    "none".into()
                } else {
                    report.environment_differences.join(", ")
                }
            ));
            for frame in &report.frames {
                let layers = [
                    &frame.runtime_semantics,
                    &frame.watch_operation_counts,
                    &frame.display_list,
                    &frame.audio_events,
                    &frame.particle_events,
                    &frame.gameplay_ui_state,
                    &frame.watermark_pose,
                    &frame.pixels,
                ];
                let changed = layers
                    .iter()
                    .filter(|layer| layer.changed)
                    .map(|layer| layer.name.as_str())
                    .collect::<Vec<_>>();
                ui.collapsing(
                    format!(
                        "{} {}x{} {}: {}",
                        frame.execution_mode,
                        frame.resolution[0],
                        frame.resolution[1],
                        frame.checkpoint_id,
                        if changed.is_empty() {
                            "matching".into()
                        } else {
                            changed.join(", ")
                        }
                    ),
                    |ui| {
                        ui.label(format!(
                            "Checkpoint step timing: {:?}ms -> {:?}ms ({})",
                            frame.timing.expected_median_ms,
                            frame.timing.current_median_ms,
                            frame.timing.classification
                        ));
                        if let Some(stats) = &frame.pixel_stats {
                            ui.label(format!(
                                "Pixel diff: {} / {} pixels; max channel delta {}; bounds {:?}",
                                stats.changed_pixels,
                                stats.compared_pixels,
                                stats.max_channel_delta,
                                stats.changed_bounds
                            ));
                        }
                        for layer in &layers {
                            if layer.changed {
                                if let Some(detail) = &layer.detail {
                                    ui.label(format!("{}: {detail}", layer.name));
                                }
                            }
                        }
                        for artifact in &frame.artifacts {
                            ui.label(artifact);
                        }
                        ui.horizontal(|ui| {
                            if ui.button("View reference PNG").clicked() {
                                open_file(&frame.reference_png);
                            }
                            if let Some(current) = frame.artifacts.first() {
                                if ui.button("View current PNG").clicked() {
                                    open_file(std::path::Path::new(current));
                                }
                            }
                            if let Some(diff) = frame.artifacts.get(1) {
                                if ui.button("View diff image").clicked() {
                                    open_file(std::path::Path::new(diff));
                                }
                            }
                            if let Some(heatmap) = frame.artifacts.get(2) {
                                if ui.button("View heatmap").clicked() {
                                    open_file(std::path::Path::new(heatmap));
                                }
                            }
                        });
                    },
                );
            }
            for comparison in &report.vm_vs_gcc {
                ui.label(format!(
                    "VM / GCC {} {}: {}{}",
                    comparison.resolution[0],
                    comparison.checkpoint_id,
                    if comparison.comparable {
                        "comparable"
                    } else {
                        "unavailable"
                    },
                    comparison
                        .reason
                        .as_ref()
                        .map_or_else(String::new, |reason| format!("  E{reason}"))
                ));
            }
        }
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        ui.heading("Engine options");
        if self.engine_job.is_some() && self.options.is_empty() {
            ui.spinner();
            ui.label("Reading engine metadata...");
        }
        if self.options.is_empty() {
            ui.label("Select an engine to discover named options.");
        }
        for (index, option) in self.options.iter().enumerate() {
            let def = option["def"].as_f64().unwrap_or(0.0);
            let current = self
                .config
                .level_options
                .iter()
                .find(|v| v.index == index)
                .map_or(def, |v| v.value);
            let mut value = current;
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [210.0, 24.0],
                        egui::Label::new(option_label(
                            option["name"].as_str().unwrap_or("Unnamed option"),
                        )),
                    );
                    match option["type"].as_str().unwrap_or("") {
                        "toggle" => {
                            let mut on = value != 0.0;
                            if ui.checkbox(&mut on, "").changed() {
                                value = if on { 1.0 } else { 0.0 };
                            }
                        }
                        "select" => {
                            egui::ComboBox::from_id_salt("value")
                                .width(180.0)
                                .selected_text(option_label(
                                    option["values"]
                                        .get(if value >= 0.0 && value.fract() == 0.0 {
                                            value as usize
                                        } else {
                                            usize::MAX
                                        })
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Custom"),
                                ))
                                .show_ui(ui, |ui| {
                                    if let Some(values) = option["values"].as_array() {
                                        for (i, name) in values.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut value,
                                                i as f64,
                                                option_label(name.as_str().unwrap_or("Unnamed")),
                                            );
                                        }
                                    }
                                });
                        }
                        "slider" => {
                            if let (Some(min), Some(max)) =
                                (option["min"].as_f64(), option["max"].as_f64())
                            {
                                if min > max {
                                    ui.label("Invalid metadata range; use numeric input");
                                } else {
                                    ui.add(
                                        egui::Slider::new(&mut value, min..=max)
                                            .clamping(egui::SliderClamping::Never)
                                            .step_by(
                                                option["step"]
                                                    .as_f64()
                                                    .filter(|v| *v > 0.0)
                                                    .unwrap_or(0.01),
                                            ),
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                    ui.add(egui::DragValue::new(&mut value).speed(0.1));
                    if ui.small_button("Default").clicked() {
                        value = def;
                    }
                });
            });
            if value != current {
                set_option(&mut self.config, index, value, def);
            }
        }
        ui.separator();
        ui.label("Arbitrary numeric override (values are not constrained by metadata)");
        ui.horizontal(|ui| {
            ui.label("Index");
            ui.add(egui::DragValue::new(&mut self.arbitrary_index));
            ui.label("Value");
            ui.add(egui::DragValue::new(&mut self.arbitrary_value));
            if ui.button("Set override").clicked() {
                set_option(
                    &mut self.config,
                    self.arbitrary_index,
                    self.arbitrary_value,
                    f64::NAN,
                );
            }
        });
        ui.label(format!(
            "{} explicit overrides",
            self.config.level_options.len()
        ));
    }

    fn gcc_status_ui(&mut self, ui: &mut egui::Ui) {
        let gcc_enabled =
            self.config.execution_mode == crate::sono_gcc::WatchExecutionMode::SonoGcc;
        let (line, diagnostic, busy, failed) = if !gcc_enabled {
            (
                "Sono-GCC disabled; renders use Sono VM / Interpreter".to_owned(),
                format!(
                    "Engine: {}\nExecution mode: Sono VM / Interpreter\nBackground Sono-GCC compilation is disabled.",
                    self.config.engine.display()
                ),
                false,
                false,
            )
        } else if !self.config.engine.is_file() {
            (
                "Sono-GCC selected; choose a valid engine file".to_owned(),
                format!(
                    "Engine: {}\nStatus: engine path is empty or does not name a readable file.\nNo background compilation has started.",
                    self.config.engine.display()
                ),
                false,
                false,
            )
        } else {
            match &self.gcc_status {
                GccUiStatus::Disabled => (
                    "Sono-GCC selected; waiting for engine Watch graph".to_owned(),
                    format!(
                        "Engine: {}\nExecution mode: Sono-GCC\nStatus: waiting for the exact Watch graph.",
                        self.config.engine.display()
                    ),
                    false,
                    false,
                ),
                GccUiStatus::Loading => (
                    "Sono-GCC: loading engine and Watch graph".to_owned(),
                    format!(
                        "Engine: {}\nStatus: loading engine metadata and Watch graph. Rendering remains available through Sono VM.",
                        self.config.engine.display()
                    ),
                    true,
                    false,
                ),
                GccUiStatus::CheckingCache {
                    graph_nodes,
                    eligible_operations,
                } => (
                    format!(
                        "Sono-GCC: checking exact-graph cache ({graph_nodes} graph nodes; {eligible_operations} eligible operation nodes)."
                    ),
                    format!(
                        "Engine: {}\nStatus: checking process and persistent exact-graph caches\nWatch graph nodes: {graph_nodes}\nEligible operation nodes: {eligible_operations}\nA cache miss starts compilation in the background. Rendering during lookup or compilation remains available through Sono VM.",
                        self.config.engine.display()
                    ),
                    true,
                    false,
                ),
                GccUiStatus::Compiling {
                    graph_nodes,
                    eligible_operations,
                } => (
                    format!(
                        "Sono-GCC: compiling eligible regions ({graph_nodes} graph nodes; {eligible_operations} eligible operation nodes). Rendering can continue with Sono VM."
                    ),
                    format!(
                        "Engine: {}\nStatus: rustc compilation in progress\nWatch graph nodes: {graph_nodes}\nStatically eligible operation nodes: {eligible_operations}\nProgress detail: rustc does not expose reliable per-region progress, so no fabricated completed-region counter is shown.\nRender behavior: a render requested while this job is active uses automatic Sono VM fallback.",
                        self.config.engine.display()
                    ),
                    true,
                    false,
                ),
                GccUiStatus::Ready(report) => (
                    format!(
                        "Sono-GCC ready: {} regions, {} / {} eligible operation nodes compiled ({:?} cache).",
                        report.compiled_regions,
                        report.compiled_operations,
                        report.eligible_operations,
                        report.cache_hit
                    ),
                    gcc_report_details(&self.config.engine, report, "Ready"),
                    false,
                    false,
                ),
                GccUiStatus::Partial(report) => (
                    format!(
                        "Sono-GCC partial: {} / {} eligible operation nodes compiled; remaining paths use Sono VM.",
                        report.compiled_operations, report.eligible_operations
                    ),
                    gcc_report_details(&self.config.engine, report, "Partial; VM fallback remains active"),
                    false,
                    false,
                ),
                GccUiStatus::Failed {
                    graph_nodes,
                    diagnostic,
                } => (
                    "Sono-GCC compilation unavailable; rendering will use Sono VM fallback".to_owned(),
                    format!(
                        "Engine: {}\nStatus: compilation failed; VM fallback remains active\nWatch graph nodes: {graph_nodes}\nCompiler/backend and runtime ABI: {:?}\nCache identity: unavailable if identity creation failed\nDiagnostic:\n{diagnostic}",
                        self.config.engine.display(),
                        crate::sono_gcc::compiler_identity()
                    ),
                    false,
                    true,
                ),
            }
        };
        ui.horizontal(|ui| {
            if busy {
                ui.spinner();
            }
            ui.colored_label(
                if failed {
                    egui::Color32::LIGHT_RED
                } else if busy {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::LIGHT_BLUE
                },
                line,
            );
            if ui
                .small_button(if self.show_gcc_details {
                    "Hide details"
                } else {
                    "View details"
                })
                .clicked()
            {
                self.show_gcc_details = !self.show_gcc_details;
            }
            if ui.small_button("Copy details").clicked() {
                ui.ctx().copy_text(diagnostic.clone());
            }
        });
        if self.show_gcc_details {
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(diagnostic).monospace())
                            .selectable(true)
                            .wrap(),
                    );
                });
        }
    }
}

fn render_diagnostic_record_ui(
    ui: &mut egui::Ui,
    report: &crate::render_diagnostics::RenderDiagnosticRecord,
) {
    if let Err(error) = report.validate_schema_version() {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            format!("Diagnostic unavailable: {error:#}"),
        );
        return;
    }
    ui.separator();
    ui.heading(format!("{:?} · {}", report.outcome, report.id));
    ui.label(format!(
        "Effective backend: {} · cache: {} · performance record: {}",
        report.effective_backend,
        report.cache_state.as_deref().unwrap_or("not observed"),
        report
            .performance_record_id
            .as_deref()
            .unwrap_or("unavailable")
    ));
    if let Some(workload) = &report.workload {
        ui.label(format!(
            "{}x{} at {} FPS · requested {:?} / {:?}",
            workload.resolution[0],
            workload.resolution[1],
            workload.fps,
            workload.requested_backend,
            workload.requested_execution_mode
        ));
    }
    if let Some(phase) = &report.critical_path.largest_exclusive_phase {
        ui.strong(format!(
            "Largest disjoint phase: {phase} · {:.2} ms · {:.1}% of execution",
            report.critical_path.phase_wall_ms.unwrap_or(0.0),
            report.critical_path.execution_share_percent.unwrap_or(0.0)
        ));
    }
    ui.label(&report.critical_path.limitation);

    for kind in [
        crate::render_diagnostics::FindingKind::MeasuredFact,
        crate::render_diagnostics::FindingKind::ObservedEvent,
        crate::render_diagnostics::FindingKind::DerivedConclusion,
        crate::render_diagnostics::FindingKind::Recommendation,
        crate::render_diagnostics::FindingKind::Unavailable,
    ] {
        let findings: Vec<_> = report
            .findings
            .iter()
            .filter(|finding| finding.kind == kind)
            .collect();
        if findings.is_empty() {
            continue;
        }
        ui.collapsing(format!("{kind:?} ({})", findings.len()), |ui| {
            for finding in findings {
                ui.label(&finding.summary);
                for evidence in &finding.evidence {
                    ui.small(format!("Evidence: {evidence}"));
                }
            }
        });
    }
    if let Some(error) = &report.error {
        ui.colored_label(
            egui::Color32::LIGHT_RED,
            format!(
                "{} · phase {} · failure {}",
                error.subsystem,
                error.phase.as_deref().unwrap_or("unknown"),
                error.failure_kind.as_deref().unwrap_or("unknown")
            ),
        );
        for cause in &error.chain {
            ui.label(format!("Cause: {cause}"));
        }
    }
    if let Some(performance) = &report.performance {
        egui::CollapsingHeader::new("Advanced telemetry")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(format!(
                    "Execution {:.2} ms · queue wait {} · total user latency {}",
                    performance.execution_time_ms,
                    performance.queue_wait_time_ms.map_or_else(|| "not queued".into(), |ms| format!("{ms:.2} ms")),
                    performance.total_user_latency_ms.map_or_else(|| "not queued".into(), |ms| format!("{ms:.2} ms"))
                ));
                ui.label("Disjoint phase spans (sum to execution wall time, within residual accounting):");
                for stage in performance.stage_timings.iter().filter(|stage| {
                    stage.semantics == crate::performance::StageSemantics::ExclusiveWallSpan
                }) {
                    ui.label(format!("{} · {:.2} ms", stage.name, stage.duration_ms));
                }
                let attribution: Vec<_> = performance.stage_timings.iter().filter(|stage| {
                    stage.semantics == crate::performance::StageSemantics::OverlappingAttribution
                }).collect();
                if !attribution.is_empty() {
                    ui.label("Overlapping attributions (do not add these to phase spans):");
                    for stage in attribution {
                        ui.label(format!("{} · {:.2} ms · {} sample(s)", stage.name, stage.duration_ms, stage.samples));
                    }
                }
                if let Some(pipeline) = &performance.pipeline_overlap {
                    ui.label(format!(
                        "Pipeline wall {:.2} ms; producer blocked {:.2} ms; consumer waiting {:.2} ms; queue {}/{}; render workers {}",
                        pipeline.wall_span_ms, pipeline.producer_blocked_ms,
                        pipeline.consumer_waiting_ms, pipeline.queue_high_water,
                        pipeline.queue_capacity, pipeline.render_workers
                    ));
                }
                if let Some(gcc) = &performance.sono_gcc_compilation {
                    ui.label(format!(
                        "Sono-GCC {}: lookup {:.2} ms, compile {:.2} ms, native load {:.2} ms, cache write {:.2} ms",
                        gcc.cache_state, gcc.cache_lookup_ms, gcc.compilation_ms,
                        gcc.native_load_ms, gcc.cache_write_ms
                    ));
                }
                ui.label(format!(
                    "Renderer {} · {} · {} · GPU {}",
                    performance.environment.renderer_version,
                    performance.environment.operating_system,
                    performance.environment.architecture,
                    performance.environment.gpu_identity.as_deref().unwrap_or("not observed")
                ));
            });
    }
}

fn should_precompile(mode: crate::sono_gcc::WatchExecutionMode) -> bool {
    mode == crate::sono_gcc::WatchExecutionMode::SonoGcc
}

fn gcc_engine_message_applies(
    job_engine: &std::path::Path,
    selected_engine: &std::path::Path,
    mode: crate::sono_gcc::WatchExecutionMode,
) -> bool {
    job_engine == selected_engine && should_precompile(mode)
}

fn status_for_gcc_report(report: crate::sono_gcc::SonoGccPrecompileReport) -> GccUiStatus {
    if report.compiled_operations < report.eligible_operations {
        GccUiStatus::Partial(report)
    } else {
        GccUiStatus::Ready(report)
    }
}

fn gcc_report_details(
    engine: &std::path::Path,
    report: &crate::sono_gcc::SonoGccPrecompileReport,
    status: &str,
) -> String {
    format!(
        "Engine: {}\nCompiler backend/runtime ABI: {:?}\nStatus: {status}\nExact graph/cache key: {}\nWatch graph nodes: {}\nCompiled regions: {}\nCompiled eligible operation nodes: {}\nEligible operation nodes: {}\nCache hit: {:?}\nTimings: lookup {:.3}s, compile {:.3}s, native load {:.3}s, cache write {:.3}s, graph identity {:.3}s, disk validation {:.3}s, metadata reconstruction {:.3}s\nCache persistence warning: {}\nFallback: unsupported and unsafe paths continue through Sono VM.",
        engine.display(),
        crate::sono_gcc::compiler_identity(),
        report.cache_key,
        report.graph_nodes,
        report.compiled_regions,
        report.compiled_operations,
        report.eligible_operations,
        report.cache_hit,
        report.timings.cache_lookup.as_secs_f64(),
        report.timings.compilation.as_secs_f64(),
        report.timings.native_load.as_secs_f64(),
        report.timings.cache_write.as_secs_f64(),
        report.timings.graph_identity.as_secs_f64(),
        report.timings.disk_validation.as_secs_f64(),
        report.timings.metadata_reconstruction.as_secs_f64(),
        report.cache_write_error.as_deref().unwrap_or("none")
    )
}
fn option_label(text: &str) -> String {
    text.strip_prefix("##LOCALIZE:")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value["en"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.to_owned())
}

fn set_option(config: &mut RenderConfig, index: usize, value: f64, default: f64) {
    config.level_options.retain(|v| v.index != index);
    if value != default {
        config.level_options.push(LevelOptionValue { index, value });
    }
}

fn event_filter_ui(
    ui: &mut egui::Ui,
    label: &str,
    events: &mut Vec<crate::notifications::RendererEventKind>,
) {
    ui.label(label);
    for kind in crate::notifications::RendererEventKind::ALL {
        let mut selected = events.contains(&kind);
        if ui.checkbox(&mut selected, kind.label()).changed() {
            events.retain(|event| *event != kind);
            if selected {
                events.push(kind);
            }
        }
    }
}

fn event_templates_ui(
    ui: &mut egui::Ui,
    templates: &mut crate::notifications::EventTemplates,
    discord: bool,
) {
    for kind in crate::notifications::RendererEventKind::ALL {
        let template = match kind {
            crate::notifications::RendererEventKind::RenderStarted => &mut templates.render_started,
            crate::notifications::RendererEventKind::RenderProgress => {
                &mut templates.render_progress
            }
            crate::notifications::RendererEventKind::RenderCompleted => {
                &mut templates.render_completed
            }
            crate::notifications::RendererEventKind::RenderFailed => &mut templates.render_failed,
            crate::notifications::RendererEventKind::JobQueued => &mut templates.job_queued,
            crate::notifications::RendererEventKind::JobStarted => &mut templates.job_started,
            crate::notifications::RendererEventKind::JobCompiling => &mut templates.job_compiling,
            crate::notifications::RendererEventKind::JobRendering => &mut templates.job_rendering,
            crate::notifications::RendererEventKind::JobCompleted => &mut templates.job_completed,
            crate::notifications::RendererEventKind::JobFailed => &mut templates.job_failed,
            crate::notifications::RendererEventKind::JobCancelled => &mut templates.job_cancelled,
        };
        ui.collapsing(kind.label(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Title");
                ui.text_edit_singleline(&mut template.title);
            });
            ui.label("Body");
            ui.add(egui::TextEdit::multiline(&mut template.body).desired_rows(2));
            if discord {
                ui.label("Discord content");
                ui.add(egui::TextEdit::multiline(&mut template.content).desired_rows(2));
                ui.horizontal(|ui| {
                    ui.label("Embed title");
                    ui.text_edit_singleline(&mut template.embed_title);
                });
                ui.label("Embed description");
                ui.add(egui::TextEdit::multiline(&mut template.embed_description).desired_rows(2));
                ui.horizontal(|ui| {
                    ui.label("Embed footer");
                    ui.text_edit_singleline(&mut template.embed_footer);
                });
                let mut remove_field = None;
                for (index, field) in template.embed_fields.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Embed field {} name", index + 1));
                        ui.text_edit_singleline(&mut field.name);
                        ui.checkbox(&mut field.inline, "Inline");
                        if ui.small_button("Remove").clicked() {
                            remove_field = Some(index);
                        }
                    });
                    ui.add(egui::TextEdit::multiline(&mut field.value).desired_rows(2));
                }
                if let Some(index) = remove_field {
                    template.embed_fields.remove(index);
                }
                if ui.small_button("Add embed field").clicked() {
                    template
                        .embed_fields
                        .push(crate::notifications::DiscordEmbedFieldTemplate::default());
                }
            }
        });
    }
    ui.label("Template values: {renderer}, {renderer_version}, {engine}, {chart}, {output_path}, {execution_mode}, {resolution}, {width}, {height}, {FPS}, {duration}, {render_time}, {frame_count}, {progress}, {error}, {job_id}, {job_state}, {attempt}, {execution_backend}, {timestamp}. Missing values become empty text. Use {{ and }} for literal braces.");
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

fn open_folder(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    let _ = result;
}

fn open_file(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    let _ = result;
}

fn replace_history_input(config: &mut RenderConfig, role: &str, path: PathBuf) -> Result<()> {
    match role {
        "engine" => config.engine = path,
        "resources" => config.resources = path,
        "level" => config.level = path,
        "music" => config.music = Some(path),
        "MV" => config.mv = Some(path),
        "particle resources" => config.resource_overrides.particles = Some(path),
        "effects resources" => config.resource_overrides.effects = Some(path),
        "background resources" => config.resource_overrides.background = Some(path),
        _ => anyhow::bail!("unknown historical input role {role:?}"),
    }
    Ok(())
}

impl Desktop {
    fn app_text(&self, key: &str) -> String {
        crate::localization::tr(&self.workspace_preferences.locale, key)
    }

    fn language_selector_ui(&mut self, ui: &mut egui::Ui) {
        let current = self.workspace_preferences.locale.clone();
        let locale = crate::localization::locale_info(&current);
        let selected_name = locale
            .map(|locale| locale.native_name.as_str())
            .unwrap_or("English");
        let options: Vec<(String, String)> =
            crate::localization::selectable_locales(cfg!(debug_assertions))
                .map(|locale| (locale.id.clone(), locale.native_name.clone()))
                .collect();
        let mut selected = current.clone();
        ui.horizontal(|ui| {
            ui.label(crate::localization::tr(&current, "language.title"));
            egui::ComboBox::from_id_salt("application-locale-selector")
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (id, native_name) in &options {
                        ui.selectable_value(&mut selected, id.clone(), native_name);
                    }
                });
        });
        if selected != current {
            self.workspace_preferences.locale = selected;
            self.save_workspace_preferences();
        }
        if self.workspace_preferences.locale != "en" {
            ui.small(crate::localization::tr(
                &self.workspace_preferences.locale,
                "language.review_notice",
            ));
        }
    }

    fn workflow_section_navigation(&mut self, ui: &mut egui::Ui) {
        use crate::workspace::WorkflowId as Workflow;
        let workflow = self.workspace_preferences.workflow;
        if workflow == Workflow::Settings {
            self.language_selector_ui(ui);
        }
        let pages: &[(usize, &str)] = match workflow {
            Workflow::Production => &[(6, "Queue"), (7, "Render History")],
            Workflow::Tools => &[
                (8, "Cache"),
                (9, "Performance"),
                (10, "Regression"),
                (11, "Compatibility"),
                (13, "Program Preview"),
            ],
            Workflow::Settings => &[
                (0, "Configuration & Profiles"),
                (1, "Engine Options"),
                (2, "Render / Video"),
                (3, "Gameplay UI"),
                (4, "Watermark"),
                (5, "Notifications"),
            ],
            _ => &[],
        };
        if !pages.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (page, label) in pages {
                    if ui.selectable_label(self.tab == *page, *label).clicked() {
                        if let Some(command) = command_for_workflow_page(workflow, *page) {
                            self.dispatch_command(command, ui.ctx());
                        }
                    }
                }
            });
        }
        if workflow == Workflow::Settings {
            ui.horizontal(|ui| {
                ui.label("Search settings");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_search)
                        .hint_text("engine, fps, profile, watermark, notifications")
                        .desired_width(260.0),
                );
                if !self.settings_search.is_empty() && ui.small_button("Clear").clicked() {
                    self.settings_search.clear();
                }
            });
            let query = self.settings_search.trim().to_ascii_lowercase();
            if !query.is_empty() {
                let results = settings_search_results(&query);
                ui.horizontal_wrapped(|ui| {
                    if results.is_empty() {
                        ui.weak("No matching settings section.");
                    } else {
                        for (title, tab) in results {
                            if ui.selectable_label(self.tab == tab, title).clicked() {
                                if let Some(command) = command_for_workflow_page(workflow, tab) {
                                    self.dispatch_command(command, ui.ctx());
                                }
                            }
                        }
                    }
                });
            }
        }
    }

    fn shared_status_ui(&mut self, ui: &mut egui::Ui) {
        if self.error && !self.summary.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.summary);
        } else if self.job.is_some()
            || (self.summary != "Select your project files to begin." && !self.summary.is_empty())
        {
            ui.weak(&self.summary);
        }
        if self.job.is_some() {
            ui.horizontal(|ui| {
                if ui.button("Cancel Render").clicked() {
                    if let Some(job) = &self.job {
                        job.control.cancel();
                    }
                    self.summary = "Cancellation requested…".into();
                }
                ui.label("Render is running. You can continue using the other workflows.");
            });
        }
        self.gcc_status_ui_if_visible(ui);
        let live = self.live.lock().unwrap();
        if let Some(progress) = &live.progress {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "{:?} · {} / {} frames · {:.1}s",
                    progress.phase,
                    progress.completed_frames,
                    progress
                        .total_frames
                        .map_or_else(|| "?".into(), |value| value.to_string()),
                    progress.elapsed_seconds
                ));
                if let Some(fps) = progress.frames_per_second {
                    ui.label(format!("{fps:.1} fps"));
                }
                if let Some(eta) = progress.eta_seconds {
                    ui.label(format!("ETA {eta:.1}s"));
                }
            });
            if let Some(percent) = progress.percentage {
                ui.add(egui::ProgressBar::new((percent / 100.0) as f32).show_percentage());
            }
        }
        if self.workspace_preferences.workflow == crate::workspace::WorkflowId::Diagnostics {
            ui.collapsing("Renderer log", |ui| {
                if ui.button("Copy log").clicked() {
                    ui.ctx()
                        .copy_text(live.logs.iter().cloned().collect::<Vec<_>>().join("\n"));
                }
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &live.logs {
                            ui.label(line);
                        }
                    });
            });
        }
    }

    fn gcc_status_ui_if_visible(&mut self, ui: &mut egui::Ui) {
        if self.workspace_preferences.workflow == crate::workspace::WorkflowId::Diagnostics {
            self.gcc_status_ui(ui);
        }
    }
}

impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Recovery is application-scoped: changing tabs or showing Welcome
        // must not suspend a pending composition checkpoint.
        self.presentation_autosave(ctx);
        self.collect_notification_diagnostics();
        let was_capturing_shortcut = self.shortcut_capture.is_some();
        self.poll_shortcut_capture(ctx);
        if !was_capturing_shortcut
            && self.shortcut_capture.is_none()
            && self.shortcut_conflict.is_none()
            && !self.command_palette_open
            && !self.shortcut_editor_open
            && !self.help_open
            && !self.presentation_tour_open
            && !ctx.egui_wants_keyboard_input()
            && !self.chart_close_prompt
            && !self.confirm_profile_delete
            && !self.confirm_profile_reset
        {
            let shortcut_command = ctx.input(|input| {
                input.raw.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => self.command_for_shortcut_event(*key, *modifiers),
                    _ => None,
                })
            });
            if let Some(command) = shortcut_command {
                self.dispatch_command(command, ctx);
            }
        }
        self.poll();
        self.poll_media_acquisition();
        self.poll_program_preview();
        self.poll_chart_jobs();
        self.poll_presentation_job(ctx);
        self.poll_quick_validation();
        self.record_quick_render_success();
        let capture_failed = self
            .presentation_studio
            .audio_capture
            .as_ref()
            .is_some_and(|capture| capture.level().error.is_some());
        if capture_failed {
            self.presentation_stop_audio();
        } else if let Some(active_id) = self
            .presentation_studio
            .audio_capture
            .as_ref()
            .filter(|_| {
                self.presentation_studio.audio_device_check.elapsed() >= Duration::from_secs(2)
            })
            .map(|capture| capture.device_id().to_owned())
        {
            self.presentation_refresh_audio_devices();
            if !self
                .presentation_studio
                .audio_devices
                .iter()
                .any(|device| device.id == active_id)
            {
                self.presentation_stop_audio();
                self.presentation_studio.error = Some(
                    "The microphone disconnected during capture. Sono-Renderer finalized the audio received before disconnection; inspect the take boundary before use.".into(),
                );
            }
        }
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if dropped.is_empty() {
            self.last_dropped_files = None;
        } else if self.last_dropped_files.as_ref() != Some(&dropped) {
            self.last_dropped_files = Some(dropped.clone());
            self.process_dropped_files(dropped);
        }
        let queue_active = self.job_manager.as_ref().is_some_and(|manager| {
            manager
                .jobs()
                .is_ok_and(|jobs| jobs.iter().any(|job| job.state.is_active()))
        });
        if ctx.input(|input| input.viewport().close_requested())
            && self.presentation_studio.audio_capture.is_some()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.presentation_stop_audio();
            self.summary = "Voiceover capture stopped and finalized. Review/save the Presentation Studio project, then close again.".into();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && (self.job.is_some()
                || self.performance_job.is_some()
                || self.regression_job.is_some()
                || self.manifest_job.is_some()
                || self.media_acquisition_job.is_some()
                || self.preview_job.is_some()
                || self.chart_studio.preview_job.is_some()
                || self.chart_studio.tempo_job.is_some()
                || self.chart_studio.audio_playback_start.is_some()
                || self.chart_studio.engine_job.is_some()
                || self.presentation_studio.job.is_some()
                || self.presentation_studio.audio_capture.is_some()
                || self.quick_validation_job.is_some()
                || queue_active)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_pending = true;
            if let Some(job) = self.job.as_ref() {
                job.control.cancel();
            }
            if let Some(job) = &self.media_acquisition_job {
                job.control.cancel();
            }
            if let Some(job) = &self.chart_studio.tempo_job {
                job.control.cancel();
            }
            self.chart_studio.audio_playback = None;
            if let Some(job) = &self.presentation_studio.job {
                job.control.cancel();
            }
            if let Some(manager) = &self.job_manager {
                if let Ok(jobs) = manager.jobs() {
                    for job in jobs.iter().filter(|job| job.state.is_active()) {
                        let _ = manager.cancel(&job.id);
                    }
                }
            }
            self.summary = "Cancelling before closing...".into();
        }
        if ctx.input(|input| input.viewport().close_requested())
            && self.chart_studio.document.is_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.chart_close_prompt = true;
        }
        if should_prompt_presentation_close(
            ctx.input(|input| input.viewport().close_requested()),
            self.close_pending,
            self.chart_studio.document.is_dirty(),
            self.presentation_studio.dirty,
            self.presentation_close_checkpointed,
        ) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.presentation_close_prompt = true;
        }
        let queue_still_active = self.job_manager.as_ref().is_some_and(|manager| {
            manager
                .jobs()
                .is_ok_and(|jobs| jobs.iter().any(|job| job.state.is_active()))
        });
        if self.close_pending
            && self.job.is_none()
            && self.performance_job.is_none()
            && self.regression_job.is_none()
            && self.manifest_job.is_none()
            && self.media_acquisition_job.is_none()
            && self.preview_job.is_none()
            && self.chart_studio.preview_job.is_none()
            && self.chart_studio.tempo_job.is_none()
            && self.chart_studio.audio_playback_start.is_none()
            && self.chart_studio.engine_job.is_none()
            && self.presentation_studio.job.is_none()
            && self.quick_validation_job.is_none()
            && !queue_still_active
        {
            if self.chart_studio.document.is_dirty() {
                self.close_pending = false;
                self.chart_close_prompt = true;
            } else if self.presentation_studio.dirty && !self.presentation_close_checkpointed {
                self.close_pending = false;
                self.presentation_close_prompt = true;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self.job.is_some()
            || self.engine_job.is_some()
            || self.performance_job.is_some()
            || self.regression_job.is_some()
            || self.manifest_job.is_some()
            || self.media_acquisition_job.is_some()
            || self.preview_job.is_some()
            || self.chart_studio.preview_job.is_some()
            || self.chart_studio.tempo_job.is_some()
            || self.chart_studio.audio_playback_start.is_some()
            || self.chart_studio.engine_job.is_some()
            || self.presentation_studio.job.is_some()
            || self.presentation_studio.audio_capture.is_some()
            || self.quick_validation_job.is_some()
            || queue_active
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.admin_server.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use crate::workspace::WorkflowId as Workflow;
        let font_locale = self.workspace_preferences.locale.clone();
        if self.ui_font_locale != font_locale {
            let _ = crate::localization::install_system_ui_font_fallbacks(root.ctx(), &font_locale);
            self.ui_font_locale = font_locale;
        }
        self.collect_notification_diagnostics();
        if self.admin_server.is_some() && self.last_profile_sync.elapsed() >= Duration::from_secs(1)
        {
            self.last_profile_sync = Instant::now();
            if let Some(manager) = self.profile_manager.as_ref() {
                if let Ok(active) = manager.active() {
                    let active_id = active.as_ref().map(|profile| profile.id.as_str());
                    if active_id != self.active_profile_id.as_deref() {
                        if let Some(profile) = active {
                            let name = profile.name.clone();
                            self.adopt_config_profile(
                                profile,
                                format!("Configuration profile {name} selected externally."),
                            );
                        } else {
                            self.active_profile_id = None;
                            self.profile_name_input.clear();
                        }
                    }
                }
            }
        }
        self.poll_chart_tempo_analysis(root.ctx());
        self.poll_chart_audio_playback(root.ctx());
        self.poll_presentation_playback(root.ctx());

        if self.welcome_visible || self.workspace_preferences.workflow == Workflow::Home {
            egui::CentralPanel::default().show(root, |ui| {
                self.global_toolbar(ui);
                ui.separator();
                self.getting_started_ui(ui);
            });
        } else if self.workspace_preferences.workflow == Workflow::QuickRender {
            egui::CentralPanel::default().show(root, |ui| {
                self.global_toolbar(ui);
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.quick_render_ui(ui);
                    });
                self.shared_status_ui(ui);
            });
        } else {
            egui::CentralPanel::default().show(root, |ui| {
                let workflow = self.workspace_preferences.workflow;
                self.global_toolbar(ui);
                self.workflow_section_navigation(ui);
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    match workflow {
                        Workflow::Home => self.getting_started_ui(ui),
                        Workflow::QuickRender => self.quick_render_ui(ui),
                        Workflow::ChartStudio => {
                            ui.heading(self.app_text("chart.title"));
                            ui.label(self.app_text("chart.description"));
                            self.chart_studio_ui(ui);
                        }
                        Workflow::PresentationStudio => {
                            ui.horizontal(|ui| {
                                ui.heading(self.app_text("presentation.title"));
                                ui.weak("Media → timeline → program preview → inspector → export");
                                if self.presentation_studio.dirty {
                                    ui.colored_label(egui::Color32::YELLOW, self.app_text("common.modified"));
                                } else {
                                    ui.weak(self.app_text("common.saved"));
                                }
                            });
                            self.presentation_studio_ui(ui);
                        }
                        Workflow::Production => match self.tab {
                            7 => self.render_history_ui(ui),
                            _ => self.job_manager_ui(ui),
                        },
                        Workflow::Tools => match self.tab {
                            9 => self.performance_lab_ui(ui),
                            10 => self.regression_lab_ui(ui),
                            11 => self.compatibility_ui(ui),
                            13 => self.program_preview_ui(ui),
                            _ => self.cache_manager_ui(ui),
                        },
                        Workflow::Settings => match self.tab {
                            0 => {
                                ui.heading("Configuration & reusable profiles");
                                ui.label("Input files and render projects belong to Quick Render. This page manages saved configuration files and reusable render profiles.");
                                self.config_file_controls(ui);
                                self.config_profile_controls(ui);
                            }
                            1 => self.options(ui),
                            2 => self.settings(ui),
                            3 => self.gameplay_ui_settings(ui),
                            4 => self.watermark_settings(ui),
                            5 => self.notifications_settings(ui),
                            _ => self.settings(ui),
                        },
                        Workflow::Diagnostics => {
                            ui.heading("Renderer Diagnostics");
                            ui.label("Detailed runtime, backend, compatibility, and log information.");
                            self.render_diagnostics_ui(ui);
                        }
                    }
                });
                self.shared_status_ui(ui);
            });
        }
        self.command_palette_ui(root.ctx());
        self.shortcut_editor_ui(root.ctx());
        self.help_window(root.ctx());
        self.presentation_tour_window(root.ctx());
        self.chart_close_prompt_ui(root.ctx());
        self.presentation_close_prompt_ui(root.ctx());
    }
}

impl Desktop {
    fn record_quick_render_success(&mut self) {
        if self
            .onboarding_preferences
            .first_successful_job_id
            .is_some()
        {
            return;
        }
        let Some(id) = self.quick_job_id.clone() else {
            return;
        };
        let job = self
            .job_manager
            .as_ref()
            .and_then(|manager| manager.inspect(&id).ok());
        if let Some(job) = job.filter(|job| job.state == crate::job_manager::JobState::Completed) {
            self.onboarding_preferences.first_successful_job_id = Some(job.id.clone());
            self.onboarding_preferences.first_successful_output =
                job.output_path().map(PathBuf::from);
            self.save_onboarding_preferences();
        }
    }

    fn save_onboarding_preferences(&mut self) {
        if self.onboarding_write_blocked {
            return;
        }
        match self.onboarding_preferences.save(&self.onboarding_path) {
            Ok(()) => self.onboarding_error = None,
            Err(error) => self.onboarding_error = Some(format!("{error:#}")),
        }
    }

    fn finish_welcome(&mut self) {
        self.onboarding_preferences.complete_welcome();
        self.welcome_visible = false;
        self.save_onboarding_preferences();
    }

    fn skip_welcome(&mut self) {
        self.onboarding_preferences.skip_welcome();
        self.welcome_visible = false;
        self.save_onboarding_preferences();
        self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
    }

    fn reset_onboarding(&mut self) {
        self.onboarding_preferences.reset_welcome();
        self.onboarding_preferences
            .gameplay_presentation_tour_completed = false;
        self.welcome_visible = true;
        self.quick_render_active = false;
        self.workspace_preferences.workflow = crate::workspace::WorkflowId::Home;
        self.save_workspace_preferences();
        self.presentation_tour_open = false;
        self.onboarding_write_blocked = false;
        self.save_onboarding_preferences();
    }

    fn start_quick_validation(&mut self, ctx: &egui::Context) {
        if self.quick_validation_job.is_some() {
            return;
        }
        let config = self.config.clone();
        let worker_config = config.clone();
        let wake = ctx.clone();
        let (sender, result) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("sono-quick-render-preflight".into())
            .spawn(move || {
                let outcome = (|| -> Result<QuickValidationReport> {
                    worker_config
                        .validate_export_inputs()
                        .context("Some required inputs or output settings need attention")?;
                    let output = worker_config
                        .output
                        .as_deref()
                        .context("Choose an output video path")?;
                    if output.exists() {
                        bail!("The output file already exists: {}", output.display());
                    }
                    let parent = output
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or(std::path::Path::new("."));
                    if parent.exists() && !parent.is_dir() {
                        bail!("The output folder is not a directory: {}", parent.display());
                    }
                    let ffmpeg_status = match crate::ffmpeg::inspect_existing() {
                        Ok(Some(installation)) => format!(
                            "FFmpeg is ready ({:?}): {}",
                            installation.source,
                            installation.ffmpeg.display()
                        ),
                        Ok(None) => {
                            #[cfg(windows)]
                            {
                                "No validated FFmpeg installation was found. Windows export will provision the pinned FFmpeg pair when needed; first-time setup requires network access.".into()
                            }
                            #[cfg(not(windows))]
                            {
                                "No validated FFmpeg installation was found. MP4 export needs a usable FFmpeg and FFprobe pair; automatic provisioning is only available on Windows.".into()
                            }
                        }
                        Err(error) => format!(
                            "Could not inspect FFmpeg yet: {error:#}. Export will report a detailed error if the tools cannot be resolved."
                        ),
                    };
                    let report = crate::compatibility::analyze(
                        &worker_config.engine,
                        &worker_config.resources,
                        &worker_config.level,
                    )
                    .context("Reading the engine, level, and resource package")?;
                    Ok(QuickValidationReport {
                        summary: "Required files are readable. Compatibility analysis is complete; review any findings below before rendering.".into(),
                        details: crate::compatibility::render_human_readable(&report),
                        ffmpeg_status,
                    })
                })()
                .map_err(|error| format!("{error:#}"));
                let _ = sender.send(outcome);
                wake.request_repaint();
            });
        match worker {
            Ok(worker) => {
                self.quick_validation_result = None;
                self.quick_validation_config = None;
                self.quick_validation_job = Some(QuickValidationJob {
                    config,
                    result,
                    worker,
                });
            }
            Err(error) => {
                self.quick_validation_result =
                    Some(Err(format!("Could not start input check: {error}")));
            }
        }
    }

    fn poll_quick_validation(&mut self) {
        let completion =
            self.quick_validation_job
                .as_ref()
                .and_then(|job| match job.result.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("input check stopped before returning a result".into()))
                    }
                });
        let Some(completion) = completion else {
            return;
        };
        let Some(job) = self.quick_validation_job.take() else {
            return;
        };
        let _ = job.worker.join();
        if job.config == self.config {
            if completion.is_ok() {
                self.quick_validation_config = Some(job.config);
            }
            self.quick_validation_result = Some(completion);
        } else {
            self.quick_validation_config = None;
            self.quick_validation_result = Some(Err(
                "Inputs changed while they were being checked. Run Check inputs again.".into(),
            ));
        }
    }

    fn enqueue_quick_render(&mut self) {
        if self.quick_validation_config.as_ref() != Some(&self.config) {
            self.quick_validation_result = Some(Err(
                "Check the current inputs before adding this render to the queue.".into(),
            ));
            return;
        }
        let result = (|| -> Result<String> {
            let manager = self
                .job_manager
                .as_ref()
                .context("The render queue is unavailable")?;
            let profile = self
                .active_profile_id
                .as_deref()
                .and_then(|id| self.profile_manager.as_ref()?.get(id).ok());
            manager.enqueue_with_metadata(
                self.config.clone(),
                None,
                profile.map(|profile| (profile.id, profile.name)),
                crate::job_manager::RenderJobMetadata {
                    project_path: self.project_path.clone(),
                    ..Default::default()
                },
            )
        })();
        match result {
            Ok(id) => {
                self.quick_job_id = Some(id.clone());
                self.selected_job = Some(id.clone());
                self.finish_welcome();
                self.summary = format!("Render queued as job {id}.");
                self.error = false;
            }
            Err(error) => {
                self.quick_validation_result = Some(Err(format!("{error:#}")));
                self.summary = "The render was not queued. Review the input guidance.".into();
                self.error = true;
            }
        }
    }

    fn getting_started_ui(&mut self, ui: &mut egui::Ui) {
        let mut open_recent = None;
        let first_run = self.onboarding_preferences.should_show_welcome();
        self.language_selector_ui(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.heading(crate::localization::tr(
                        &self.workspace_preferences.locale,
                        if first_run { "home.title.welcome" } else { "home.title.home" },
                    ));
                    ui.add_space(5.0);
                    ui.heading(self.app_text("home.question"));
                    ui.add_space(5.0);
                    ui.label(self.app_text("home.intro"));
                });
                ui.add_space(18.0);
                ui.columns(2, |columns| {
                    columns[0].group(|ui| {
                        ui.heading(self.app_text("home.render.title"));
                        ui.label(self.app_text("home.render.body"));
                        if ui.button(self.app_text("home.render.action")).clicked() {
                            self.dispatch_command(crate::workspace::CommandId::GoQuickRender, ui.ctx());
                        }
                    });
                    columns[1].group(|ui| {
                        ui.heading(self.app_text("home.chart.title"));
                        ui.label(self.app_text("home.chart.body"));
                        if ui.button(self.app_text("home.chart.action")).clicked() {
                            self.dispatch_command(crate::workspace::CommandId::GoChartStudio, ui.ctx());
                        }
                    });
                });
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    columns[0].group(|ui| {
                        ui.heading(self.app_text("home.video.title"));
                        ui.label(self.app_text("home.video.body"));
                        if ui.button(self.app_text("home.video.action")).clicked() {
                            self.dispatch_command(crate::workspace::CommandId::GoPresentationStudio, ui.ctx());
                        }
                    });
                    columns[1].group(|ui| {
                        ui.heading(self.app_text("home.project.title"));
                        ui.label(self.app_text("home.project.body"));
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(self.app_text("home.project.render")).clicked() {
                                self.dispatch_command(crate::workspace::CommandId::OpenRenderProject, ui.ctx());
                            }
                            if ui.button(self.app_text("home.project.video")).clicked() {
                                self.dispatch_command(crate::workspace::CommandId::GoPresentationStudio, ui.ctx());
                                self.dispatch_command(crate::workspace::CommandId::OpenComposition, ui.ctx());
                            }
                        });
                        ui.separator();
                        ui.strong("Recent render projects");
                        let recent_paths = self.project_recents.paths.iter().take(5).cloned().collect::<Vec<_>>();
                        if recent_paths.is_empty() {
                            ui.weak("No recent render projects yet. Video compositions can be opened from Presentation Studio.");
                        } else {
                            for path in recent_paths {
                                let label = path.file_name().and_then(|name| name.to_str()).unwrap_or("Render project");
                                if ui.small_button(format!("{label}  ·  {}", path.display())).clicked() {
                                    open_recent = Some(path);
                                }
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(self.app_text("home.learn.title"));
                        ui.label(self.app_text("home.learn.body"));
                        if ui.button(self.app_text("home.learn.action")).clicked() {
                            self.help_open = true;
                        }
                        if ui.button("Render workflow tour").clicked() {
                            self.presentation_tour_open = true;
                            self.presentation_tour_step = 0;
                            self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                        }
                    });
                    ui.small("Learn also covers vertical Shorts, importing local media, From URL acquisition, recording voiceover, saving projects, and exporting.");
                });
                ui.add_space(12.0);
                if first_run {
                    ui.horizontal(|ui| {
                        if ui.button("Skip for now").clicked() {
                            self.skip_welcome();
                        }
                        ui.weak("You can reopen this screen any time from Help → Home / Getting Started.");
                    });
                } else {
                    ui.weak("Choose a workflow. You can return here any time from the top navigation.");
                }
                if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
                    ui.separator();
                    ui.label("You can also drop supported render inputs, a project, or local media into the window. Ambiguous render-input matches are shown for you to choose.");
                }
                if let Some(error) = &self.onboarding_error {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Onboarding preferences could not be loaded or saved, so these choices may not persist. Use Help → Replay Getting Started to replace the local preference file. Details: {error}"),
                    );
                }
            });
        if let Some(path) = open_recent {
            self.finish_welcome();
            self.tab = 0;
            self.open_project(path);
        }
    }

    fn quick_render_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.app_text("quick.title"));
            if ui.button("Renderer settings…").clicked() {
                self.select_workflow_page(crate::workspace::WorkflowId::Settings, 0);
            }
            if ui.button("Help").clicked() {
                self.help_open = true;
            }
        });
        ui.label(self.app_text("quick.description"));
        ui.label("Engine, level, and resources are required. Music is required while Include music is on. MV and separate particle/effect/background packages are optional.");
        if ui.button("Open an existing project...").clicked() {
            self.pick_open_project();
        }
        ui.collapsing("Project and reusable profiles", |ui| {
            self.project_controls(ui);
            self.project_workflow_ui(ui);
            ui.label("Profiles and detailed configuration are available in Settings.");
        });
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            ui.colored_label(
                egui::Color32::LIGHT_BLUE,
                "Release to classify the dropped files. Ambiguous matches require your choice.",
            );
        }
        self.inputs(ui);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.layers.bgm, "Include music").on_hover_text(
                "Include the selected Music / BGM file in the exported video. Turn this off only when you want gameplay without the chart music.",
            );
            ui.label(format!(
                "Current output: {}x{} at {} FPS; {} seconds",
                self.config.width,
                self.config.height,
                self.config.fps,
                if self.config.whole_chart { "whole chart".into() } else { format!("{:.2}", self.config.duration) },
            ));
        });
        ui.collapsing("Render summary", |ui| {
            egui::Grid::new("quick-render-summary")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, path) in [
                        ("Engine", Some(&self.config.engine)),
                        ("Level", Some(&self.config.level)),
                        ("Resources", Some(&self.config.resources)),
                        (
                            "Music",
                            self.config
                                .music
                                .as_ref()
                                .filter(|_| self.config.layers.bgm),
                        ),
                        ("Output", self.config.output.as_ref()),
                    ] {
                        ui.label(label);
                        ui.label(path.map_or_else(
                            || "Not selected".into(),
                            |path| path.display().to_string(),
                        ));
                        ui.end_row();
                    }
                    ui.label("Resolution / FPS");
                    ui.label(format!(
                        "{}x{} / {} FPS",
                        self.config.width, self.config.height, self.config.fps
                    ));
                    ui.end_row();
                });
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.quick_validation_job.is_none(),
                    egui::Button::new("Check inputs"),
                )
                .clicked()
            {
                self.start_quick_validation(ui.ctx());
            }
            let validation_current = self.quick_validation_config.as_ref() == Some(&self.config);
            if ui
                .add_enabled(
                    validation_current && self.quick_validation_job.is_none(),
                    egui::Button::new("Queue render"),
                )
                .clicked()
            {
                self.enqueue_quick_render();
            }
            if self.quick_validation_job.is_some() {
                ui.spinner();
                ui.label("Checking required files and engine/level compatibility...");
            }
        });
        if let Some(result) = &self.quick_validation_result {
            match result {
                Ok(report) if self.quick_validation_config.as_ref() == Some(&self.config) => {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, &report.summary);
                    ui.label(&report.ffmpeg_status);
                    egui::CollapsingHeader::new("Compatibility findings")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.label(&report.details);
                        });
                }
                Ok(_) => {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Settings changed after checking. Check inputs again before rendering.",
                    );
                }
                Err(error) => {
                    ui.colored_label(egui::Color32::LIGHT_RED, friendly_beginner_error(error));
                    ui.collapsing("Technical details", |ui| {
                        ui.label(error);
                    });
                }
            }
        }
        self.quick_job_status_ui(ui);
    }

    fn quick_job_status_ui(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.quick_job_id.clone() else {
            return;
        };
        let inspected = self
            .job_manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("render queue is unavailable"))
            .and_then(|manager| manager.inspect(&id));
        match inspected {
            Ok(job) => {
                ui.separator();
                match job.state {
                    crate::job_manager::JobState::Completed => {
                        ui.heading("Render complete");
                        let output = job.output_path().map(PathBuf::from);
                        if self
                            .onboarding_preferences
                            .first_successful_job_id
                            .is_none()
                        {
                            self.onboarding_preferences.first_successful_job_id =
                                Some(job.id.clone());
                            self.onboarding_preferences.first_successful_output = output.clone();
                            self.save_onboarding_preferences();
                        }
                        if let Some(path) = &output {
                            ui.label(format!("Output video: {}", path.display()));
                            ui.horizontal(|ui| {
                                if ui.button("Open Video").clicked() {
                                    open_file(path);
                                }
                                if ui.button("Open Folder").clicked() {
                                    if let Some(parent) = path.parent() {
                                        open_folder(parent);
                                    }
                                }
                                if ui.button("Render Again (choose a new output)").clicked() {
                                    self.config.output = None;
                                    self.quick_validation_config = None;
                                    self.quick_validation_result = None;
                                }
                            });
                        }
                        if let Some(outcome) = &job.outcome {
                            ui.label(format!(
                                "{} frames (export range: {:.2}s)",
                                outcome.submitted_frames,
                                outcome.timeline_end - outcome.timeline_start
                            ));
                        }
                    }
                    crate::job_manager::JobState::Failed
                    | crate::job_manager::JobState::Cancelled => {
                        ui.heading(if job.state == crate::job_manager::JobState::Cancelled {
                            "Render cancelled"
                        } else {
                            "Render could not be completed"
                        });
                        let guidance = if let Some(diagnostic) = &job.diagnostic {
                            friendly_beginner_error(diagnostic)
                        } else {
                            match job.failure_kind {
                                Some(crate::job_manager::JobFailureKind::ExternalResource) => "A media or resource file could not be used. Check that the selected files still exist and match their roles.".into(),
                                Some(crate::job_manager::JobFailureKind::Configuration) => "One of the render settings or output choices needs attention.".into(),
                                Some(crate::job_manager::JobFailureKind::Interrupted) => "The application stopped before this render completed. You can check its saved diagnostics or queue it again.".into(),
                                _ => "Review the technical details or open Diagnostics for the full report.".into(),
                            }
                        };
                        ui.label(guidance);
                        if let Some(diagnostic) = &job.diagnostic {
                            ui.collapsing("Technical details", |ui| {
                                ui.label(diagnostic);
                            });
                        }
                        if ui.button("Open queue and diagnostics").clicked() {
                            self.select_workflow_page(crate::workspace::WorkflowId::Production, 6);
                        }
                    }
                    state => {
                        ui.heading("Render in progress");
                        if state == crate::job_manager::JobState::Queued {
                            ui.label("Waiting in the render queue.");
                        }
                        if let Some(progress) = &job.progress {
                            let stage = match progress.phase {
                                crate::export_progress::ExportPhase::Preparing => "Preparing",
                                crate::export_progress::ExportPhase::Validating => "Checking files",
                                crate::export_progress::ExportPhase::Compiling => {
                                    "Preparing optional acceleration"
                                }
                                crate::export_progress::ExportPhase::AudioMixing
                                | crate::export_progress::ExportPhase::AudioEncoding => {
                                    "Preparing audio"
                                }
                                crate::export_progress::ExportPhase::Rendering => {
                                    "Rendering gameplay"
                                }
                                crate::export_progress::ExportPhase::Finalizing => {
                                    "Finishing video"
                                }
                                crate::export_progress::ExportPhase::Complete => "Complete",
                            };
                            ui.label(format!(
                                "{stage} · {} / {} frames",
                                progress.completed_frames,
                                progress
                                    .total_frames
                                    .map_or_else(|| "?".into(), |count| count.to_string())
                            ));
                            if let Some(percent) = progress.percentage {
                                ui.add(
                                    egui::ProgressBar::new((percent / 100.0) as f32)
                                        .show_percentage(),
                                );
                            }
                        }
                    }
                }
            }
            Err(error) => {
                if let Some(path) = &self.onboarding_preferences.first_successful_output {
                    ui.separator();
                    ui.heading("Your first render");
                    ui.label(format!("Saved output: {}", path.display()));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(path.is_file(), egui::Button::new("Open Video"))
                            .clicked()
                        {
                            open_file(path);
                        }
                        if ui.button("Open Folder").clicked() {
                            if let Some(parent) = path.parent() {
                                open_folder(parent);
                            }
                        }
                    });
                } else {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("The saved render job is unavailable: {error}"),
                    );
                }
            }
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.help_open {
            return;
        }
        let topics = [
            ("Render a Level", "In Quick Render, choose an engine ZIP, a level, and its resources. Music is required only while Include music is enabled. Choose an MP4 output, check inputs, then queue the render. Projects and profiles are optional."),
            ("Create or edit a chart", "Chart Studio opens or creates Sonolus LevelData, supports generic entity editing, can inspect archetypes from a selected engine, previews through the production Watch renderer, validates document structure, and exports gzip LevelData. Engine-specific authoring labels require a matching schema."),
            ("Edit a Video", "Presentation Studio edits local video, audio, still images, text, effects, and multiple timeline tracks. It is a standalone composition workflow: no Sonolus engine, LevelData, Chart Studio, Watch, or Sono-GCC is required."),
            ("Make a vertical Short", "In Presentation Studio, start a composition, set its canvas to 1080×1920, place landscape or portrait media, then use transforms/crop, text, effects, audio, and the normal MP4 export. This is the same editor and project format."),
            ("Import local media", "Presentation Studio can import local video, audio, and still images into its media bin. Dragging media onto a timeline track creates ordinary clips. Local import works without URL tools or Sonolus content."),
            ("Get media from a URL", "Project / inputs and Chart Studio provide the optional From URL acquisition workflow. Inspect and explicitly download one supported HTTP(S) item; the result becomes a local media file. URL acquisition is not required for local editing or rendering."),
            ("Record voiceover", "In Presentation Studio, choose an input and use the input meter before recording. Record Voiceover starts timeline playback and captures a lossless WAV take at the playhead; the result is an ordinary audio media source and timeline clip. Capture starts only after the user requests it."),
            ("Save and reopen projects", "Render projects save reusable engine/level/resource references and render defaults. Presentation Studio saves standalone .sonocomp compositions with media references. These are separate project types; neither requires a profile."),
            ("Playback and export", "Presentation Studio uses continuous timeline playback for video and mixed audio. Export uses the same composition model and filter graph, supports landscape or vertical dimensions and an optional In/Out range, and validates the completed MP4."),
            ("Navigation and layouts", "Choose a workflow from the top navigation: Home, Render a Level, Chart Studio, Edit Video, or Jobs & History. Tools, Settings, and Diagnostics are under More. In Presentation Studio, Layout changes panel visibility and timeline scale only; it does not switch the project or workflow. Use the Command Palette to search actions and Keyboard Shortcuts to rebind them."),
            ("Jobs and history", "A render job captures its settings when queued. The queue shows active work; Render History keeps completed and failed render attempts."),
            ("Advanced renderer settings", "Sono VM / Interpreter is the default and compatibility reference. Sono-GCC is an optional native backend for eligible regions; unsupported regions continue through the VM, and no speedup is promised."),
            ("Gameplay presentation", "Gameplay UI profiles customize the rendered Sonolus HUD. Program Preview renders still frames through the real renderer. These tools affect gameplay presentation, not standalone Presentation Studio projects."),
            ("Command line", "The direct command form is: renderer render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --start-time 0 --duration 10 --fps 60 --width 1920 --height 1080. Use renderer render-video --help for current options."),
        ];
        let mut open = self.help_open;
        egui::Window::new("Learn and Help")
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Find a topic");
                    ui.text_edit_singleline(&mut self.help_search);
                });
                ui.separator();
                let query = self.help_search.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        for (title, text) in topics {
                            if !query.is_empty()
                                && !title.to_lowercase().contains(&query)
                                && !text.to_lowercase().contains(&query)
                            {
                                continue;
                            }
                            ui.collapsing(title, |ui| {
                                ui.label(text);
                            });
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.button("Render a Level").clicked() {
                        self.dispatch_command(crate::workspace::CommandId::GoQuickRender, ui.ctx());
                        self.help_open = false;
                    }
                    if ui.button("Chart Studio").clicked() {
                        self.dispatch_command(crate::workspace::CommandId::GoChartStudio, ui.ctx());
                        self.help_open = false;
                    }
                    if ui.button("Presentation Studio").clicked() {
                        self.dispatch_command(crate::workspace::CommandId::GoPresentationStudio, ui.ctx());
                        self.help_open = false;
                    }
                    if ui.button("From URL").clicked() {
                        self.dispatch_command(crate::workspace::CommandId::OpenMediaAcquisition, ui.ctx());
                        self.help_open = false;
                    }
                    if ui.button("Replay Getting Started").clicked() {
                        self.reset_onboarding();
                        self.help_open = false;
                    }
                    if ui.button("Render workflow tour").clicked() {
                        self.presentation_tour_step = 0;
                        self.presentation_tour_open = true;
                        self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                        self.help_open = false;
                    }
                });
                ui.separator();
                ui.collapsing("About Sono-Renderer", |ui| {
                    ui.label(format!(
                        "Sono-Renderer {}",
                        crate::build_info::VERSION
                    ));
                    ui.label(format!("Build: {}", crate::build_info::BUILD_ID));
                    ui.label(format!("Target: {}", crate::build_info::TARGET));
                    if crate::build_info::is_dirty() {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "This executable was built from a modified source tree.",
                        );
                    }
                    ui.label("Sono-Renderer is MIT licensed. Third-party notices are included with release packages.");
                    ui.label("MP4 export downloads the pinned FFmpeg distribution on first use when it is not already installed.");
                });
                if let Some(error) = &self.onboarding_error {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Onboarding preferences could not be loaded or saved. Use Replay Getting Started to replace the local preference file. Details: {error}"),
                    );
                }
            });
        self.help_open = open;
    }

    fn presentation_tour_window(&mut self, ctx: &egui::Context) {
        if !self.presentation_tour_open {
            return;
        }
        const STEPS: [(crate::workspace::WorkflowId, usize, &str, &str); 5] = [
            (crate::workspace::WorkflowId::QuickRender, 0, "Quick Render", "Choose the engine ZIP, level, resources, music, and output. Input discovery offers choices when dropped files are ambiguous."),
            (crate::workspace::WorkflowId::Settings, 3, "Gameplay UI settings", "Load a gameplay UI profile and choose custom, legacy, or off mode. This affects rendered gameplay, not the desktop interface."),
            (crate::workspace::WorkflowId::Tools, 13, "Program Preview", "Render selected Watch times through the actual renderer. This is a still-frame monitor, not continuous playback."),
            (crate::workspace::WorkflowId::Settings, 2, "Render settings", "Set range, resolution, FPS, backend, and optional Sono-GCC execution. The default remains the Sono VM interpreter."),
            (crate::workspace::WorkflowId::Production, 6, "Render queue", "Queued jobs capture their configuration, report progress, and associate output, diagnostics, and history."),
        ];
        let step = self.presentation_tour_step.min(STEPS.len() - 1);
        let (workflow, tab, title, description) = STEPS[step];
        if self.workspace_preferences.workflow != workflow || self.tab != tab {
            self.navigate_workflow(workflow);
            self.tab = tab;
        }
        let mut open = self.presentation_tour_open;
        egui::Window::new("Render Workflow Tour")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Step {} of {} · {}", step + 1, STEPS.len(), title));
                ui.label(description);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(step > 0, egui::Button::new("Back"))
                        .clicked()
                    {
                        self.presentation_tour_step -= 1;
                        let (workflow, tab, _, _) = STEPS[self.presentation_tour_step];
                        self.navigate_workflow(workflow);
                        self.tab = tab;
                    }
                    if ui.button("Skip tour").clicked() {
                        self.presentation_tour_open = false;
                        self.onboarding_preferences
                            .gameplay_presentation_tour_completed = true;
                        self.save_onboarding_preferences();
                    }
                    let next_label = if step + 1 == STEPS.len() {
                        "Finish"
                    } else {
                        "Next"
                    };
                    if ui.button(next_label).clicked() {
                        if step + 1 == STEPS.len() {
                            self.presentation_tour_open = false;
                            self.onboarding_preferences
                                .gameplay_presentation_tour_completed = true;
                            self.save_onboarding_preferences();
                        } else {
                            self.presentation_tour_step += 1;
                            let (workflow, tab, _, _) = STEPS[self.presentation_tour_step];
                            self.navigate_workflow(workflow);
                            self.tab = tab;
                        }
                    }
                });
            });
        if !open {
            self.presentation_tour_open = false;
            self.onboarding_preferences
                .gameplay_presentation_tour_completed = true;
            self.save_onboarding_preferences();
        }
    }

    fn choose_chart_level(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open Sonolus LevelData")
            .add_filter("LevelData", &["json", "gz", "zip"])
            .add_filter("All files", &["*"]);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.load_chart_level_path(path);
        }
    }

    fn load_chart_level_path(&mut self, path: PathBuf) {
        match crate::formats::load_level_bounded(&path, crate::chart_studio::MAX_LEVELDATA_BYTES) {
            Ok(mut level) => {
                // Legacy package adapters attach item.json to this compatibility
                // field; item metadata is not part of standalone LevelData.
                if path.is_dir()
                    || path.extension().is_some_and(|extension| {
                        extension.to_string_lossy().eq_ignore_ascii_case("zip")
                    })
                {
                    level.metadata = None;
                }
                let mut document = crate::chart_studio::ChartDocument::new(level);
                document.mark_saved(Some(path));
                if self.chart_studio.document.is_dirty() {
                    self.chart_studio.pending_document = Some(document);
                } else {
                    self.install_chart_document(document);
                }
            }
            Err(error) => {
                self.chart_studio.error = Some(format!("Could not open LevelData: {error:#}"))
            }
        }
    }

    fn install_chart_document(&mut self, document: crate::chart_studio::ChartDocument) {
        self.config.level = document.source.clone().unwrap_or_default();
        if let Some(path) = self.config.music.clone() {
            if self.chart_studio.music_path != path {
                self.set_chart_music_path(path);
            }
        }
        if let Some(grid) = &document.beat_grid {
            self.chart_studio.tempo_bpm_edit = grid.bpm;
            self.chart_studio.tempo_phase_edit = grid.phase_media_seconds;
        }
        self.chart_studio.document = document;
        self.chart_studio.active = true;
        self.chart_studio.selected_entity = None;
        self.chart_studio.selection.clear();
        self.chart_studio.entity_draft.clear();
        self.chart_studio.validation = None;
        self.chart_studio.preview_frame = None;
        self.chart_studio.preview_texture = None;
        self.chart_studio.preview_error = None;
        self.chart_studio.error = None;
    }

    fn new_chart_level(&mut self) {
        let document = crate::chart_studio::ChartDocument::default();
        if self.chart_studio.document.is_dirty() {
            self.chart_studio.pending_document = Some(document);
        } else {
            self.install_chart_document(document);
        }
    }

    fn start_chart_engine_load(&mut self) {
        if self.config.engine.as_os_str().is_empty() {
            self.chart_studio.error = Some("Choose an engine in Project / inputs first.".into());
            return;
        }
        if self.chart_studio.engine_job.is_some() {
            return;
        }
        let path = self.config.engine.clone();
        let (sender, result) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("sono-chart-engine-inspection".into())
            .spawn(move || {
                let summary = crate::formats::load_engine(&path)
                    .and_then(|engine| {
                        let identity = crate::chart_studio::engine_identity(&engine)?;
                        let mut archetypes: Vec<_> = engine
                            .level_archetype_names()
                            .into_iter()
                            .map(str::to_owned)
                            .collect();
                        archetypes.sort();
                        archetypes.dedup();
                        Ok(ChartEngineSummary {
                            path,
                            archetypes,
                            identity,
                        })
                    })
                    .map_err(|error| format!("{error:#}"));
                let _ = sender.send(summary);
            });
        match worker {
            Ok(worker) => self.chart_studio.engine_job = Some(ChartEngineJob { result, worker }),
            Err(error) => {
                self.chart_studio.error =
                    Some(format!("Could not start engine inspection: {error}"))
            }
        }
    }

    fn poll_chart_jobs(&mut self) {
        if let Some(job) = self.chart_studio.engine_job.as_ref() {
            let completion = match job.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("engine inspection worker ended unexpectedly".into()))
                }
            };
            if let Some(completion) = completion {
                let job = self.chart_studio.engine_job.take().unwrap();
                let _ = job.worker.join();
                match completion {
                    Ok(summary) => {
                        self.chart_studio.engine = Some(summary);
                        self.chart_studio.error = None;
                        if let (Some(schema), Some(engine)) =
                            (&self.chart_studio.schema, &self.chart_studio.engine)
                        {
                            let names: Vec<_> =
                                engine.archetypes.iter().map(String::as_str).collect();
                            if let Err(error) =
                                schema.validate_for_identity(&engine.identity, &names)
                            {
                                self.chart_studio.schema_error = Some(format!(
                                    "Schema no longer matches the selected engine: {error:#}"
                                ));
                                self.chart_studio.schema = None;
                            }
                        }
                    }
                    Err(error) => self.chart_studio.error = Some(error),
                }
            }
        }
        if let Some(job) = self.chart_studio.preview_job.as_ref() {
            let completion = match job.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("chart preview worker ended unexpectedly".into()))
                }
            };
            if let Some(completion) = completion {
                let job = self.chart_studio.preview_job.take().unwrap();
                let _ = job.worker.join();
                if job.revision == self.chart_studio.document.revision() {
                    match completion {
                        Ok(frame) => {
                            self.chart_studio.preview_frame = Some(frame);
                            self.chart_studio.preview_texture = None;
                            self.chart_studio.preview_error = None;
                        }
                        Err(error) => self.chart_studio.preview_error = Some(error),
                    }
                }
            }
        }
    }

    fn load_chart_schema(&mut self) {
        let Some(engine) = self.chart_studio.engine.as_ref() else {
            self.chart_studio.schema_error =
                Some("Inspect the selected engine before loading a schema.".into());
            return;
        };
        if engine.path != self.config.engine {
            self.chart_studio.schema_error =
                Some("Inspect the currently selected engine before loading a schema.".into());
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("Load engine authoring schema")
            .add_filter("Authoring schema JSON", &["json"]);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.pick_file() {
            let result = (|| -> Result<crate::chart_studio::AuthoringSchema> {
                let metadata = std::fs::metadata(&path)?;
                if metadata.len() > crate::chart_studio::AUTHORING_SCHEMA_LIMIT {
                    anyhow::bail!("schema exceeds the 1 MiB size limit");
                }
                let schema: crate::chart_studio::AuthoringSchema =
                    serde_json::from_slice(&std::fs::read(&path)?)?;
                let names: Vec<_> = engine.archetypes.iter().map(String::as_str).collect();
                schema.validate_for_identity(&engine.identity, &names)?;
                Ok(schema)
            })();
            match result {
                Ok(schema) => {
                    self.chart_studio.schema = Some(schema);
                    self.chart_studio.schema_path = path;
                    self.chart_studio.schema_error = None;
                }
                Err(error) => {
                    self.chart_studio.schema = None;
                    self.chart_studio.schema_error =
                        Some(format!("Authoring schema rejected: {error:#}"));
                }
            }
        }
    }

    fn current_chart_authoring_schema(&self) -> Option<&crate::chart_studio::AuthoringSchema> {
        if self.chart_studio.engine_job.is_some() {
            return None;
        }
        let engine = self
            .chart_studio
            .engine
            .as_ref()
            .filter(|engine| engine.path == self.config.engine)?;
        let schema = self.chart_studio.schema.as_ref()?;
        let archetypes: Vec<_> = engine.archetypes.iter().map(String::as_str).collect();
        schema
            .validate_for_identity(&engine.identity, &archetypes)
            .ok()
            .map(|()| schema)
    }

    fn save_chart_to_project(&mut self) {
        if self.project.is_none() {
            self.new_project();
        }
        self.sync_project_from_config();
        let previous_snapshot = self
            .project
            .as_ref()
            .and_then(|project| project.extra.get(crate::chart_studio::PROJECT_LEVEL_KEY))
            .cloned();
        let serialized = match crate::chart_studio::project_level_snapshot(
            &self.chart_studio.document.level,
            self.chart_studio.document.beat_grid.as_ref(),
            previous_snapshot.as_ref(),
        ) {
            Ok(value) => value,
            Err(error) => {
                self.chart_studio.error = Some(format!(
                    "Could not serialize chart project snapshot: {error}"
                ));
                return;
            }
        };
        let Some(project) = self.project.as_mut() else {
            return;
        };
        project
            .extra
            .insert(crate::chart_studio::PROJECT_LEVEL_KEY.into(), serialized);
        if let Some(path) = self.project_path.clone() {
            match project.save(&path) {
                Ok(()) => {
                    self.chart_studio.document.mark_saved(None);
                    self.summary = format!(
                        "Saved editable LevelData snapshot in project {}",
                        path.display()
                    );
                    self.project_error = None;
                    self.chart_studio.error = None;
                }
                Err(error) => {
                    self.chart_studio.error =
                        Some(format!("Could not save chart in project: {error:#}"))
                }
            }
        } else {
            self.pick_save_project();
            if self.project_path.is_some() && self.project_error.is_none() {
                self.chart_studio.document.mark_saved(None);
            }
        }
    }

    fn choose_chart_export(&mut self) {
        let suggested = self
            .chart_studio
            .document
            .source
            .as_ref()
            .and_then(|source| source.file_stem().and_then(|name| name.to_str()))
            .map(|stem| format!("{stem}.json.gz"))
            .unwrap_or_else(|| "level.json.gz".into());
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export Sonolus LevelData")
            .add_filter("Gzip LevelData", &["gz"])
            .set_file_name(suggested);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.save_file() {
            self.chart_studio.export_pending = Some(path);
        }
    }

    fn perform_chart_export(&mut self, path: PathBuf, overwrite: bool) {
        match crate::chart_studio::export_level_gzip(
            &self.chart_studio.document.level,
            &path,
            overwrite,
        ) {
            Ok(report) => {
                self.chart_studio.document.mark_saved(Some(path.clone()));
                self.chart_studio.validation = Some(self.current_chart_validation());
                self.chart_studio.error = None;
                self.chart_studio.export_error = None;
                self.chart_studio.last_export = Some(path.clone());
                self.summary = format!(
                    "Exported standalone LevelData: {} (SHA-256 {})",
                    path.display(),
                    report.sha256
                );
                self.config.level = path;
            }
            Err(error) => {
                self.chart_studio.export_error = Some(format!("LevelData export failed: {error:#}"))
            }
        }
    }

    fn request_chart_preview(&mut self) {
        if self.chart_studio.preview_job.is_some() {
            return;
        }
        if self.config.engine.as_os_str().is_empty() {
            self.chart_studio.preview_error =
                Some("Select an engine before previewing this LevelData.".into());
            return;
        }
        let snapshot = self.chart_studio.document.level.clone();
        let revision = self.chart_studio.document.revision();
        let time = self.chart_studio.preview_time.max(0.0);
        let mut config = self.config.clone();
        let (width, height) = preview_dimensions(config.width, config.height);
        config.width = width;
        config.height = height;
        let (sender, result) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("sono-chart-watch-preview".into())
            .spawn(move || {
                let rendered = catch_export_panic(|| {
                    let temporary_directory = tempfile::tempdir()
                        .context("creating temporary chart preview directory")?;
                    let level_path = temporary_directory.path().join("preview-level.json.gz");
                    crate::chart_studio::export_level_gzip(&snapshot, &level_path, false)?;
                    config.level = level_path;
                    let frame = config.render_frame(time)?;
                    let expected = config.width as usize * config.height as usize * 3;
                    if frame.rgb.len() != expected {
                        bail!(
                            "chart preview returned {} RGB bytes; expected {expected}",
                            frame.rgb.len()
                        );
                    }
                    Ok(ChartPreviewFrame {
                        revision,
                        time,
                        width: config.width,
                        height: config.height,
                        rgb: frame.rgb,
                    })
                })
                .map_err(|error| format!("{error:#}"));
                let _ = sender.send(rendered);
            });
        match worker {
            Ok(worker) => {
                self.chart_studio.preview_job = Some(ChartPreviewJob {
                    revision,
                    result,
                    worker,
                });
                self.chart_studio.preview_error = None;
            }
            Err(error) => {
                self.chart_studio.preview_error =
                    Some(format!("Could not start Watch preview: {error}"))
            }
        }
    }

    fn set_chart_music_path(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if self.chart_studio.music_path == path {
            return;
        }
        if let Some(job) = &self.chart_studio.tempo_job {
            job.control.cancel();
        }
        self.chart_studio.tempo_generation = self.chart_studio.tempo_generation.wrapping_add(1);
        self.chart_studio.audio_generation = self.chart_studio.audio_generation.wrapping_add(1);
        self.chart_studio.audio_playback = None;
        self.chart_studio.music_path = path.clone();
        self.config.music = Some(path);
        self.chart_studio.music_duration_seconds = 0.0;
        self.chart_studio.music_waveform.clear();
        self.chart_studio.tempo_analysis = None;
        self.chart_studio.tempo_error = None;
        self.chart_studio.selected_tempo_candidate = 0;
        self.chart_studio.audio_playhead = 0.0;
    }

    fn choose_chart_music(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose Chart Audio")
            .add_filter(
                "Audio",
                &["mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "wma"],
            )
            .add_filter("All files", &["*"]);
        if let Some(parent) = self.parent.as_deref() {
            dialog = dialog.set_parent(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.set_chart_music_path(path);
        }
    }

    fn start_chart_tempo_analysis(&mut self, ctx: &egui::Context) {
        if self.chart_studio.tempo_job.is_some() {
            return;
        }
        let path = self.chart_studio.music_path.clone();
        if !path.is_file() {
            self.chart_studio.tempo_error = Some(
                "Choose an existing audio source, or use the project's Music/BGM input first."
                    .into(),
            );
            return;
        }
        let generation = self.chart_studio.tempo_generation;
        let control = crate::tempo_analysis::AnalysisControl::default();
        let worker_control = control.clone();
        let progress = Arc::new(Mutex::new(crate::tempo_analysis::AnalysisProgress {
            stage: crate::tempo_analysis::AnalysisStage::Hashing,
            decoded_seconds: 0.0,
        }));
        let worker_progress = progress.clone();
        let worker_path = path.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::Builder::new()
            .name("sono-chart-tempo-analysis".into())
            .spawn(move || {
                let analyzed = catch_export_panic(|| {
                    // Use the ordinary media probe and waveform producer, but
                    // require existing FFmpeg so analysis remains offline and
                    // never provisions optional tools in the background.
                    let (duration, waveform) =
                        crate::composition::inspect_audio_with_existing_tools(&worker_path)?;
                    let analysis = crate::tempo_analysis::analyze_file(
                        &worker_path,
                        &worker_control,
                        |state| {
                            if let Ok(mut progress) = worker_progress.lock() {
                                *progress = state;
                            }
                        },
                    )?;
                    Ok((analysis, duration, waveform))
                })
                .map_err(|error| format!("{error:#}"));
                let _ = tx.send(analyzed);
                wake.request_repaint();
            });
        match worker {
            Ok(worker) => {
                self.chart_studio.tempo_job = Some(ChartTempoJob {
                    generation,
                    path,
                    control,
                    progress,
                    result,
                    worker,
                });
                self.chart_studio.tempo_error = None;
            }
            Err(error) => {
                self.chart_studio.tempo_error =
                    Some(format!("Could not start tempo analysis: {error}"));
            }
        }
    }

    fn poll_chart_tempo_analysis(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.chart_studio.tempo_job else {
            return;
        };
        let completed = match job.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("tempo analysis worker stopped unexpectedly".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        let Some(completed) = completed else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let job = self
            .chart_studio
            .tempo_job
            .take()
            .expect("tempo job was present");
        let _ = job.worker.join();
        let current = job.generation == self.chart_studio.tempo_generation
            && job.path == self.chart_studio.music_path;
        if !current || job.control.is_cancelled() {
            return;
        }
        match completed {
            Ok((analysis, duration, waveform)) => {
                self.chart_studio.music_duration_seconds = duration;
                self.chart_studio.music_waveform = waveform;
                self.chart_studio.selected_tempo_candidate = 0;
                if let Some(primary) = analysis.primary() {
                    self.chart_studio.tempo_bpm_edit = primary.bpm;
                    self.chart_studio.tempo_phase_edit = primary.phase_seconds;
                }
                self.chart_studio.tempo_analysis = Some(analysis);
                self.chart_studio.tempo_error = None;
            }
            Err(error) => self.chart_studio.tempo_error = Some(error),
        }
    }

    fn start_chart_audio_playback(&mut self, ctx: &egui::Context) {
        if self.chart_studio.audio_playback_start.is_some() {
            return;
        }
        if self
            .chart_studio
            .audio_playback
            .as_ref()
            .is_some_and(|playback| !playback.is_ended())
        {
            self.chart_studio
                .audio_playback
                .as_ref()
                .unwrap()
                .set_playing(true);
            return;
        }
        let path = self.chart_studio.music_path.clone();
        if !path.is_file() {
            self.chart_studio.tempo_error =
                Some("Choose an existing audio source before auditioning.".into());
            return;
        }
        let generation = self.chart_studio.audio_generation;
        let start = self
            .chart_studio
            .audio_playhead
            .clamp(0.0, self.chart_studio.music_duration_seconds);
        let worker_path = path.clone();
        let (tx, result) = mpsc::sync_channel(1);
        let wake = ctx.clone();
        let worker = std::thread::Builder::new()
            .name("sono-chart-audio-preview".into())
            .spawn(move || {
                let playback = catch_export_panic(|| {
                    let project = crate::composition::CompositionProject::
                        single_audio_preview_with_existing_tools(&worker_path)?;
                    crate::timeline_playback::TimelinePlayback::start(&project, start, false)
                })
                .map_err(|error| format!("{error:#}"));
                let _ = tx.send(playback);
                wake.request_repaint();
            });
        match worker {
            Ok(worker) => {
                self.chart_studio.audio_playback_start = Some(ChartAudioPlaybackStart {
                    generation,
                    path,
                    result,
                    worker,
                });
                self.chart_studio.tempo_error = None;
            }
            Err(error) => {
                self.chart_studio.tempo_error =
                    Some(format!("Could not start audio preview: {error}"))
            }
        }
    }

    fn poll_chart_audio_playback(&mut self, ctx: &egui::Context) {
        let Some(start) = &self.chart_studio.audio_playback_start else {
            if let Some(playback) = &mut self.chart_studio.audio_playback {
                self.chart_studio.audio_playhead = playback.position_seconds();
                let _ = playback.next_video_frame();
                if let Some(error) = playback.take_error() {
                    self.chart_studio.tempo_error = Some(format!("Audio preview failed: {error}"));
                    self.chart_studio.audio_playback = None;
                } else if playback.is_ended() {
                    self.chart_studio.audio_playhead = self.chart_studio.music_duration_seconds;
                    self.chart_studio.audio_playback = None;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
            }
            return;
        };
        let completed = match start.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("audio preview startup stopped unexpectedly".into()))
            }
            Err(mpsc::TryRecvError::Empty) => None,
        };
        let Some(completed) = completed else {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        };
        let start = self
            .chart_studio
            .audio_playback_start
            .take()
            .expect("playback start was present");
        let _ = start.worker.join();
        let current = start.generation == self.chart_studio.audio_generation
            && start.path == self.chart_studio.music_path;
        if !current {
            return;
        }
        match completed {
            Ok(playback) => {
                self.chart_studio.audio_playhead = playback.position_seconds();
                self.chart_studio.audio_playback = Some(playback);
                self.chart_studio.tempo_error = None;
            }
            Err(error) => {
                self.chart_studio.tempo_error = Some(format!("Audio preview failed: {error}"))
            }
        }
    }

    fn seek_chart_audio(&mut self, ctx: &egui::Context, position: f64) {
        let active = self.chart_studio.audio_playback.is_some();
        let was_playing = self
            .chart_studio
            .audio_playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing());
        self.chart_studio.audio_generation = self.chart_studio.audio_generation.wrapping_add(1);
        self.chart_studio.audio_playback = None;
        self.chart_studio.audio_playhead =
            position.clamp(0.0, self.chart_studio.music_duration_seconds.max(0.0));
        if active {
            self.start_chart_audio_playback(ctx);
            if !was_playing {
                if let Some(playback) = &self.chart_studio.audio_playback {
                    playback.set_playing(false);
                }
            }
        }
    }

    fn chart_audio_tempo_ui(&mut self, ui: &mut egui::Ui) {
        let mut choose = false;
        let mut use_project_music = false;
        let mut analyze = false;
        let mut cancel = false;
        let mut apply = false;
        let mut clear_grid = false;
        let mut play = false;
        let mut stop = false;
        let mut tapped = false;
        let mut seek_to = None;
        ui.collapsing("Song waveform, tempo and beat reference", |ui| {
            ui.label("Analysis is a local proposal. Apply writes an authoring-only reference grid; it does not alter entity timestamps, engine-defined BPM entities, or bgmOffset.");
            ui.horizontal_wrapped(|ui| {
                choose = ui.button("Choose chart audio...").clicked();
                use_project_music = ui.button("Use current project Music/BGM").clicked();
                let busy = self.chart_studio.tempo_job.is_some();
                let command = crate::workspace::COMMANDS.iter().find(|command| command.id == crate::workspace::CommandId::AnalyzeTempo).expect("registered command");
                analyze = ui.add_enabled(!busy && self.chart_studio.music_path.is_file(), egui::Button::new(format!("Analyze Tempo ({})", self.shortcut_label(command)))).clicked();
                if busy {
                    cancel = ui.button("Cancel analysis").clicked();
                }
            });
            if self.chart_studio.music_path.as_os_str().is_empty() {
                ui.weak("No chart audio selected. Local audio import and the existing URL acquisition flow are both supported.");
            } else {
                ui.small(format!("Audio source: {}", self.chart_studio.music_path.display()));
            }
            if let Some(job) = &self.chart_studio.tempo_job {
                let state = job.progress.lock().ok().map(|state| *state);
                if let Some(state) = state {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(match state.stage {
                            crate::tempo_analysis::AnalysisStage::Hashing => "Identifying audio content...".to_owned(),
                            crate::tempo_analysis::AnalysisStage::CacheLookup => "Checking tempo-analysis cache...".to_owned(),
                            crate::tempo_analysis::AnalysisStage::Decoding => format!("Decoding {:.1}s of audio...", state.decoded_seconds),
                            crate::tempo_analysis::AnalysisStage::Estimating => "Estimating beat periodicity and phase...".to_owned(),
                            crate::tempo_analysis::AnalysisStage::Complete => "Tempo analysis complete.".to_owned(),
                        });
                    });
                }
            }
            if let Some(error) = &self.chart_studio.tempo_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            if let Some(analysis) = &self.chart_studio.tempo_analysis {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Quality: {}", analysis.quality.label()));
                    ui.label(format!("Heuristic periodicity: {:.2}", analysis.confidence_score));
                    ui.label(if analysis.from_cache { "Content cache hit" } else { "Analyzed locally" });
                    if let Some(primary) = analysis.primary() {
                        ui.label(format!("Estimated {:.2} BPM · media phase {:.3}s", primary.bpm, primary.phase_seconds));
                    } else {
                        ui.weak("No tempo candidate passed the reliability floor.");
                    }
                });
                if !analysis.candidates.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Compare metrical interpretations:");
                        for (index, candidate) in analysis.candidates.iter().enumerate() {
                            let label = format!("{:.2} BPM", candidate.bpm);
                            if ui.selectable_label(self.chart_studio.selected_tempo_candidate == index, label).clicked() {
                                self.chart_studio.selected_tempo_candidate = index;
                                self.chart_studio.tempo_bpm_edit = candidate.bpm;
                                self.chart_studio.tempo_phase_edit = candidate.phase_seconds;
                            }
                        }
                    });
                    ui.small("Half/double candidates are alternate beat subdivisions, not competing certainty claims. Meter and downbeat are not inferred.");
                if analysis.tempo_stability == crate::tempo_analysis::TempoStability::Varies {
                    ui.colored_label(egui::Color32::YELLOW, "Different local sections suggest different tempos. This global grid is only a rough proposal; no variable-tempo map was generated.");
                } else if analysis.tempo_stability == crate::tempo_analysis::TempoStability::Uncertain {
                    ui.weak("There was not enough stable local evidence to assess tempo consistency.");
                }
                }
                ui.small(format!("Decoded {:.1}s · decode {} ms · estimate {} ms · cache lookup {} ms · detector {}", analysis.decoded_duration_seconds, analysis.decode_ms, analysis.analysis_ms, analysis.cache_lookup_ms, analysis.detector_version));
            }
            if let Some(analysis) = &self.chart_studio.tempo_analysis {
                ui.small(format!("Content identity checked in {} ms", analysis.source_identity_ms));
                if analysis.from_cache {
                    ui.weak("Cache hit: decode and tempo estimation were skipped for this run.");
                }
            }
            let is_playing = self.chart_studio.audio_playback.as_ref().is_some_and(|playback| playback.is_playing());
            ui.horizontal(|ui| {
                play = ui.button(if is_playing { "Pause audio" } else { "Play / resume audio" }).clicked();
                stop = ui.button("Stop audio").clicked();
                let duration = self.chart_studio.music_duration_seconds.max(0.0);
                let mut playhead = self.chart_studio.audio_playhead;
                if ui.add_enabled(duration > 0.0, egui::Slider::new(&mut playhead, 0.0..=duration).show_value(true).text("Audio position")).changed() {
                    seek_to = Some(playhead);
                }
            });
            ui.horizontal(|ui| {
                ui.label("BPM");
                ui.add(egui::DragValue::new(&mut self.chart_studio.tempo_bpm_edit).range(20.0..=400.0).speed(0.01).fixed_decimals(2));
                ui.label("Beat phase (audio media seconds)");
                ui.add(egui::DragValue::new(&mut self.chart_studio.tempo_phase_edit).range(-86_400.0..=86_400.0).speed(0.001).fixed_decimals(3));
                if ui.small_button("−10 ms").on_hover_text("Nudge the proposed phase 10 milliseconds earlier").clicked() {
                    self.chart_studio.tempo_phase_edit -= 0.01;
                }
                if ui.small_button("+10 ms").on_hover_text("Nudge the proposed phase 10 milliseconds later").clicked() {
                    self.chart_studio.tempo_phase_edit += 0.01;
                }
                apply = ui.button("Apply reference grid").clicked();
                if self.chart_studio.document.beat_grid.is_some() {
                    clear_grid = ui.button("Clear applied grid").clicked();
                }
            });
            ui.horizontal(|ui| {
                tapped = ui.button("Tap Tempo").clicked();
                if let Some(bpm) = self.chart_studio.tap_tempo_bpm {
                    ui.label(format!("Tap estimate: {bpm:.2} BPM (phase unchanged)"));
                } else {
                    ui.weak("Tap several beats to estimate BPM; align phase with the field or 10 ms nudges.");
                }
            });
            if let Some(grid) = &self.chart_studio.document.beat_grid {
                ui.colored_label(egui::Color32::LIGHT_GREEN, format!("Applied authoring grid: {:.2} BPM, media phase {:.3}s. LevelData entity times remain unchanged.", grid.bpm, grid.phase_media_seconds));
                if let Some(analysis) = &self.chart_studio.tempo_analysis {
                    if grid.source_sha256.as_deref().is_some_and(|hash| hash != analysis.source_sha256) {
                        ui.colored_label(egui::Color32::YELLOW, "The applied grid came from different audio content than the currently selected source.");
                    }
                }
            }
            let grid = self.chart_studio.document.beat_grid.as_ref();
            let bpm = if self.chart_studio.tempo_bpm_edit.is_finite() && self.chart_studio.tempo_bpm_edit > 0.0 {
                self.chart_studio.tempo_bpm_edit
            } else {
                grid.map_or(0.0, |grid| grid.bpm)
            };
            let phase = self.chart_studio.tempo_phase_edit;
            chart_waveform_grid(
                ui,
                &self.chart_studio.music_waveform,
                self.chart_studio.music_duration_seconds,
                bpm,
                phase,
                self.chart_studio.audio_playhead,
            );
            ui.small("The grid uses decoded audio media time. Sonolus timing conversion remains media time = Watch time + bgmOffset. Analysis never edits bgmOffset.");
        });
        if choose {
            self.choose_chart_music();
        }
        if use_project_music {
            if let Some(path) = self.config.music.clone() {
                self.set_chart_music_path(path);
            } else {
                self.chart_studio.tempo_error =
                    Some("This project has no Music/BGM input assigned yet.".into());
            }
        }
        if analyze {
            self.dispatch_command(crate::workspace::CommandId::AnalyzeTempo, ui.ctx());
        }
        if cancel {
            if let Some(job) = &self.chart_studio.tempo_job {
                job.control.cancel();
            }
        }
        if play {
            if let Some(playback) = &self.chart_studio.audio_playback {
                playback.set_playing(!playback.is_playing());
            } else {
                self.start_chart_audio_playback(ui.ctx());
            }
        }
        if stop {
            self.chart_studio.audio_generation = self.chart_studio.audio_generation.wrapping_add(1);
            self.chart_studio.audio_playback = None;
            self.chart_studio.audio_playhead = 0.0;
        }
        if let Some(position) = seek_to {
            self.seek_chart_audio(ui.ctx(), position);
        }
        if tapped {
            let now = Instant::now();
            self.chart_studio.tap_times.push_back(now);
            while self.chart_studio.tap_times.len() > 9 {
                self.chart_studio.tap_times.pop_front();
            }
            if self.chart_studio.tap_times.len() >= 3 {
                let times = self
                    .chart_studio
                    .tap_times
                    .iter()
                    .copied()
                    .collect::<Vec<_>>();
                let intervals = times
                    .windows(2)
                    .map(|pair| pair[1].duration_since(pair[0]).as_secs_f64())
                    .collect::<Vec<_>>();
                self.chart_studio.tap_tempo_bpm = crate::tempo_analysis::tap_tempo_bpm(&intervals);
                if let Some(bpm) = self.chart_studio.tap_tempo_bpm {
                    self.chart_studio.tempo_bpm_edit = bpm;
                }
            }
        }
        if apply {
            let source_sha256 = self
                .chart_studio
                .tempo_analysis
                .as_ref()
                .map(|analysis| analysis.source_sha256.clone());
            let grid = crate::chart_studio::ChartBeatGrid {
                bpm: self.chart_studio.tempo_bpm_edit,
                phase_media_seconds: self.chart_studio.tempo_phase_edit,
                source_sha256,
            };
            match self.chart_studio.document.set_beat_grid(Some(grid)) {
                Ok(()) => {
                    self.chart_studio.validation = None;
                    self.chart_studio.tempo_error = None;
                }
                Err(error) => {
                    self.chart_studio.tempo_error =
                        Some(format!("Could not apply reference grid: {error:#}"))
                }
            }
        }
        if clear_grid {
            if let Err(error) = self.chart_studio.document.set_beat_grid(None) {
                self.chart_studio.tempo_error =
                    Some(format!("Could not clear reference grid: {error:#}"));
            }
        }
    }

    fn chart_studio_ui(&mut self, ui: &mut egui::Ui) {
        self.update_chart_timeline();
        let authoring_schema = self.current_chart_authoring_schema().cloned();
        ui.heading(self.app_text("chart.title"));
        ui.label(self.app_text("chart.description"));
        self.media_acquisition_ui(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("New LevelData").clicked() {
                self.new_chart_level();
            }
            if ui.button("Open LevelData...").clicked() {
                self.choose_chart_level();
            }
            if ui
                .add_enabled(
                    self.chart_studio.document.can_undo(),
                    egui::Button::new(self.app_text("common.undo")),
                )
                .clicked()
            {
                self.chart_studio.document.undo();
                self.chart_studio.selected_entity = None;
                self.chart_studio.selection.clear();
                self.chart_studio.validation = None;
            }
            if ui
                .add_enabled(
                    self.chart_studio.document.can_redo(),
                    egui::Button::new(self.app_text("common.redo")),
                )
                .clicked()
            {
                self.chart_studio.document.redo();
                self.chart_studio.selected_entity = None;
                self.chart_studio.selection.clear();
                self.chart_studio.validation = None;
            }
            if ui.button(self.app_text("chart.inspect_engine")).clicked() {
                self.start_chart_engine_load();
            }
            if ui.button("Load authoring schema...").clicked() {
                self.load_chart_schema();
            }
            if ui.button("Save in render project").clicked() {
                self.save_chart_to_project();
            }
            if ui.button(self.app_text("chart.export_level")).clicked() {
                self.dispatch_command(crate::workspace::CommandId::ExportLevelData, ui.ctx());
            }
        });
        ui.horizontal(|ui| {
            ui.label(if self.chart_studio.document.is_dirty() {
                "Unsaved chart edits"
            } else {
                "Chart document saved"
            });
            if let Some(source) = &self.chart_studio.document.source {
                ui.small(source.display().to_string());
            }
            ui.label(format!(
                "{} entities",
                self.chart_studio.document.level.entities.len()
            ));
        });
        if let Some(error) = &self.chart_studio.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(error) = &self.chart_studio.schema_error {
            ui.colored_label(egui::Color32::YELLOW, error);
        }
        if let Some(error) = &self.chart_studio.export_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(path) = self.chart_studio.last_export.clone() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Exported and production-parser validated: {}",
                    path.display()
                ));
                if ui.button("Open Folder").clicked() {
                    if let Some(parent) = path.parent() {
                        open_folder(parent);
                    }
                }
                if ui.button("Use in render inputs").clicked() {
                    self.config.level = path.clone();
                    self.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
                }
            });
        }
        if self.chart_studio.engine_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading engine archetypes and computing content identity...");
            });
        }
        if let Some(engine) = self
            .chart_studio
            .engine
            .as_ref()
            .filter(|engine| engine.path == self.config.engine)
        {
            ui.small(format!(
                "{} engine archetypes available | identity {}…",
                engine.archetypes.len(),
                &engine.identity.content_sha256[..12]
            ));
        } else if self.chart_studio.engine.is_some() {
            ui.colored_label(egui::Color32::YELLOW, "The configured engine changed. Inspect it again before using this palette or schema.");
        }
        if let Some(schema) = &self.chart_studio.schema {
            if authoring_schema.is_some() {
                ui.small(format!(
                    "Authoring schema v{} matches this engine content; {} archetypes described.",
                    schema.schema_version,
                    schema.archetypes.len()
                ));
            } else {
                ui.colored_label(egui::Color32::YELLOW, "The loaded authoring schema is stale or belongs to another inspected engine. Generic / Raw Mode is active until a matching schema is loaded.");
            }
        }
        if authoring_schema.is_none() {
            ui.group(|ui| {
                ui.strong(self.app_text("chart.generic_mode"));
                ui.label(self.app_text("chart.raw_notice"));
            });
        } else {
            ui.group(|ui| {
                ui.strong("Schema-backed authoring");
                ui.label("The matching schema supplies declared labels, field constraints, optional creation data, and timing bindings. Other fields remain raw canonical LevelData.");
            });
        }

        let mut bgm_offset = self.chart_studio.document.level.bgm_offset.unwrap_or(0.0);
        ui.horizontal(|ui| {
            ui.label("Level bgmOffset (seconds)");
            if ui
                .add(egui::DragValue::new(&mut bgm_offset).speed(0.01))
                .changed()
            {
                match self.chart_studio.document.set_bgm_offset(Some(bgm_offset)) {
                    Ok(()) => {
                        self.chart_studio.validation = None;
                    }
                    Err(error) => {
                        self.chart_studio.error = Some(format!("Invalid bgmOffset: {error:#}"))
                    }
                }
            }
            if ui.button("Unset").clicked() {
                let _ = self.chart_studio.document.set_bgm_offset(None);
                self.chart_studio.validation = None;
            }
            if ui.button("Validate structure").clicked() {
                self.chart_studio.validation = Some(self.current_chart_validation());
            }
            if ui.button("Preview at Watch time").clicked() {
                self.request_chart_preview();
            }
            ui.add(
                egui::DragValue::new(&mut self.chart_studio.preview_time)
                    .range(0.0..=86_400.0)
                    .speed(0.05)
                    .suffix(" s"),
            );
        });
        self.chart_audio_tempo_ui(ui);
        if let Some(validation) = &self.chart_studio.validation {
            if validation.is_valid() {
                ui.colored_label(egui::Color32::LIGHT_GREEN, format!("Structural checks passed for {} entities. This does not replace engine compatibility/runtime validation.", validation.entity_count));
            } else {
                for error in &validation.errors {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            }
            for warning in &validation.warnings {
                ui.colored_label(egui::Color32::YELLOW, warning);
            }
            if let Some(schema) = &authoring_schema {
                for finding in schema.validate_level(&self.chart_studio.document.level) {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Schema convention: {finding}"),
                    );
                }
            }
        }
        ui.separator();
        ui.collapsing("Timeline (schema-declared timing fields and reference grid)", |ui| {
            if self.chart_studio.timeline_rows.is_empty() {
                ui.weak("No matching explicit timing descriptors are available. Entity time stays engine-defined and editable in raw JSON.");
            } else {
                egui::ScrollArea::vertical().id_salt("chart-timeline").max_height(120.0).show_rows(ui, 22.0, self.chart_studio.timeline_rows.len(), |ui, rows| {
                    for row in rows {
                        let (index, time, unit) = self.chart_studio.timeline_rows[row];
                        let unit_label = match unit {
                            crate::chart_studio::TimingUnit::WatchSeconds => "Watch s",
                            crate::chart_studio::TimingUnit::MediaSeconds => "media s",
                        };
                        let grid_annotation = self.chart_studio.document.beat_grid.as_ref().and_then(|grid| {
                            let media_time = match unit {
                                crate::chart_studio::TimingUnit::WatchSeconds => crate::export_timeline::watch_to_media_time(time, self.chart_studio.document.level.bgm_offset.unwrap_or(0.0)),
                                crate::chart_studio::TimingUnit::MediaSeconds => time,
                            };
                            let beat = ((media_time - grid.phase_media_seconds) / (60.0 / grid.bpm)).round();
                            (beat >= 0.0).then(|| {
                                let delta = media_time - (grid.phase_media_seconds + beat * 60.0 / grid.bpm);
                                format!(" | nearest beat {:.0} ({delta:+.3}s)", beat)
                            })
                        }).unwrap_or_default();
                        if ui.selectable_label(self.chart_studio.selected_entity == Some(index), format!("{time:.3} {unit_label}{grid_annotation} | entity {index}")).clicked() {
                            self.chart_studio.selected_entity = Some(index);
                            self.chart_studio.selection.clear();
                            self.chart_studio.selection.insert(index);
                            self.refresh_chart_entity_draft();
                            let watch_time = match unit {
                                crate::chart_studio::TimingUnit::WatchSeconds => time,
                                crate::chart_studio::TimingUnit::MediaSeconds => crate::export_timeline::media_to_watch_time(time, self.chart_studio.document.level.bgm_offset.unwrap_or(0.0)),
                            };
                            self.chart_studio.preview_time = watch_time.max(0.0);
                        }
                    }
                });
            }
        });
        ui.separator();
        ui.columns(2, |columns| {
            let left = &mut columns[0];
            left.heading("Archetypes");
            left.add(egui::TextEdit::singleline(&mut self.chart_studio.archetype_filter).hint_text("Filter engine archetypes"));
            if let Some(engine) = self.chart_studio.engine.as_ref().filter(|engine| engine.path == self.config.engine) {
                if engine.archetypes.is_empty() {
                    left.weak("This package did not provide EnginePlayData archetype declarations. Use a raw archetype id; Watch callback names are not treated as level declarations.");
                } else {
                    let filter = self.chart_studio.archetype_filter.to_lowercase();
                    let visible: Vec<_> = engine.archetypes.iter().filter(|name| name.to_lowercase().contains(&filter)).collect();
                    egui::ScrollArea::vertical().id_salt("chart-archetypes").max_height(170.0).show_rows(left, 22.0, visible.len(), |ui, rows| {
                        for row in rows {
                            let name = visible[row];
                            let label = authoring_schema.as_ref().map(|schema| schema.archetype_label(name)).unwrap_or(name.as_str());
                            if ui.selectable_label(false, label).on_hover_text(name).clicked() {
                                self.chart_studio.new_archetype = name.clone();
                            }
                        }
                    });
                }
            } else {
                left.weak("Inspect an engine to populate its actual archetype palette. Until then, arbitrary archetype strings remain editable.");
            }
            left.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.chart_studio.new_archetype).hint_text("Raw archetype id").desired_width(170.0));
                if ui.button("Add entity").clicked() {
                    let index = self.chart_studio.document.level.entities.len();
                    let archetype = self.chart_studio.new_archetype.trim().to_owned();
                    let data = authoring_schema
                        .as_ref()
                        .and_then(|schema| schema.archetype(&archetype))
                        .and_then(|info| info.initial_data.clone())
                        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
                    let entity = crate::formats::LevelEntity {
                        archetype: serde_json::Value::String(archetype),
                        data,
                        extra: serde_json::Map::new(),
                    };
                    match self.chart_studio.document.insert_entity(index, entity) {
                        Ok(()) => { self.chart_studio.selected_entity = Some(index); self.chart_studio.selection = [index].into_iter().collect(); self.refresh_chart_entity_draft(); self.chart_studio.validation = None; }
                        Err(error) => self.chart_studio.error = Some(format!("Could not add entity: {error:#}")),
                    }
                }
            });
            left.heading("Entities");
            left.horizontal(|ui| {
                if ui.button("Copy selected").clicked() {
                    self.chart_studio.entity_clipboard = self.chart_studio.selection.iter().filter_map(|index| self.chart_studio.document.level.entities.get(*index).cloned()).collect();
                }
                if ui.button("Cut selected").clicked() {
                    let mut indices: Vec<_> = self.chart_studio.selection.iter().copied().collect(); indices.sort_unstable_by(|a,b| b.cmp(a));
                    for index in indices { let _ = self.chart_studio.document.delete_entity(index); }
                    self.chart_studio.selection.clear(); self.chart_studio.selected_entity = None; self.chart_studio.validation = None;
                }
                if ui.add_enabled(!self.chart_studio.entity_clipboard.is_empty(), egui::Button::new("Paste")).clicked() {
                    let start = self.chart_studio.document.level.entities.len();
                    for (offset, entity) in self.chart_studio.entity_clipboard.clone().into_iter().enumerate() { let _ = self.chart_studio.document.insert_entity(start + offset, entity); }
                    self.chart_studio.selection = (start..self.chart_studio.document.level.entities.len()).collect();
                    self.chart_studio.selected_entity = self.chart_studio.selection.first().copied(); self.refresh_chart_entity_draft(); self.chart_studio.validation = None;
                }
            });
            left.small("Copied raw entities keep their exact references; the editor does not guess engine identity/reference rules.");
            let entity_count = self.chart_studio.document.level.entities.len();
            egui::ScrollArea::vertical().id_salt("chart-entities").max_height(260.0).show_rows(left, 25.0, entity_count, |ui, rows| {
                for index in rows {
                    let archetype = self.chart_studio.document.level.entities[index].archetype.as_str().unwrap_or("<non-string archetype>").to_owned();
                    ui.horizontal(|ui| {
                        let mut checked = self.chart_studio.selection.contains(&index);
                        if ui.checkbox(&mut checked, "").changed() {
                            if checked { self.chart_studio.selection.insert(index); } else { self.chart_studio.selection.remove(&index); }
                        }
                        if ui.selectable_label(self.chart_studio.selected_entity == Some(index), format!("{index}: {archetype}")).clicked() {
                            self.chart_studio.selected_entity = Some(index);
                            if !ui.input(|input| input.modifiers.ctrl || input.modifiers.command) {
                                self.chart_studio.selection.clear(); self.chart_studio.selection.insert(index);
                            } else if self.chart_studio.selection.contains(&index) { self.chart_studio.selection.remove(&index); } else { self.chart_studio.selection.insert(index); }
                            self.refresh_chart_entity_draft();
                        }
                    });
                }
            });
            let right = &mut columns[1];
            right.heading(if authoring_schema.is_some() {
                "Entity inspector"
            } else {
                "Generic entity inspector"
            });
            if let Some(index) = self.chart_studio.selected_entity.filter(|index| *index < self.chart_studio.document.level.entities.len()) {
                let entity = &self.chart_studio.document.level.entities[index];
                right.label(format!("Entity {index} | {} selected", self.chart_studio.selection.len().max(1)));
                right.label(format!("Archetype: {}", entity.archetype));
                if let Some(schema) = &authoring_schema {
                    if let Some(info) = schema.archetypes.iter().find(|item| item.id == entity.archetype.as_str().unwrap_or_default()) {
                        if let Some(category) = &info.category { right.small(format!("Category: {category}")); }
                        for property in &info.properties {
                            right.small(format!("{}{}", property.label.as_deref().unwrap_or(&property.path), property.description.as_deref().map(|value| format!("  E{value}")).unwrap_or_default()));
                        }
                    }
                }
                right.add(egui::TextEdit::multiline(&mut self.chart_studio.entity_draft).font(egui::TextStyle::Monospace).desired_rows(14).desired_width(f32::INFINITY));
                self.chart_schema_property_widgets(right, index);
                right.horizontal(|ui| {
                    if ui.button("Apply raw JSON").clicked() {
                        match serde_json::from_str::<crate::formats::LevelEntity>(&self.chart_studio.entity_draft) {
                            Ok(entity) => match self.chart_studio.document.replace_entity(index, entity) {
                                Ok(()) => { self.chart_studio.validation = None; }
                                Err(error) => self.chart_studio.error = Some(format!("Entity edit failed: {error:#}")),
                            },
                            Err(error) => self.chart_studio.error = Some(format!("Invalid entity JSON; current LevelData is unchanged: {error}")),
                        }
                    }
                    if ui.button("Duplicate").clicked() {
                        match self.chart_studio.document.duplicate_entity(index) {
                            Ok(()) => { self.chart_studio.selected_entity = Some(index + 1); self.chart_studio.selection = [index + 1].into_iter().collect(); self.refresh_chart_entity_draft(); self.chart_studio.validation = None; }
                            Err(error) => self.chart_studio.error = Some(format!("Duplicate failed: {error:#}")),
                        }
                    }
                    if ui.button("Delete").clicked()
                        && self.chart_studio.document.delete_entity(index).is_ok()
                    {
                        self.chart_studio.selected_entity = None;
                        self.chart_studio.selection.clear();
                        self.chart_studio.entity_draft.clear();
                        self.chart_studio.validation = None;
                    }
                    if index > 0 && ui.button("Move up").clicked() { let _ = self.chart_studio.document.move_entity(index, index - 1); self.chart_studio.selected_entity = Some(index - 1); self.refresh_chart_entity_draft(); }
                    if index + 1 < self.chart_studio.document.level.entities.len() && ui.button("Move down").clicked() { let _ = self.chart_studio.document.move_entity(index, index + 1); self.chart_studio.selected_entity = Some(index + 1); self.refresh_chart_entity_draft(); }
                });
                right.small("Raw JSON edits commit only after parsing; unknown entity fields are retained. Property edits currently apply to one entity at a time.");
            } else {
                right.weak("Select an entity to inspect and edit its canonical LevelData fields.");
            }
        });
        if self.chart_studio.preview_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Running a real Watch frame from an immutable LevelData snapshot...");
            });
        }
        if let Some(error) = &self.chart_studio.preview_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(frame) = &self.chart_studio.preview_frame {
            if frame.revision != self.chart_studio.document.revision() {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Chart has changed since this preview; render again to update it.",
                );
            }
            ui.small(format!(
                "Watch preview at {:.3}s | {}x{} | revision {}",
                frame.time, frame.width, frame.height, frame.revision
            ));
            if self.chart_studio.preview_texture.is_none() {
                let image = egui::ColorImage::from_rgb(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgb,
                );
                self.chart_studio.preview_texture = Some(ui.ctx().load_texture(
                    "sono-chart-watch-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            if let Some(texture) = &self.chart_studio.preview_texture {
                let size = texture.size_vec2();
                let scale = (ui.available_width() / size.x).min(1.0);
                ui.image((texture.id(), size * scale));
            }
        }
        if self.chart_studio.pending_document.is_some() {
            egui::Window::new("Discard unsaved LevelData edits?").collapsible(false).show(ui.ctx(), |ui| {
                ui.label("The current chart has edits that are not saved in the render project or exported LevelData.");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() { self.chart_studio.pending_document = None; }
                    if ui.button("Discard and continue").clicked() {
                        if let Some(document) = self.chart_studio.pending_document.take() { self.install_chart_document(document); }
                    }
                });
            });
        }
        if let Some(path) = self.chart_studio.export_pending.clone() {
            egui::Window::new("Export standalone LevelData").collapsible(false).show(ui.ctx(), |ui| {
                ui.label("Destination"); ui.label(path.display().to_string());
                ui.label("The output is a standalone deterministic gzip-compressed Sonolus LevelData file.");
                if path.exists() { ui.colored_label(egui::Color32::YELLOW, "This file exists. Replacement requires explicit confirmation."); }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.chart_studio.export_pending = None; }
                    if ui.button(if path.exists() { "Overwrite" } else { "Export" }).clicked() {
                        self.chart_studio.export_pending = None;
                        self.perform_chart_export(path.clone(), path.exists());
                    }
                });
            });
        }
    }

    fn refresh_chart_entity_draft(&mut self) {
        self.chart_studio.entity_draft = self
            .chart_studio
            .selected_entity
            .and_then(|index| self.chart_studio.document.level.entities.get(index))
            .and_then(|entity| serde_json::to_string_pretty(entity).ok())
            .unwrap_or_default();
    }

    fn update_chart_timeline(&mut self) {
        let revision = self.chart_studio.document.revision();
        let schema_identity = self
            .current_chart_authoring_schema()
            .map(|schema| schema.engine.content_sha256.to_ascii_lowercase());
        if self.chart_studio.timeline_revision == revision
            && self.chart_studio.timeline_schema_identity == schema_identity
        {
            return;
        }
        let mut rows = Vec::new();
        if let Some(schema) = self.current_chart_authoring_schema() {
            for (index, entity) in self.chart_studio.document.level.entities.iter().enumerate() {
                let Some(archetype) = entity.archetype.as_str() else {
                    continue;
                };
                let Some(timing) = schema
                    .archetypes
                    .iter()
                    .find(|item| item.id == archetype)
                    .and_then(|item| item.timing.as_ref())
                else {
                    continue;
                };
                let Some(time) = entity
                    .data
                    .pointer(&timing.path)
                    .and_then(serde_json::Value::as_f64)
                    .filter(|time| time.is_finite())
                else {
                    continue;
                };
                rows.push((index, time, timing.unit));
            }
        }
        rows.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
        self.chart_studio.timeline_rows = rows;
        self.chart_studio.timeline_revision = revision;
        self.chart_studio.timeline_schema_identity = schema_identity;
    }

    fn current_chart_validation(&self) -> crate::chart_studio::LevelValidation {
        let names: Vec<&str> = self
            .chart_studio
            .engine
            .as_ref()
            .filter(|engine| engine.path == self.config.engine)
            .map(|engine| engine.archetypes.iter().map(String::as_str).collect())
            .unwrap_or_default();
        crate::chart_studio::validate_level_with_archetypes(
            &self.chart_studio.document.level,
            self.chart_studio
                .engine
                .as_ref()
                .filter(|engine| engine.path == self.config.engine)
                .map(|_| names.as_slice()),
        )
    }

    fn chart_schema_property_widgets(&mut self, ui: &mut egui::Ui, index: usize) {
        let archetype = self
            .chart_studio
            .document
            .level
            .entities
            .get(index)
            .and_then(|entity| entity.archetype.as_str())
            .unwrap_or_default()
            .to_owned();
        let schema = self
            .current_chart_authoring_schema()
            .and_then(|schema| schema.archetype(&archetype))
            .cloned();
        let Ok(mut entity) =
            serde_json::from_str::<crate::formats::LevelEntity>(&self.chart_studio.entity_draft)
        else {
            ui.weak("Structured field controls are unavailable until the raw entity JSON parses.");
            return;
        };
        ui.separator();
        ui.label(if schema.is_some() {
            "Schema-described fields (draft only; Apply commits)"
        } else {
            "Structured fields (JSON paths only; draft only; Apply commits)"
        });
        let mut changed = false;
        let properties = schema
            .as_ref()
            .map(|schema| schema.properties.as_slice())
            .unwrap_or_default();
        let described_paths: std::collections::BTreeSet<_> = properties
            .iter()
            .map(|property| property.path.as_str())
            .collect();
        for (property_index, property) in properties.iter().enumerate() {
            let label = property
                .label
                .as_deref()
                .unwrap_or(&property.path)
                .to_owned();
            let Some(value) =
                crate::chart_studio::json_pointer_mut(&mut entity.data, &property.path)
            else {
                ui.weak(format!("{label}: path is not present in this entity"));
                continue;
            };
            let original = value.clone();
            let mut next = original.clone();
            if !property.enum_values.is_empty() {
                if let Some(current) = original.as_str() {
                    let mut selected = current.to_owned();
                    egui::ComboBox::from_id_salt(("chart-schema-enum", index, property_index))
                        .selected_text(format!("{label}: {selected}"))
                        .show_ui(ui, |ui| {
                            for option in &property.enum_values {
                                ui.selectable_value(&mut selected, option.clone(), option);
                            }
                        });
                    if selected != current {
                        next = serde_json::Value::String(selected);
                    }
                }
            } else if let Some(current) = original.as_f64() {
                let mut edited = current;
                ui.horizontal(|ui| {
                    ui.label(&label)
                        .on_hover_text(property.description.as_deref().unwrap_or(""));
                    let mut drag = egui::DragValue::new(&mut edited).speed(0.01);
                    if property.minimum.is_some() || property.maximum.is_some() {
                        drag = drag.range(
                            property.minimum.unwrap_or(f64::MIN)
                                ..=property.maximum.unwrap_or(f64::MAX),
                        );
                    }
                    if ui.add(drag).changed() {
                        if let Some(number) = serde_json::Number::from_f64(edited) {
                            next = serde_json::Value::Number(number);
                        }
                    }
                });
            } else if let Some(current) = original.as_bool() {
                let mut edited = current;
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut edited, &label)
                        .on_hover_text(property.description.as_deref().unwrap_or(""))
                        .changed()
                    {
                        next = serde_json::Value::Bool(edited);
                    }
                });
            } else if let Some(current) = original.as_str() {
                let mut edited = current.to_owned();
                ui.horizontal(|ui| {
                    ui.label(&label)
                        .on_hover_text(property.description.as_deref().unwrap_or(""));
                    if ui.text_edit_singleline(&mut edited).changed() {
                        next = serde_json::Value::String(edited);
                    }
                });
            } else {
                ui.small(format!("{label}: edit in raw JSON"));
            }
            if next != original {
                match crate::chart_studio::set_schema_property_value(
                    &mut entity.data,
                    property,
                    next,
                ) {
                    Ok(()) => changed = true,
                    Err(error) => {
                        ui.colored_label(egui::Color32::YELLOW, format!("{label}: {error}"));
                    }
                }
            }
        }
        match crate::chart_studio::json_leaf_pointers(&entity.data) {
            Ok(paths) => {
                for path in paths {
                    if described_paths.contains(path.as_str()) {
                        continue;
                    }
                    let Some(value) =
                        crate::chart_studio::json_pointer_mut(&mut entity.data, &path)
                    else {
                        continue;
                    };
                    let label = if path.is_empty() {
                        "<data root>"
                    } else {
                        path.as_str()
                    };
                    match value {
                        serde_json::Value::Number(number) => {
                            let Some(current) = number.as_f64() else {
                                continue;
                            };
                            let integer = number.is_i64() || number.is_u64();
                            if integer && current.abs() > 9_007_199_254_740_992.0 {
                                ui.small(format!("{label}: integer exceeds the exact structured-editor range; use raw JSON to avoid precision loss"));
                                continue;
                            }
                            let mut edited = current;
                            let mut replacement = None;
                            ui.horizontal(|ui| {
                                ui.label(label).on_hover_text("Generic JSON number; no gameplay meaning or range is inferred.");
                                let speed = if integer { 1.0 } else { 0.01 };
                                if ui.add(egui::DragValue::new(&mut edited).speed(speed)).changed() {
                                    if integer && edited >= i64::MIN as f64 && edited <= i64::MAX as f64 {
                                        replacement = Some(serde_json::Number::from(edited.round() as i64));
                                    } else if let Some(number) = serde_json::Number::from_f64(edited) {
                                        replacement = Some(number);
                                    }
                                }
                            });
                            if let Some(number) = replacement {
                                *value = serde_json::Value::Number(number);
                                changed = true;
                            }
                        }
                        serde_json::Value::Bool(edited) => {
                            ui.horizontal(|ui| {
                                if ui
                                    .checkbox(edited, label)
                                    .on_hover_text(
                                        "Generic JSON boolean; no gameplay meaning is inferred.",
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                            });
                        }
                        serde_json::Value::String(edited) => {
                            ui.horizontal(|ui| {
                                ui.label(label).on_hover_text(
                                    "Generic JSON string; references are not guessed or rewritten.",
                                );
                                if ui.text_edit_singleline(edited).changed() {
                                    changed = true;
                                }
                            });
                        }
                        serde_json::Value::Null => {
                            ui.small(format!("{label}: null (edit structure in raw JSON)"));
                        }
                        serde_json::Value::Array(_) => {
                            ui.small(format!("{label}: empty array (edit structure in raw JSON)"));
                        }
                        serde_json::Value::Object(_) => {
                            ui.small(format!(
                                "{label}: empty object (edit structure in raw JSON)"
                            ));
                        }
                    }
                }
            }
            Err(error) => {
                ui.colored_label(egui::Color32::YELLOW, error.to_string());
            }
        }
        if changed {
            self.chart_studio.entity_draft =
                serde_json::to_string_pretty(&entity).unwrap_or_default();
        }
    }

    fn chart_close_prompt_ui(&mut self, ctx: &egui::Context) {
        if !self.chart_close_prompt {
            return;
        }
        egui::Window::new("Unsaved chart edits")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Save the current LevelData snapshot in a render project before closing?");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() {
                        self.chart_close_prompt = false;
                    }
                    if ui.button("Discard and close").clicked() {
                        self.chart_close_prompt = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Save and close").clicked() {
                        self.save_chart_to_project();
                        if !self.chart_studio.document.is_dirty() {
                            self.chart_close_prompt = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
            });
    }

    fn request_program_preview(&mut self) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        let time = self.preview_time.clamp(0.0, 86_400.0);
        let config = self.config.clone();
        self.preview_error = None;
        if self.preview_job.is_some() {
            self.preview_queued = Some((generation, time, config));
        } else {
            self.launch_program_preview(generation, time, config);
        }
    }

    fn launch_program_preview(&mut self, generation: u64, time: f64, config: RenderConfig) {
        let (sender, result) = mpsc::channel();
        let (preview_width, preview_height) = preview_dimensions(config.width, config.height);
        let source_config = config.clone();
        let mut worker_config = config.clone();
        worker_config.width = preview_width;
        worker_config.height = preview_height;
        let worker = std::thread::Builder::new()
            .name("sono-program-preview".into())
            .spawn(move || {
                let rendered = catch_export_panic(|| {
                    let frame = worker_config.render_frame(time)?;
                    let expected = worker_config.width as usize * worker_config.height as usize * 3;
                    if frame.rgb.len() != expected {
                        bail!(
                            "renderer preview returned {} RGB bytes; expected {expected}",
                            frame.rgb.len()
                        );
                    }
                    Ok(ProgramPreviewFrame {
                        time,
                        width: worker_config.width,
                        height: worker_config.height,
                        rgb: frame.rgb,
                        config: source_config,
                        ui_state: frame.ui_state,
                    })
                })
                .map_err(|error| format!("{error:#}"));
                let _ = sender.send(rendered);
            });
        match worker {
            Ok(worker) => {
                self.preview_job = Some(ProgramPreviewJob {
                    generation,
                    time,
                    config,
                    result,
                    worker,
                });
            }
            Err(error) => {
                self.preview_error = Some(format!("Could not start preview worker: {error}"));
            }
        }
    }

    fn poll_program_preview(&mut self) {
        let completion = self
            .preview_job
            .as_ref()
            .and_then(|job| match job.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("preview worker ended without returning a frame".into()))
                }
            });
        let Some(completion) = completion else {
            return;
        };
        let Some(job) = self.preview_job.take() else {
            return;
        };
        let _ = job.worker.join();
        let still_current = job.generation == self.preview_generation && job.config == self.config;
        if still_current {
            match completion {
                Ok(frame) => {
                    self.preview_time = frame.time;
                    self.preview_frame = Some(frame);
                    self.preview_texture = None;
                    self.preview_error = None;
                }
                Err(error) => self.preview_error = Some(error),
            }
        } else if job.generation == self.preview_generation && job.config != self.config {
            self.preview_error = Some(
                "Render settings changed while the preview was running. Render the frame again."
                    .into(),
            );
        }
        if let Some((generation, time, config)) = self.preview_queued.take() {
            if generation == self.preview_generation && config == self.config {
                self.launch_program_preview(generation, time, config);
            }
        }
    }

    fn program_preview_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Program preview");
        ui.label("Render a selected Watch frame through the production renderer and gameplay compositor. This is a still-frame monitor; sequential playback and multi-clip editing are not implemented yet.");
        ui.label("The preview captures the current configuration. Final MP4 export continues through the existing render job and queue.");
        if self.config.whole_chart {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Whole Chart export infers its end at render time. The seek slider uses the configured start and duration as a preview window.",
            );
        }

        let start = if self.config.start_time.is_finite() {
            self.config.start_time.clamp(0.0, 86_400.0 - 1.0 / 240.0)
        } else {
            0.0
        };
        let duration = if self.config.duration.is_finite() {
            self.config.duration.clamp(1.0 / 240.0, 86_400.0)
        } else {
            2.0
        };
        let end = (start + duration).clamp(start + 1.0 / 240.0, 86_400.0);
        self.preview_time = self.preview_time.clamp(start.min(end), end);
        let frame_step = 1.0 / f64::from(self.config.fps.clamp(1, 240));
        let mut render_requested = false;
        ui.horizontal(|ui| {
            if ui.button("|< Start").clicked() {
                self.preview_time = start;
                render_requested = true;
            }
            if ui.button("< Frame").clicked() {
                self.preview_time = (self.preview_time - frame_step).max(start);
                render_requested = true;
            }
            if ui.button("Frame >").clicked() {
                self.preview_time = (self.preview_time + frame_step).min(end);
                render_requested = true;
            }
            if ui.button("End >|").clicked() {
                self.preview_time = end;
                render_requested = true;
            }
            if ui
                .add_enabled(
                    self.preview_job.is_none(),
                    egui::Button::new("Render frame"),
                )
                .clicked()
            {
                render_requested = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Watch time");
            ui.add(
                egui::Slider::new(&mut self.preview_time, start..=end)
                    .step_by(frame_step)
                    .show_value(true)
                    .suffix(" s"),
            );
            ui.label(format!("{} FPS", self.config.fps));
        });
        let navigation_key = !ui.ctx().egui_wants_keyboard_input()
            && ui.ctx().input(|input| {
                input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowRight)
            });
        if navigation_key {
            ui.ctx().input(|input| {
                if input.key_pressed(egui::Key::ArrowLeft) {
                    self.preview_time = (self.preview_time - frame_step).max(start);
                } else if input.key_pressed(egui::Key::ArrowRight) {
                    self.preview_time = (self.preview_time + frame_step).min(end);
                }
            });
            render_requested = true;
        }
        if render_requested {
            self.request_program_preview();
        }

        if self.preview_job.is_some() {
            let time = self
                .preview_job
                .as_ref()
                .map(|job| job.time)
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Rendering Watch frame at {time:.3}s..."));
            });
        }
        if let Some(error) = &self.preview_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(frame) = &self.preview_frame {
            if frame.config != self.config {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Settings changed since this frame was rendered. Render the frame again to update the monitor.",
                );
            }
            ui.small(format!(
                "Frame at {:.3}s | {}x{} | captured render settings",
                frame.time, frame.width, frame.height
            ));
            ui.collapsing("Gameplay values rendered in this frame", |ui| {
                if let Some(status) = &frame.ui_state.scoring_status {
                    ui.label(status);
                }
                if !frame.ui_state.scoring_details.is_empty() {
                    ui.collapsing("Scoring details", |ui| {
                        for detail in &frame.ui_state.scoring_details {
                            ui.small(detail);
                        }
                    });
                }
                if let Some(gameplay) = &frame.ui_state.gameplay {
                    let provider = match frame.config.ui.resolved_scoring_source() {
                        crate::render_ui::UiScoringSource::SonoOverlay => "Sono-Overlay forecast",
                        crate::render_ui::UiScoringSource::EventTrack => "Event-backed state",
                        crate::render_ui::UiScoringSource::Disabled
                        | crate::render_ui::UiScoringSource::Automatic => "Gameplay state",
                    };
                    ui.label(format!(
                        "{provider}: score {:.0}, combo {}, life {:.0}.",
                        gameplay.score, gameplay.combo, gameplay.life
                    ));
                    if let Some(rating) = gameplay.variables.get("effective_level_rating") {
                        ui.small(format!("Effective level rating: {rating:.1}."));
                    }
                    if let Some(judgment) = &gameplay.judgment {
                        ui.label(format!("Latest judgment: {judgment}"));
                    } else {
                        ui.label("No judgment event has occurred at this timestamp.");
                    }
                    if let Some(event_time) = gameplay.latest_event_time {
                        ui.small(format!("Latest accepted event at Watch time {event_time:.3}s."));
                    }
                } else if frame.config.ui.resolved_scoring_source()
                    == crate::render_ui::UiScoringSource::EventTrack
                {
                    ui.label("Event-track mode has no state at this timestamp. Check the event-file status above.");
                } else if frame.config.ui.resolved_scoring_source()
                    == crate::render_ui::UiScoringSource::SonoOverlay
                {
                    ui.label("No simulated state is available at this timestamp. Check the scoring status above for unsupported chart/rating details.");
                } else {
                    ui.label("No gameplay event state was supplied. Watch rendering does not infer player score, combo, life, or judgments from chart visuals.");
                }
                ui.small("This is the actual state passed to the compositor for this still frame; it is not the sample data shown in Design Preview.");
            });
            if frame.width != self.config.width || frame.height != self.config.height {
                ui.small(format!(
                    "Preview is downscaled to fit its 1920-pixel limit; export remains {}x{}.",
                    self.config.width, self.config.height
                ));
            }
            if self.preview_texture.is_none() {
                let image = egui::ColorImage::from_rgb(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgb,
                );
                self.preview_texture = Some(ui.ctx().load_texture(
                    "sono-program-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
            if let Some(texture) = &self.preview_texture {
                let available = ui.available_size();
                let max_height = (available.y - 8.0).max(100.0);
                let scale = (available.x / frame.width as f32)
                    .min(max_height / frame.height as f32)
                    .max(0.05);
                let size = egui::vec2(frame.width as f32 * scale, frame.height as f32 * scale);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::new((texture.id(), size)));
                });
            }
        } else if self.preview_job.is_none() && self.preview_error.is_none() {
            ui.add_space(24.0);
            ui.centered_and_justified(|ui| {
                ui.label("Choose a time and render a frame to inspect the current composition.");
            });
        }
    }

    fn collect_notification_diagnostics(&mut self) {
        let diagnostics = crate::notifications::take_diagnostics();
        if diagnostics.is_empty() {
            return;
        }
        let mut live = self.live.lock().unwrap();
        for diagnostic in diagnostics {
            live.log(format!("Notification: {diagnostic}"));
        }
    }
}

fn chart_waveform_grid(
    ui: &mut egui::Ui,
    waveform: &[f32],
    duration_seconds: f64,
    bpm: f64,
    phase_seconds: f64,
    playhead_seconds: f64,
) {
    let width = ui.available_width().max(120.0);
    let desired = egui::vec2(width, 96.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(23, 27, 35));
    let center = rect.center().y;
    painter.line_segment(
        [
            egui::pos2(rect.left(), center),
            egui::pos2(rect.right(), center),
        ],
        egui::Stroke::new(1.0, egui::Color32::from_gray(58)),
    );
    if waveform.is_empty() || duration_seconds <= 0.0 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Analyze or audition an audio source to load its waveform",
            egui::FontId::proportional(13.0),
            egui::Color32::from_gray(155),
        );
        return;
    }
    let half_height = rect.height() * 0.42;
    for (index, peak) in waveform.iter().enumerate() {
        let x = rect.left() + (index as f32 + 0.5) * rect.width() / waveform.len() as f32;
        let amplitude = peak.clamp(0.0, 1.0) * half_height;
        painter.line_segment(
            [
                egui::pos2(x, center - amplitude),
                egui::pos2(x, center + amplitude),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(95, 170, 212)),
        );
    }
    if bpm.is_finite() && (20.0..=400.0).contains(&bpm) && phase_seconds.is_finite() {
        let period = 60.0 / bpm;
        let first_index = ((-phase_seconds / period).ceil()).max(0.0);
        let first_time = phase_seconds + first_index * period;
        let count = (((duration_seconds - first_time) / period).floor() + 1.0).max(0.0);
        let lod = (count / (f64::from(rect.width()) * 1.5).max(1.0))
            .ceil()
            .max(1.0) as u64;
        let total = (count as u64).min(500_000);
        let accent = egui::Color32::from_rgba_unmultiplied(248, 186, 78, 185);
        let mut beat = 0u64;
        while beat < total {
            let time = first_time + beat as f64 * period;
            let x = rect.left() + (time / duration_seconds) as f32 * rect.width();
            painter.line_segment(
                [
                    egui::pos2(x, rect.top() + 2.0),
                    egui::pos2(x, rect.bottom() - 2.0),
                ],
                egui::Stroke::new(if beat == 0 { 1.8 } else { 1.0 }, accent),
            );
            beat = beat.saturating_add(lod);
        }
    }
    if playhead_seconds.is_finite() && playhead_seconds >= 0.0 {
        let x = rect.left()
            + (playhead_seconds / duration_seconds).clamp(0.0, 1.0) as f32 * rect.width();
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.5, egui::Color32::WHITE),
        );
    }
}

fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    const MAX_DIMENSION: u32 = 1920;
    let scale = (MAX_DIMENSION as f64 / f64::from(width.max(1)))
        .min(MAX_DIMENSION as f64 / f64::from(height.max(1)))
        .min(1.0);
    (
        (f64::from(width) * scale).round().max(2.0) as u32,
        (f64::from(height) * scale).round().max(2.0) as u32,
    )
}

fn should_prompt_presentation_close(
    close_requested: bool,
    close_pending: bool,
    chart_has_unsaved_changes: bool,
    presentation_has_unsaved_changes: bool,
    presentation_is_checkpointed_for_close: bool,
) -> bool {
    close_requested
        && !close_pending
        && !chart_has_unsaved_changes
        && presentation_has_unsaved_changes
        && !presentation_is_checkpointed_for_close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_presentation_close_requires_explicit_resolution() {
        assert!(should_prompt_presentation_close(
            true, false, false, true, false
        ));
        assert!(!should_prompt_presentation_close(
            true, false, false, false, false
        ));
        assert!(!should_prompt_presentation_close(
            false, false, false, true, false
        ));
        assert!(!should_prompt_presentation_close(
            true, true, false, true, false
        ));
        assert!(!should_prompt_presentation_close(
            true, false, true, true, false
        ));
        assert!(!should_prompt_presentation_close(
            true, false, false, true, true
        ));
    }

    #[test]
    fn composition_snapshot_save_validates_and_atomically_writes_png() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frame.png");
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([20, 40, 60, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();

        save_composition_preview_png(&path, &bytes).unwrap();
        let decoded = image::load_from_memory_with_format(
            &std::fs::read(&path).unwrap(),
            image::ImageFormat::Png,
        )
        .unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
        assert!(save_composition_preview_png(&path, b"not a PNG").is_err());
        assert_eq!(
            image::load_from_memory(&std::fs::read(&path).unwrap())
                .unwrap()
                .width(),
            2,
            "invalid data must not replace the previous valid frame"
        );
    }

    #[test]
    fn program_preview_caps_resolution_without_changing_aspect_ratio() {
        assert_eq!(preview_dimensions(1920, 1080), (1920, 1080));
        assert_eq!(preview_dimensions(3840, 2160), (1920, 1080));
        assert_eq!(preview_dimensions(2160, 3840), (1080, 1920));
        assert_eq!(preview_dimensions(8192, 8192), (1920, 1920));
    }

    #[test]
    fn timeline_snap_toggle_selects_frame_snapped_or_free_time() {
        let project = crate::composition::CompositionProject::new();
        let raw = 1.234;
        let snapped = snap_timeline_time(&project, raw, None, 0.0, 45.0, true);
        let free = snap_timeline_time(&project, raw, None, 0.0, 45.0, false);
        assert!((snapped - 1.2333333333333334).abs() < 1e-9);
        assert_eq!(free, raw);
    }

    #[test]
    fn presentation_media_drop_targets_tracks_and_keeps_av_pairs_linked_atomically() {
        let mut project = crate::composition::CompositionProject::new();
        project.sources.push(crate::composition::MediaSource {
            id: "camera".into(),
            display_name: "Camera take".into(),
            has_video: true,
            has_audio: true,
            width: Some(640),
            height: Some(360),
            duration: 4.0,
            ..Default::default()
        });
        project.sources.push(crate::composition::MediaSource {
            id: "voice".into(),
            display_name: "Voice take".into(),
            has_audio: true,
            duration: 4.0,
            ..Default::default()
        });
        let video_track = project
            .tracks
            .iter()
            .find(|track| track.kind == crate::composition::TrackKind::Visual)
            .unwrap()
            .id
            .clone();
        let audio_track = project
            .tracks
            .iter()
            .find(|track| track.kind == crate::composition::TrackKind::Audio)
            .unwrap()
            .id
            .clone();

        let video_id =
            add_media_source_to_timeline_track(&mut project, "camera", &video_track, 2.0).unwrap();
        let video = project
            .clips
            .iter()
            .find(|clip| clip.id == video_id)
            .unwrap();
        let link_group = video.link_group_id.as_deref().unwrap();
        assert_eq!(video.track_id.as_deref(), Some(video_track.as_str()));
        let linked_audio = project
            .clips
            .iter()
            .find(|clip| clip.link_group_id.as_deref() == Some(link_group) && clip.id != video_id)
            .unwrap();
        assert_eq!(linked_audio.kind, crate::composition::ClipKind::Audio);
        assert_eq!(linked_audio.track_id.as_deref(), Some(audio_track.as_str()));
        assert_eq!(linked_audio.timeline_start, 2.0);

        let voice_id =
            add_media_source_to_timeline_track(&mut project, "voice", &audio_track, 5.0).unwrap();
        assert_eq!(
            project
                .clips
                .iter()
                .find(|clip| clip.id == voice_id)
                .unwrap()
                .track_id
                .as_deref(),
            Some(audio_track.as_str())
        );

        let before_invalid_drop = project.clone();
        assert!(
            add_media_source_to_timeline_track(&mut project, "voice", &video_track, 6.0).is_err()
        );
        assert_eq!(project, before_invalid_drop);
        project
            .tracks
            .iter_mut()
            .find(|track| track.id == video_track)
            .unwrap()
            .locked = true;
        let before_locked_drop = project.clone();
        assert!(
            add_media_source_to_timeline_track(&mut project, "camera", &video_track, 6.0).is_err()
        );
        assert_eq!(project, before_locked_drop);
    }

    #[test]
    fn presentation_insert_group_undo_redo_roundtrips_one_authoritative_project() {
        let mut project = crate::composition::CompositionProject::new();
        let first_source = project.add_solid_color_source("C02020").unwrap();
        let first_clip = project
            .add_clip(&first_source, crate::composition::ClipKind::Video, 0.0)
            .unwrap();
        let before_insert = project.clone();
        let second_source = project.add_solid_color_source("2040C0").unwrap();
        let second_clip = project
            .add_clip(&second_source, crate::composition::ClipKind::Video, 1.0)
            .unwrap();

        let mut history = CompositionEditHistory::default();
        history.remember(before_insert);
        assert!(history.undo(&mut project));
        assert!(project
            .sources
            .iter()
            .any(|source| source.id == second_source));
        assert!(!project.clips.iter().any(|clip| clip.id == second_clip));
        assert!(history.redo(&mut project));
        assert!(project.clips.iter().any(|clip| clip.id == second_clip));

        let before_group = project.clone();
        history.remember(before_group);
        let group_id = project
            .group_clips(&[first_clip.clone(), second_clip.clone()])
            .unwrap();
        assert!(project
            .clips
            .iter()
            .filter(|clip| clip.id == first_clip || clip.id == second_clip)
            .all(|clip| clip.group_id.as_deref() == Some(group_id.as_str())));
        assert!(history.undo(&mut project));
        assert!(project
            .clips
            .iter()
            .all(|clip| clip.group_id.as_deref() != Some(group_id.as_str())));
        assert!(history.redo(&mut project));
        assert!(project
            .clips
            .iter()
            .filter(|clip| clip.id == first_clip || clip.id == second_clip)
            .all(|clip| clip.group_id.as_deref() == Some(group_id.as_str())));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("group-undo-redo.sonocomp");
        project.save(&path).unwrap();
        let reopened = crate::composition::CompositionProject::load(&path).unwrap();
        assert_eq!(reopened, project);
    }

    #[test]
    fn beginner_validation_explains_common_missing_inputs() {
        assert!(friendly_beginner_error("engine input is missing")
            .to_lowercase()
            .contains("engine zip"));
        assert!(friendly_beginner_error("level input is missing")
            .to_lowercase()
            .contains("level or chart"));
        assert!(
            friendly_beginner_error("BGM is enabled but no music is present")
                .contains("Music / BGM")
        );
        assert!(friendly_beginner_error("FFmpeg could not be launched")
            .contains("Video encoding could not use FFmpeg"));
    }

    #[test]
    fn presentation_thumbnail_identity_changes_when_source_file_is_relinked() {
        let temp = tempfile::tempdir().unwrap();
        let first_path = temp.path().join("first.mp4");
        let second_path = temp.path().join("second.mp4");
        std::fs::write(&first_path, b"first source identity").unwrap();
        std::fs::write(&second_path, b"replacement source identity").unwrap();
        let mut source = crate::composition::MediaSource {
            id: "source-id".into(),
            path: first_path,
            has_video: true,
            ..Default::default()
        };
        let first_key = presentation_thumbnail_key(&source).unwrap();
        source.path = second_path;
        let replacement_key = presentation_thumbnail_key(&source).unwrap();
        assert_ne!(first_key, replacement_key);
        source.path = temp.path().join("missing.mp4");
        assert!(presentation_thumbnail_key(&source).is_none());
    }

    #[test]
    fn settings_search_finds_existing_sections_for_common_terms() {
        for (query, expected_tab) in [
            ("gcc", 2),
            ("watermark", 4),
            ("discord", 5),
            ("life", 3),
            ("fps", 2),
            ("profile", 0),
        ] {
            assert!(
                settings_search_results(query)
                    .iter()
                    .any(|(_, tab)| *tab == expected_tab),
                "no result for {query}"
            );
        }
        assert!(settings_search_results("not a setting").is_empty());
    }

    #[test]
    fn historical_input_repair_changes_only_the_requested_role() {
        let mut config = RenderConfig::default();
        config.engine = PathBuf::from("missing-engine.zip");
        config.level = PathBuf::from("original-level.json.gz");
        let replacement = PathBuf::from("replacement-engine.zip");
        replace_history_input(&mut config, "engine", replacement.clone()).unwrap();
        assert_eq!(config.engine, replacement);
        assert_eq!(config.level, PathBuf::from("original-level.json.gz"));
        assert!(replace_history_input(&mut config, "unknown", PathBuf::from("x")).is_err());
    }

    #[test]
    fn desktop_first_run_preserves_existing_render_defaults() {
        let app = Desktop::default();
        assert!(
            app.welcome_visible,
            "missing preferences should show first-run guidance"
        );
        assert_eq!(
            (app.config.width, app.config.height, app.config.fps),
            (1920, 1080, 60)
        );
        assert_eq!(app.config.duration, 20.0);
        assert_eq!(app.config.start_time, 0.0);
        assert_eq!(app.config.backend, RenderBackend::Wgpu);
        assert_eq!(app.config.layers, crate::render::RenderLayers::default());
        assert!(!app.config.compile_sono_gcc_on_demand);
        assert!(app.active_profile_id.is_none());
        assert_eq!(app.config.ui.profile_name, "Asset-backed Gameplay HUD");
        assert_eq!(app.config.ui.fonts["combo"].center_spacing, Some(108.0));
        assert_eq!(
            app.config.ui.fonts["combo-burst"].center_spacing,
            Some(108.0)
        );
        assert!(app
            .config
            .ui
            .elements
            .iter()
            .any(|element| element.id == "combo-event-pulse"));
        assert!(app
            .config
            .ui
            .elements
            .iter()
            .any(|element| element.id == "combo-event-pop"));
        let export_config = Desktop::gui_export_config(&app.config);
        assert_eq!(export_config.ui, app.config.ui);
    }

    #[test]
    fn desktop_skip_persists_and_hides_first_run_home() {
        let mut app = Desktop::default();
        assert!(app.welcome_visible);

        app.skip_welcome();

        assert!(!app.welcome_visible);
        let persisted =
            crate::onboarding::OnboardingPreferences::load(&app.onboarding_path).unwrap();
        assert!(persisted.welcome_skipped);
        assert!(!persisted.should_show_welcome());
    }

    #[test]
    fn dropped_composition_opens_from_first_run_without_sonolus_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("standalone.sonocomp");
        crate::composition::CompositionProject::new()
            .save(&path)
            .unwrap();
        let mut app = Desktop::default();

        app.process_dropped_files(vec![path.clone()]);

        assert!(!app.welcome_visible);
        assert_eq!(app.tab, 15);
        assert_eq!(
            app.presentation_studio.project_path.as_deref(),
            Some(path.as_path())
        );
        assert!(app.presentation_studio.project.sources.is_empty());
    }

    #[test]
    fn dropped_render_project_from_presentation_routes_to_render_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("render.sonoproject");
        crate::project::RenderProject::from_config(&RenderConfig::default(), None)
            .save(&path)
            .unwrap();
        let mut app = Desktop::default();
        app.tab = 15;

        app.process_dropped_files(vec![path.clone()]);

        assert_eq!(app.tab, 0);
        assert_eq!(app.project_path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn gui_applies_saved_profile_to_existing_settings_and_queue_defaults() {
        let mut app = Desktop::default();
        let manager = app.profile_manager.as_ref().unwrap();
        let mut config = RenderConfig::default();
        config.width = 1280;
        config.height = 720;
        config.job_manager.max_concurrent_jobs = 3;
        let saved = manager.create("GUI profile", config.clone()).unwrap();

        app.adopt_config_profile(saved.clone(), "Profile applied".into());

        assert_eq!(app.active_profile_id.as_deref(), Some(saved.id.as_str()));
        assert_eq!(app.profile_name_input, "GUI profile");
        assert_eq!(app.config, config);
        assert_eq!(
            app.job_manager.as_ref().unwrap().config(),
            config.job_manager
        );
        assert!(!app.error);
    }

    #[test]
    fn cancelled_picker_preserves_spaced_unicode_path() {
        let mut p = PathBuf::from("C:/my files/\u{8c31}\u{9762}.data");
        let old = p.clone();
        apply_selection(&mut p, None);
        assert_eq!(p, old);
        apply_selection(&mut p, Some(PathBuf::from("other.scp")));
        assert_eq!(p, PathBuf::from("other.scp"));
    }
    #[test]
    fn arbitrary_override_is_preserved_and_default_removes_override() {
        let mut app = Desktop::default();
        set_option(&mut app.config, 73, -2.25, 0.0);
        assert_eq!(app.config.level_options[0].value, -2.25);
        set_option(&mut app.config, 73, 0.0, 0.0);
        assert!(app.config.level_options.is_empty());
    }
    #[test]
    fn progress_is_typed_and_logs_are_bounded() {
        let mut state = LiveState::default();
        state.progress(ExportProgress {
            phase: crate::export_progress::ExportPhase::Rendering,
            completed_frames: 8,
            total_frames: Some(100),
            percentage: Some(8.0),
            elapsed_seconds: 2.0,
            frames_per_second: Some(4.0),
            eta_seconds: Some(23.0),
        });
        assert_eq!(state.progress.as_ref().unwrap().completed_frames, 8);
        for i in 0..300 {
            state.log(i.to_string());
        }
        assert_eq!(state.logs.len(), 200);
    }
    #[test]
    fn background_gcc_only_starts_when_selected() {
        assert!(!should_precompile(
            crate::sono_gcc::WatchExecutionMode::Interpreter
        ));
        assert!(should_precompile(
            crate::sono_gcc::WatchExecutionMode::SonoGcc
        ));
    }
    #[test]
    fn gui_export_defers_compile_to_background_warmup() {
        let mut app = Desktop::default();
        app.config.execution_mode = crate::sono_gcc::WatchExecutionMode::SonoGcc;
        assert!(!app.config.compile_sono_gcc_on_demand);
        let export_config = Desktop::gui_export_config(&app.config);
        assert!(!export_config.compile_sono_gcc_on_demand);
        app.config.compile_sono_gcc_on_demand = true;
        assert!(Desktop::gui_export_config(&app.config).compile_sono_gcc_on_demand);
    }
    #[test]
    fn gui_and_export_share_persisted_notification_settings() {
        let mut app = Desktop::default();
        app.config.notifications.discord.enabled = true;
        app.config.notifications.discord.bot_token_env = "MY_RENDERER_BOT_SECRET".into();
        app.config.notifications.discord.destinations.push(
            crate::notifications::DiscordDestination {
                guild_id: Some("123456789012345670".into()),
                channel_id: "123456789012345678".into(),
                events: vec![crate::notifications::RendererEventKind::RenderFailed],
                ..Default::default()
            },
        );
        let shared = Desktop::gui_export_config(&app.config);
        assert_eq!(shared.notifications, app.config.notifications);
        assert_eq!(
            shared.notifications.discord.destinations[0].channel_id,
            "123456789012345678"
        );
        let serialized = serde_json::to_string(&shared).unwrap();
        assert!(serialized.contains("MY_RENDERER_BOT_SECRET"));
        assert!(!serialized.contains("Authorization: Bot"));
    }
    #[test]
    fn engine_switch_and_vm_mode_discard_stale_gcc_progress() {
        let engine_a = PathBuf::from("engine-a.zip");
        let engine_b = PathBuf::from("engine-b.zip");
        assert!(!gcc_engine_message_applies(
            &engine_a,
            &engine_b,
            crate::sono_gcc::WatchExecutionMode::SonoGcc
        ));
        assert!(!gcc_engine_message_applies(
            &engine_a,
            &engine_a,
            crate::sono_gcc::WatchExecutionMode::Interpreter
        ));
        assert!(gcc_engine_message_applies(
            &engine_a,
            &engine_a,
            crate::sono_gcc::WatchExecutionMode::SonoGcc
        ));
    }
    #[test]
    fn precompile_report_distinguishes_partial_fallback_from_ready() {
        let report =
            |compiled_operations, eligible_operations| crate::sono_gcc::SonoGccPrecompileReport {
                cache_key: "graph-key".into(),
                cache_hit: crate::sono_gcc::CacheHit::None,
                timings: crate::sono_gcc::CompileTimings::default(),
                graph_nodes: 20,
                compiled_regions: 4,
                compiled_operations,
                eligible_operations,
                cache_write_error: None,
            };
        assert!(matches!(
            status_for_gcc_report(report(8, 10)),
            GccUiStatus::Partial(_)
        ));
        assert!(matches!(
            status_for_gcc_report(report(10, 10)),
            GccUiStatus::Ready(_)
        ));
    }
    #[test]
    fn running_job_blocks_duplicate_start_and_reports_error_or_cancel() {
        for cancelled in [false, true] {
            let mut app = Desktop::default();
            let (tx, rx) = mpsc::sync_channel(1);
            let control = ExportControl::default();
            app.job = Some(Job {
                control: control.clone(),
                result: rx,
                worker: std::thread::spawn(|| {}),
                config_snapshot: app.config.clone(),
            });
            let summary = app.summary.clone();
            app.start(&egui::Context::default());
            assert_eq!(app.summary, summary);
            assert!(app.job.is_some());
            if cancelled {
                control.cancel();
                tx.send(Err(Cancelled.into())).unwrap();
            } else {
                tx.send(Err(
                    anyhow::anyhow!("FFmpeg failed").context("GUI export during Rendering")
                ))
                .unwrap();
            }
            app.poll();
            assert!(app.job.is_none());
            assert_eq!(app.live.lock().unwrap().logs.len(), 1);
            assert_eq!(app.error, !cancelled);
            assert!(app.summary.contains(if cancelled {
                "Cancelled"
            } else {
                "FFmpeg failed"
            }));
        }
    }

    #[test]
    fn localized_options_use_supplied_english_metadata() {
        assert_eq!(
            option_label(r##"##LOCALIZE:{"en":"Standard","ja":"other"}"##),
            "Standard"
        );
        assert_eq!(option_label("#NOTE_SPEED"), "#NOTE_SPEED");
        assert_eq!(option_label("##LOCALIZE:invalid"), "##LOCALIZE:invalid");
    }

    #[test]
    fn obvious_invalid_input_is_reported() {
        let app = Desktop::default();
        assert!(obvious_validation(&app.config)
            .unwrap_err()
            .to_string()
            .contains("engine ZIP"));
    }

    #[test]
    fn workspace_commands_share_navigation_and_reset_the_active_layout() {
        let mut app = Desktop::default();
        let ctx = egui::Context::default();
        assert!(app.dispatch_command(crate::workspace::CommandId::GoPresentationStudio, &ctx));
        assert_eq!(app.tab, 15);
        assert!(!app.quick_render_active);
        app.workspace_preferences.panels.media = false;
        app.presentation_studio.timeline_zoom = 132.0;
        assert!(app.dispatch_command(crate::workspace::CommandId::ResetWorkspace, &ctx));
        assert_eq!(
            app.workspace_preferences.panels,
            crate::workspace::PanelVisibility::default()
        );
        assert_eq!(app.presentation_studio.timeline_zoom, 45.0);
        assert!(app.dispatch_command(crate::workspace::CommandId::GoChartStudio, &ctx));
        assert_eq!(app.tab, 14);
        assert!(app.dispatch_command(crate::workspace::CommandId::GoVoiceoverWorkspace, &ctx));
        assert_eq!(
            app.workspace_preferences.active,
            crate::workspace::WorkspaceId::Voiceover
        );
        assert_eq!(app.tab, 15);
        assert!(app.workspace_preferences.panels.voiceover);
    }

    #[test]
    fn switching_workspaces_preserves_the_editing_layout() {
        let mut app = Desktop::default();
        app.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);
        app.activate_workspace_layout(crate::workspace::WorkspaceId::Editing);
        app.workspace_preferences.panels.media = false;
        app.presentation_studio.timeline_zoom = 128.0;
        app.navigate_workflow(crate::workspace::WorkflowId::ChartStudio);
        assert_eq!(
            app.workspace_preferences.workflow,
            crate::workspace::WorkflowId::ChartStudio
        );
        assert_eq!(
            app.workspace_preferences.active,
            crate::workspace::WorkspaceId::Editing
        );
        app.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);
        assert!(!app.workspace_preferences.panels.media);
        assert_eq!(app.presentation_studio.timeline_zoom, 128.0);
    }

    #[test]
    fn workflow_switching_keeps_open_documents_and_quick_render_inputs() {
        let mut app = Desktop::default();
        app.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);
        app.presentation_studio.dirty = true;
        app.presentation_studio.project.name = "Unsaved composition".into();
        app.config.width = 1280;
        app.navigate_workflow(crate::workspace::WorkflowId::ChartStudio);
        app.navigate_workflow(crate::workspace::WorkflowId::QuickRender);
        app.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);

        assert!(app.presentation_studio.dirty);
        assert_eq!(app.presentation_studio.project.name, "Unsaved composition");
        assert_eq!(app.config.width, 1280);
        assert_eq!(
            app.workspace_preferences.workflow,
            crate::workspace::WorkflowId::PresentationStudio
        );
    }

    #[test]
    fn page_navigation_uses_registered_commands_and_keeps_layout_separate() {
        let mut app = Desktop::default();
        let ctx = egui::Context::default();
        app.navigate_workflow(crate::workspace::WorkflowId::PresentationStudio);
        app.workspace_preferences.panels.media = false;
        let editing_layout = app.workspace_preferences.active;

        assert!(app.dispatch_command(crate::workspace::CommandId::GoPerformanceLab, &ctx));
        assert_eq!(app.tab, 9);
        assert_eq!(
            app.workspace_preferences.workflow,
            crate::workspace::WorkflowId::Tools
        );
        assert!(app.dispatch_command(crate::workspace::CommandId::GoRenderHistory, &ctx));
        assert_eq!(app.tab, 7);
        assert_eq!(
            app.workspace_preferences.workflow,
            crate::workspace::WorkflowId::Production
        );
        assert!(app.dispatch_command(crate::workspace::CommandId::GoRenderSettings, &ctx));
        assert_eq!(app.tab, 2);
        assert_eq!(
            app.workspace_preferences.workflow,
            crate::workspace::WorkflowId::Settings
        );
        assert_eq!(app.workspace_preferences.active, editing_layout);
        assert!(!app.workspace_preferences.panels.media);
    }

    #[test]
    fn contextual_palette_commands_do_not_run_without_their_document_context() {
        let mut app = Desktop::default();
        let ctx = egui::Context::default();
        app.tab = 15;
        let initial_clips = app.presentation_studio.project.clips.len();
        assert!(!app.dispatch_command(crate::workspace::CommandId::SplitClip, &ctx));
        assert_eq!(app.presentation_studio.project.clips.len(), initial_clips);
        assert!(app.summary.contains("Select a timeline clip"));
        app.tab = 14;
        app.chart_studio.music_path = PathBuf::new();
        assert!(!app.dispatch_command(crate::workspace::CommandId::AnalyzeTempo, &ctx));
        assert!(app.chart_studio.tempo_job.is_none());
        let render = crate::workspace::command_by_stable_id("render.start").unwrap();
        assert_eq!(app.command_disabled_reason(render), None);
    }

    #[test]
    fn acquire_media_command_enters_quick_render_and_requests_the_url_panel() {
        let mut app = Desktop::default();
        let ctx = egui::Context::default();
        assert!(app.dispatch_command(crate::workspace::CommandId::OpenMediaAcquisition, &ctx));
        assert_eq!(app.tab, 0);
        assert!(app.quick_render_active);
        assert!(app.media_acquisition_requested);
    }

    #[test]
    fn new_render_project_button_action_uses_the_registered_command() {
        let mut app = Desktop::default();
        let ctx = egui::Context::default();
        assert!(app.dispatch_command(crate::workspace::CommandId::NewRenderProject, &ctx));
        assert!(app.project.is_some());
        assert!(app.project_path.is_none());
    }

    #[test]
    fn split_command_enablement_requires_a_valid_playhead_position() {
        let app_clip = {
            let mut clip = crate::composition::CompositionClip::default();
            clip.id = "clip-1".into();
            clip.duration = 1.0;
            clip
        };
        let mut app = Desktop::default();
        app.quick_render_active = false;
        app.tab = 15;
        app.presentation_studio.project.clips.push(app_clip);
        app.presentation_studio.selected_clip = Some("clip-1".into());
        app.presentation_studio.playhead = 0.5;
        let command = crate::workspace::command_by_stable_id("timeline.split").unwrap();
        assert_eq!(app.command_disabled_reason(command), None);
        app.presentation_studio.playhead = 0.0;
        assert!(app
            .command_disabled_reason(command)
            .is_some_and(|reason| reason.contains("inside the selected clip")));
    }

    #[test]
    fn worker_panic_preserves_payload_origin_and_allows_recovery() {
        let error = std::thread::spawn(|| {
            catch_export_panic::<()>(|| panic!("diagnostic sentinel"))
                .unwrap_err()
                .to_string()
        })
        .join()
        .unwrap();
        assert!(error.contains("diagnostic sentinel"));
        assert!(error.contains(file!()));
        assert_eq!(catch_export_panic(|| Ok(42)).unwrap(), 42);
        let non_string = catch_export_panic::<()>(|| std::panic::panic_any(7u32)).unwrap_err();
        assert!(non_string.to_string().contains("non-string panic payload"));
    }
}
