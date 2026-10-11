//! Application command vocabulary, workflow routing, and user-owned UI layouts.
//!
//! Commands identify operations independently of their menu text or shortcut.
//! UI entry points dispatch these IDs; document mutations still use the
//! document's existing undo history.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const FORMAT: &str = "sono-renderer-workspace-preferences";
const SCHEMA_VERSION: u32 = 3;

/// The user's current task surface. This is navigation state, separate from
/// `WorkspaceId`, which stores panel/layout preferences for an editing view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowId {
    Home,
    QuickRender,
    ChartStudio,
    PresentationStudio,
    Production,
    Tools,
    Settings,
    Diagnostics,
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::Home
    }
}

impl WorkflowId {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::QuickRender => "Quick Render",
            Self::ChartStudio => "Chart Studio",
            Self::PresentationStudio => "Presentation Studio",
            Self::Production => "Jobs & History",
            Self::Tools => "Tools",
            Self::Settings => "Settings",
            Self::Diagnostics => "Diagnostics",
        }
    }

    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Home => "nav.home",
            Self::QuickRender => "nav.quick_render",
            Self::ChartStudio => "nav.chart_studio",
            Self::PresentationStudio => "nav.presentation_studio",
            Self::Production => "nav.production",
            Self::Tools => "nav.tools",
            Self::Settings => "nav.settings",
            Self::Diagnostics => "nav.diagnostics",
        }
    }

    pub fn localized_label(self, locale: &str) -> String {
        crate::localization::tr(locale, self.localization_key())
    }

    fn from_legacy_workspace(workspace: WorkspaceId) -> Self {
        match workspace {
            WorkspaceId::QuickRender => Self::QuickRender,
            WorkspaceId::Charting => Self::ChartStudio,
            WorkspaceId::Editing | WorkspaceId::Voiceover => Self::PresentationStudio,
            WorkspaceId::Advanced => Self::Diagnostics,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandId {
    ShowCommandPalette,
    OpenShortcutSettings,
    GoHome,
    GoQuickRender,
    GoChartStudio,
    GoPresentationStudio,
    GoVoiceoverWorkspace,
    GoProduction,
    GoRenderQueue,
    GoRenderHistory,
    GoTools,
    GoCacheManager,
    GoPerformanceLab,
    GoRegressionLab,
    GoCompatibility,
    GoProgramPreview,
    GoSettings,
    GoConfigurationSettings,
    GoEngineOptionsSettings,
    GoRenderSettings,
    GoGameplayUiSettings,
    GoWatermarkSettings,
    GoNotificationSettings,
    GoAdvancedDiagnostics,
    OpenMediaAcquisition,
    NewRenderProject,
    OpenRenderProject,
    SaveRenderProject,
    SaveRenderProjectAs,
    NewComposition,
    OpenComposition,
    SaveComposition,
    SaveCompositionAs,
    ExportComposition,
    Undo,
    Redo,
    CopyClip,
    PasteClip,
    DuplicateClip,
    DeleteClip,
    SplitClip,
    RippleDeleteClip,
    TogglePlayback,
    PreviousFrame,
    NextFrame,
    ToggleTimelineSnapping,
    AddKeyframe,
    GenerateSubtitles,
    RecordVoiceover,
    StopRecording,
    AnalyzeTempo,
    ExportLevelData,
    StartRender,
    ResetWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandContext {
    Global,
    Presentation,
    PresentationSelection,
    PresentationAudioSelection,
    Chart,
    ChartWithMusic,
    Render,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub id: CommandId,
    pub stable_id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub keywords: &'static str,
    pub default_shortcut: Option<&'static str>,
    pub context: CommandContext,
}

pub const COMMANDS: &[CommandSpec] = &[
    command(
        CommandId::ShowCommandPalette,
        "app.command_palette",
        "Open Command Palette",
        "Application",
        "Search and run an application command.",
        "search commands actions",
        Some("Ctrl+Shift+P"),
        CommandContext::Global,
    ),
    command(
        CommandId::OpenShortcutSettings,
        "app.shortcut_settings",
        "Keyboard Shortcuts",
        "Application",
        "Search and customize command shortcuts.",
        "bindings keys preferences",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoHome,
        "app.home",
        "Home",
        "Application",
        "Choose a workflow or reopen recent work.",
        "welcome start recent home",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoQuickRender,
        "workspace.quick_render",
        "Render a Level",
        "Workflow",
        "Open the direct engine + level render workflow.",
        "render sonolus engine level mp4",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoChartStudio,
        "workspace.chart_studio",
        "Create or Edit a Chart",
        "Workflow",
        "Open Chart Studio.",
        "chart leveldata bpm tempo authoring",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoPresentationStudio,
        "workspace.presentation_studio",
        "Edit a Video",
        "Workflow",
        "Open the standalone Presentation Studio editor.",
        "video edit nle timeline",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoVoiceoverWorkspace,
        "workspace.voiceover",
        "Voiceover Layout",
        "Layout",
        "Open Presentation Studio with recording controls and timeline emphasized.",
        "microphone narration audio panels timeline",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoProduction,
        "app.production",
        "Jobs and History",
        "Application",
        "Review queued renders, completed outputs, and render history.",
        "queue jobs outputs history",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoRenderQueue,
        "app.production.queue",
        "Render Queue",
        "Production",
        "Show queued and active render jobs.",
        "jobs queue running cancel output",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoRenderHistory,
        "app.production.history",
        "Render History",
        "Production",
        "Show completed, failed, and cancelled render attempts.",
        "history rerender previous output",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoTools,
        "app.tools",
        "Tools",
        "Application",
        "Open analysis and specialized renderer tools.",
        "analysis tempo bpm compatibility cache regression preview",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoCacheManager,
        "app.tools.cache",
        "Cache Manager",
        "Tools",
        "Inspect and safely manage generated renderer caches.",
        "cache disk cleanup storage",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoPerformanceLab,
        "app.tools.performance",
        "Performance Lab",
        "Tools",
        "Run or inspect renderer performance measurements.",
        "benchmark timings telemetry",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoRegressionLab,
        "app.tools.regression",
        "Regression Lab",
        "Tools",
        "Compare real-renderer visual, audio, semantic, or timing results.",
        "visual audio compare regression",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoCompatibility,
        "app.tools.compatibility",
        "Compatibility Analysis",
        "Tools",
        "Inspect engine, resource, and level compatibility findings.",
        "engine level resources preflight",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoProgramPreview,
        "app.tools.program_preview",
        "Program Preview",
        "Tools",
        "Render selected Watch times through the production renderer.",
        "still frame watch preview seek",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoSettings,
        "app.settings",
        "Settings",
        "Application",
        "Configure renderer defaults, profiles, notifications, and gameplay presentation.",
        "preferences configuration profile watermark notifications",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoConfigurationSettings,
        "app.settings.configuration",
        "Configuration and Profiles",
        "Settings",
        "Open saved renderer configuration and reusable profile settings.",
        "profile config file defaults",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoEngineOptionsSettings,
        "app.settings.engine_options",
        "Engine Options",
        "Settings",
        "Configure the selected engine's renderer options.",
        "watch options engine",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoRenderSettings,
        "app.settings.render",
        "Render Settings",
        "Settings",
        "Configure render range, output dimensions, frame rate, and backend defaults.",
        "fps resolution output backend",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoGameplayUiSettings,
        "app.settings.gameplay_ui",
        "Gameplay UI Settings",
        "Settings",
        "Configure gameplay presentation profiles used by renderer output.",
        "hud score combo life profile",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoWatermarkSettings,
        "app.settings.watermark",
        "Watermark Settings",
        "Settings",
        "Configure the renderer's fixed and moving watermark behavior.",
        "attribution mark",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoNotificationSettings,
        "app.settings.notifications",
        "Notification Settings",
        "Settings",
        "Configure desktop and Discord render notifications.",
        "desktop discord alerts",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::GoAdvancedDiagnostics,
        "workspace.advanced",
        "Diagnostics",
        "Application",
        "Open render diagnostics.",
        "advanced performance logs compatibility",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::OpenMediaAcquisition,
        "render.acquire_media_from_url",
        "Acquire Media From URL",
        "Quick Render",
        "Open the Quick Render URL acquisition controls; acquired files become render inputs.",
        "url download yt-dlp import media",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::NewRenderProject,
        "project.new",
        "New Render Project",
        "Project",
        "Create a reusable render-input project from the current settings.",
        "engine level inputs project",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::OpenRenderProject,
        "project.open",
        "Open Render Project",
        "Project",
        "Open a saved render-input project.",
        "recent project inputs",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::SaveRenderProject,
        "project.save",
        "Save Render Project",
        "Project",
        "Save the current render-input project.",
        "inputs document",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::SaveRenderProjectAs,
        "project.save_as",
        "Save Render Project As",
        "Project",
        "Save the current render-input project to a new path.",
        "inputs document copy",
        None,
        CommandContext::Global,
    ),
    command(
        CommandId::NewComposition,
        "presentation.new",
        "New Composition",
        "Presentation Studio",
        "Create a new standalone video composition.",
        "project video",
        None,
        CommandContext::Presentation,
    ),
    command(
        CommandId::OpenComposition,
        "presentation.open",
        "Open Composition",
        "Presentation Studio",
        "Open a saved video composition.",
        "project video",
        None,
        CommandContext::Presentation,
    ),
    command(
        CommandId::SaveComposition,
        "presentation.save",
        "Save Composition",
        "Presentation Studio",
        "Save the current composition.",
        "project document",
        Some("Ctrl+S"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::SaveCompositionAs,
        "presentation.save_as",
        "Save Composition As",
        "Presentation Studio",
        "Save the composition to a new path.",
        "project document",
        Some("Ctrl+Shift+S"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::ExportComposition,
        "presentation.export",
        "Export Composition",
        "Presentation Studio",
        "Export the current video composition.",
        "render video mp4",
        Some("Ctrl+E"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::Undo,
        "edit.undo",
        "Undo",
        "Edit",
        "Undo the last edit in the active document.",
        "history",
        Some("Ctrl+Z"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::Redo,
        "edit.redo",
        "Redo",
        "Edit",
        "Redo the last undone edit.",
        "history",
        Some("Ctrl+Shift+Z"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::CopyClip,
        "timeline.copy",
        "Copy Clip",
        "Timeline",
        "Copy the selected timeline clip.",
        "clipboard",
        Some("Ctrl+C"),
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::PasteClip,
        "timeline.paste",
        "Paste Clip",
        "Timeline",
        "Paste a copied clip into the composition.",
        "clipboard",
        Some("Ctrl+V"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::DuplicateClip,
        "timeline.duplicate",
        "Duplicate Clip",
        "Timeline",
        "Duplicate the selected clip.",
        "copy",
        Some("Ctrl+D"),
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::DeleteClip,
        "timeline.delete",
        "Delete Clip",
        "Timeline",
        "Delete the selected clip.",
        "remove",
        Some("Delete"),
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::SplitClip,
        "timeline.split",
        "Split Clip at Playhead",
        "Timeline",
        "Split the selected clip at the playhead.",
        "cut blade",
        Some("S"),
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::RippleDeleteClip,
        "timeline.ripple_delete",
        "Ripple Delete Clip",
        "Timeline",
        "Delete the selected clip and close its timeline gap.",
        "remove gap",
        None,
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::TogglePlayback,
        "presentation.toggle_playback",
        "Play / Pause",
        "Presentation Studio",
        "Toggle composition playback.",
        "space preview",
        Some("Space"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::PreviousFrame,
        "timeline.previous_frame",
        "Previous Frame",
        "Timeline",
        "Move the playhead back one frame.",
        "step left",
        Some("ArrowLeft"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::NextFrame,
        "timeline.next_frame",
        "Next Frame",
        "Timeline",
        "Move the playhead forward one frame.",
        "step right",
        Some("ArrowRight"),
        CommandContext::Presentation,
    ),
    command(
        CommandId::ToggleTimelineSnapping,
        "timeline.toggle_snapping",
        "Toggle Timeline Snapping",
        "Timeline",
        "Toggle frame/edge snapping while editing.",
        "magnet snap",
        None,
        CommandContext::Presentation,
    ),
    command(
        CommandId::AddKeyframe,
        "timeline.add_keyframe",
        "Add Keyframe",
        "Timeline",
        "Add a keyframe for the selected animated property at the playhead.",
        "animation motion",
        None,
        CommandContext::PresentationSelection,
    ),
    command(
        CommandId::GenerateSubtitles,
        "presentation.generate_subtitles",
        "Generate Subtitles",
        "Presentation Studio",
        "Transcribe the selected audio/video clip into editable captions.",
        "caption transcription speech",
        None,
        CommandContext::PresentationAudioSelection,
    ),
    command(
        CommandId::RecordVoiceover,
        "presentation.record_voiceover",
        "Record Voiceover",
        "Presentation Studio",
        "Start microphone recording from the timeline playhead.",
        "microphone narration audio",
        None,
        CommandContext::Presentation,
    ),
    command(
        CommandId::StopRecording,
        "presentation.stop_recording",
        "Stop Recording",
        "Presentation Studio",
        "Stop and finalize the active microphone take.",
        "microphone voiceover",
        None,
        CommandContext::Presentation,
    ),
    command(
        CommandId::AnalyzeTempo,
        "chart.analyze_tempo",
        "Analyze Tempo",
        "Chart Studio",
        "Analyze the selected chart audio for a beat-grid proposal.",
        "bpm beat music",
        None,
        CommandContext::ChartWithMusic,
    ),
    command(
        CommandId::ExportLevelData,
        "chart.export_level",
        "Export LevelData",
        "Chart Studio",
        "Export and validate the current LevelData document.",
        "chart level",
        None,
        CommandContext::Chart,
    ),
    command(
        CommandId::StartRender,
        "render.start",
        "Render MP4",
        "Quick Render",
        "Queue a render using the current direct render inputs.",
        "engine level output",
        None,
        CommandContext::Render,
    ),
    command(
        CommandId::ResetWorkspace,
        "workspace.reset",
        "Reset Current Workspace",
        "Workspace",
        "Restore the selected built-in workspace layout.",
        "layout panels restore",
        None,
        CommandContext::Global,
    ),
];

/// Kept for compatibility with an existing editor shortcut. A user override
/// replaces the complete binding set for that command.
pub const ALTERNATE_DEFAULT_SHORTCUTS: &[(&str, &str)] = &[("edit.redo", "Ctrl+Y")];

const fn command(
    id: CommandId,
    stable_id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
    keywords: &'static str,
    default_shortcut: Option<&'static str>,
    context: CommandContext,
) -> CommandSpec {
    CommandSpec {
        id,
        stable_id,
        name,
        category,
        description,
        keywords,
        default_shortcut,
        context,
    }
}

pub fn command_by_stable_id(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|command| command.stable_id == id)
}

/// Small subsequence search: forgiving for partial names without allocating
/// or adding a fuzzy-search dependency to the GUI.
pub fn search_commands(query: &str) -> Vec<&'static CommandSpec> {
    search_commands_localized(query, "en")
}

pub fn localized_command_name(command: &CommandSpec, locale: &str) -> String {
    let key = format!("command.{}", command.stable_id);
    if crate::localization::contains_message(locale, &key) {
        crate::localization::tr(locale, &key)
    } else {
        command.name.to_owned()
    }
}

pub fn localized_command_description(command: &CommandSpec, locale: &str) -> String {
    let key = format!("command.{}.description", command.stable_id);
    if crate::localization::contains_message(locale, &key) {
        crate::localization::tr(locale, &key)
    } else {
        command.description.to_owned()
    }
}

pub fn search_commands_localized(query: &str, locale: &str) -> Vec<&'static CommandSpec> {
    let query = normalize(query);
    let mut matches: Vec<_> = COMMANDS
        .iter()
        .filter_map(|command| {
            let localized_name = localized_command_name(command, locale);
            let name = normalize(&localized_name);
            let canonical_name = normalize(command.name);
            let category = normalize(command.category);
            let id = normalize(command.stable_id);
            let keywords = normalize(command.keywords);
            let score = if query.is_empty() {
                Some(0)
            } else if name == query {
                Some(0)
            } else if name.starts_with(&query) {
                Some(1)
            } else if name.contains(&query)
                || canonical_name.contains(&query)
                || category.contains(&query)
                || id.contains(&query)
            {
                Some(2)
            } else if keywords
                .split_whitespace()
                .any(|word| word.contains(&query))
            {
                Some(3)
            } else if subsequence(
                &query,
                &format!("{name} {canonical_name} {category} {id} {keywords}"),
            ) {
                Some(4)
            } else {
                None
            }?;
            Some((score, localized_name, command))
        })
        .collect();
    matches.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    matches.into_iter().map(|(_, _, command)| command).collect()
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut needle = needle.chars();
    let Some(mut wanted) = needle.next() else {
        return true;
    };
    for character in haystack.chars() {
        if character == wanted {
            let Some(next) = needle.next() else {
                return true;
            };
            wanted = next;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceId {
    /// Retained to migrate workspace selections from pre-workflow UI versions.
    QuickRender,
    /// Retained to migrate workspace selections from pre-workflow UI versions.
    Charting,
    /// Presentation Studio panel arrangement.
    Editing,
    /// Presentation Studio panel arrangement emphasizing voiceover controls.
    Voiceover,
    /// Retained to migrate workspace selections from pre-workflow UI versions.
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceLayout {
    pub panels: PanelVisibility,
    pub timeline_zoom: u16,
    pub timeline_track_height: u16,
}

impl WorkspaceLayout {
    pub fn defaults(workspace: WorkspaceId) -> Self {
        Self {
            panels: PanelVisibility::for_workspace(workspace),
            timeline_zoom: if workspace == WorkspaceId::Voiceover {
                60
            } else {
                45
            },
            timeline_track_height: if workspace == WorkspaceId::Voiceover {
                64
            } else {
                52
            },
        }
    }

    fn clamp(&mut self) {
        self.timeline_zoom = self.timeline_zoom.clamp(10, 180);
        self.timeline_track_height = self.timeline_track_height.clamp(24, 120);
    }
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self::defaults(WorkspaceId::Editing)
    }
}

impl WorkspaceId {
    pub const ALL: [Self; 5] = [
        Self::QuickRender,
        Self::Charting,
        Self::Editing,
        Self::Voiceover,
        Self::Advanced,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::QuickRender => "Quick Render",
            Self::Charting => "Charting",
            Self::Editing => "Video Editing",
            Self::Voiceover => "Voiceover",
            Self::Advanced => "Advanced / Diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelVisibility {
    pub composition: bool,
    pub tracks: bool,
    pub media: bool,
    pub source_monitor: bool,
    pub voiceover: bool,
    pub captions: bool,
    pub timeline: bool,
    pub inspector: bool,
}

impl Default for PanelVisibility {
    fn default() -> Self {
        Self {
            composition: true,
            tracks: true,
            media: true,
            source_monitor: true,
            voiceover: true,
            captions: true,
            timeline: true,
            inspector: true,
        }
    }
}

impl PanelVisibility {
    pub fn for_workspace(workspace: WorkspaceId) -> Self {
        match workspace {
            WorkspaceId::QuickRender | WorkspaceId::Charting | WorkspaceId::Advanced => {
                Self::default()
            }
            WorkspaceId::Editing => Self::default(),
            WorkspaceId::Voiceover => Self {
                composition: false,
                tracks: true,
                media: true,
                source_monitor: true,
                voiceover: true,
                captions: true,
                timeline: true,
                inspector: true,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    pub name: String,
    pub base: WorkspaceId,
    pub panels: PanelVisibility,
    pub timeline_zoom: u16,
    pub timeline_track_height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspacePreferences {
    format: String,
    schema_version: u32,
    #[serde(default)]
    pub workflow: WorkflowId,
    /// Application interface locale. This is presentation preference state;
    /// it is not copied into render jobs, projects, or reproducibility data.
    #[serde(default = "default_locale")]
    pub locale: String,
    pub active: WorkspaceId,
    pub panels: PanelVisibility,
    pub timeline_zoom: u16,
    pub timeline_track_height: u16,
    /// A remembered layout for each built-in workspace. The top-level layout
    /// fields remain the active workspace snapshot for schema compatibility.
    pub layouts: BTreeMap<WorkspaceId, WorkspaceLayout>,
    /// Stable command IDs map to one user binding. Removed command IDs are
    /// pruned during load so they cannot create dead or ambiguous bindings.
    pub shortcut_overrides: BTreeMap<String, ShortcutBinding>,
    pub saved: Vec<SavedWorkspace>,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            schema_version: SCHEMA_VERSION,
            workflow: WorkflowId::Home,
            locale: default_locale(),
            active: WorkspaceId::Editing,
            panels: PanelVisibility::default(),
            timeline_zoom: 45,
            timeline_track_height: 52,
            layouts: BTreeMap::new(),
            shortcut_overrides: BTreeMap::new(),
            saved: Vec::new(),
        }
    }
}

impl WorkspacePreferences {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading workspace preferences {}", path.display()))
            }
        };
        let mut preferences: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing workspace preferences {}", path.display()))?;
        if preferences.format != FORMAT {
            bail!(
                "{} is not Sono-Renderer workspace preferences",
                path.display()
            );
        }
        if preferences.schema_version > SCHEMA_VERSION {
            bail!(
                "workspace preferences use unsupported schema version {}",
                preferences.schema_version
            );
        }
        if preferences.schema_version == 0 {
            bail!("workspace preference schema version must be positive");
        }
        if preferences.schema_version < 2 {
            preferences.workflow = WorkflowId::from_legacy_workspace(preferences.active);
        }
        if !crate::localization::is_valid_preference(&preferences.locale, cfg!(debug_assertions)) {
            preferences.locale = default_locale();
        }
        if preferences.saved.len() > 64 {
            preferences.saved.truncate(64);
        }
        preferences.saved.retain(|saved| {
            let name = saved.name.trim();
            !name.is_empty() && name.len() <= 80 && !name.chars().any(char::is_control)
        });
        preferences.timeline_zoom = preferences.timeline_zoom.clamp(10, 180);
        preferences.timeline_track_height = preferences.timeline_track_height.clamp(24, 120);
        for layout in preferences.layouts.values_mut() {
            layout.clamp();
        }
        preferences
            .layouts
            .entry(preferences.active)
            .or_insert(WorkspaceLayout {
                panels: preferences.panels,
                timeline_zoom: preferences.timeline_zoom,
                timeline_track_height: preferences.timeline_track_height,
            });
        if let Some(layout) = preferences.layouts.get(&preferences.active) {
            preferences.panels = layout.panels;
            preferences.timeline_zoom = layout.timeline_zoom;
            preferences.timeline_track_height = layout.timeline_track_height;
        }
        for saved in &mut preferences.saved {
            saved.timeline_zoom = saved.timeline_zoom.clamp(10, 180);
            saved.timeline_track_height = saved.timeline_track_height.clamp(24, 120);
        }
        preferences
            .shortcut_overrides
            .retain(|id, binding| command_by_stable_id(id).is_some() && binding.validate().is_ok());
        preferences.schema_version = SCHEMA_VERSION;
        preferences.format = FORMAT.into();
        Ok(preferences)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if self.format != FORMAT || self.schema_version != SCHEMA_VERSION {
            bail!("cannot save workspace preferences with unsupported format or schema");
        }
        if !crate::localization::is_valid_preference(&self.locale, cfg!(debug_assertions)) {
            bail!("workspace preferences contain an unsupported application locale");
        }
        if self.saved.len() > 64 {
            bail!("at most 64 custom workspaces may be saved");
        }
        if !(10..=180).contains(&self.timeline_zoom)
            || !(24..=120).contains(&self.timeline_track_height)
        {
            bail!("active workspace layout values are outside supported bounds");
        }
        for (workspace, layout) in &self.layouts {
            let mut bounded = *layout;
            bounded.clamp();
            if bounded != *layout {
                bail!(
                    "layout values for {} are outside supported bounds",
                    workspace.label()
                );
            }
        }
        for saved in &self.saved {
            let name = saved.name.trim();
            if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
                bail!("workspace name must contain 1–80 printable characters");
            }
            if !(10..=180).contains(&saved.timeline_zoom)
                || !(24..=120).contains(&saved.timeline_track_height)
            {
                bail!("saved workspace layout values are outside supported bounds");
            }
        }
        for (id, binding) in &self.shortcut_overrides {
            if command_by_stable_id(id).is_none() {
                bail!("unknown command ID in shortcut preferences: {id}");
            }
            binding.validate()?;
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let bytes = serde_json::to_vec_pretty(self).context("serializing workspace preferences")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).with_context(|| {
            format!(
                "creating temporary workspace preference file in {}",
                parent.display()
            )
        })?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("saving workspace preferences {}", path.display()))?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }

    pub fn binding_for(&self, command: &CommandSpec) -> Option<ShortcutBinding> {
        self.shortcut_overrides
            .get(command.stable_id)
            .cloned()
            .or_else(|| {
                command
                    .default_shortcut
                    .and_then(|shortcut| ShortcutBinding::parse(shortcut).ok())
            })
    }

    pub fn bindings_for(&self, command: &CommandSpec) -> Vec<ShortcutBinding> {
        if let Some(binding) = self.shortcut_overrides.get(command.stable_id) {
            return (!binding.is_disabled())
                .then(|| binding.clone())
                .into_iter()
                .collect();
        }
        let mut bindings = command
            .default_shortcut
            .and_then(|shortcut| ShortcutBinding::parse(shortcut).ok())
            .into_iter()
            .collect::<Vec<_>>();
        bindings.extend(
            ALTERNATE_DEFAULT_SHORTCUTS
                .iter()
                .filter(|(id, _)| *id == command.stable_id)
                .filter_map(|(_, shortcut)| ShortcutBinding::parse(shortcut).ok()),
        );
        bindings
    }

    pub fn conflicts(&self, command_id: &str, binding: &ShortcutBinding) -> Vec<&'static str> {
        if binding.is_disabled() {
            return Vec::new();
        }
        COMMANDS
            .iter()
            .filter(|candidate| {
                candidate.stable_id != command_id && self.bindings_for(candidate).contains(&binding)
            })
            .map(|candidate| candidate.stable_id)
            .collect()
    }

    pub fn assign_binding(
        &mut self,
        command_id: &str,
        binding: ShortcutBinding,
        replace: bool,
    ) -> Result<Vec<&'static str>> {
        if command_by_stable_id(command_id).is_none() {
            bail!("unknown command ID: {command_id}");
        }
        binding.validate()?;
        let conflicts = self.conflicts(command_id, &binding);
        if !conflicts.is_empty() && !replace {
            return Ok(conflicts);
        }
        if replace {
            for conflict in &conflicts {
                self.shortcut_overrides.remove(*conflict);
            }
            for conflict in &conflicts {
                self.shortcut_overrides
                    .insert((*conflict).into(), ShortcutBinding::DISABLED);
            }
        }
        self.shortcut_overrides.insert(command_id.into(), binding);
        Ok(conflicts)
    }

    pub fn reset_shortcuts(&mut self) {
        self.shortcut_overrides.clear();
    }

    pub fn capture_active_layout(&mut self) {
        self.layouts.insert(
            self.active,
            WorkspaceLayout {
                panels: self.panels,
                timeline_zoom: self.timeline_zoom,
                timeline_track_height: self.timeline_track_height,
            },
        );
    }

    pub fn activate(&mut self, workspace: WorkspaceId) {
        self.capture_active_layout();
        self.active = workspace;
        let layout = self
            .layouts
            .get(&workspace)
            .copied()
            .unwrap_or_else(|| WorkspaceLayout::defaults(workspace));
        self.panels = layout.panels;
        self.timeline_zoom = layout.timeline_zoom;
        self.timeline_track_height = layout.timeline_track_height;
    }

    pub fn reset_active_layout(&mut self) {
        let layout = WorkspaceLayout::defaults(self.active);
        self.panels = layout.panels;
        self.timeline_zoom = layout.timeline_zoom;
        self.timeline_track_height = layout.timeline_track_height;
        self.layouts.insert(self.active, layout);
    }

    pub fn save_custom(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
            bail!("workspace name must contain 1–80 printable characters");
        }
        if self.saved.len() >= 64
            && !self
                .saved
                .iter()
                .any(|saved| saved.name.eq_ignore_ascii_case(name))
        {
            bail!("the custom workspace limit (64) has been reached");
        }
        if !(10..=180).contains(&self.timeline_zoom)
            || !(24..=120).contains(&self.timeline_track_height)
        {
            bail!("workspace layout values are outside supported bounds");
        }
        self.capture_active_layout();
        self.saved
            .retain(|saved| !saved.name.eq_ignore_ascii_case(name));
        self.saved.push(SavedWorkspace {
            name: name.into(),
            base: self.active,
            panels: self.panels,
            timeline_zoom: self.timeline_zoom,
            timeline_track_height: self.timeline_track_height,
        });
        Ok(())
    }

    pub fn apply_saved(&mut self, name: &str) -> bool {
        if let Some(saved) = self.saved.iter().find(|saved| saved.name == name) {
            self.active = saved.base;
            self.panels = saved.panels;
            self.timeline_zoom = saved.timeline_zoom;
            self.timeline_track_height = saved.timeline_track_height;
            self.capture_active_layout();
            true
        } else {
            false
        }
    }
}

fn default_locale() -> String {
    "en".into()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutBinding {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}

impl ShortcutBinding {
    pub const DISABLED: Self = Self {
        primary: false,
        shift: false,
        alt: false,
        key: String::new(),
    };

    pub fn parse(text: &str) -> Result<Self> {
        let mut binding = Self::DISABLED;
        let mut parts = text
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                binding.key = canonical_key(part)
                    .ok_or_else(|| anyhow::anyhow!("unsupported shortcut key {part:?}"))?;
            } else {
                match part.to_ascii_lowercase().as_str() {
                    "ctrl" | "cmd" | "command" | "primary" => binding.primary = true,
                    "shift" => binding.shift = true,
                    "alt" | "option" => binding.alt = true,
                    _ => bail!("unsupported shortcut modifier {part:?}"),
                }
            }
        }
        Ok(binding)
    }

    pub fn validate(&self) -> Result<()> {
        if self.key.is_empty() {
            if *self == Self::DISABLED {
                return Ok(());
            }
            bail!("shortcut key is empty");
        }
        if canonical_key(&self.key).as_deref() != Some(self.key.as_str()) {
            bail!("unsupported shortcut key {:?}", self.key);
        }
        if !self.primary && !self.alt && self.key.len() == 1 && !self.shift {
            bail!("single-letter shortcuts require Ctrl/Cmd or Alt to protect text fields");
        }
        Ok(())
    }

    pub fn is_disabled(&self) -> bool {
        self.key.is_empty()
    }

    pub fn label(&self) -> String {
        if self.is_disabled() {
            return "Unbound".into();
        }
        let mut parts = Vec::new();
        if self.primary {
            #[cfg(target_os = "macos")]
            parts.push("⌘".to_owned());
            #[cfg(not(target_os = "macos"))]
            parts.push("Ctrl".to_owned());
        }
        if self.shift {
            parts.push("Shift".into());
        }
        if self.alt {
            parts.push("Alt".into());
        }
        parts.push(self.key.clone());
        parts.join("+")
    }
}

fn canonical_key(key: &str) -> Option<String> {
    let normalized = match key.to_ascii_lowercase().as_str() {
        "space" => "Space".to_owned(),
        "delete" | "del" => "Delete".to_owned(),
        "backspace" => "Backspace".to_owned(),
        "enter" | "return" => "Enter".to_owned(),
        "escape" | "esc" => "Escape".to_owned(),
        "tab" => "Tab".to_owned(),
        "arrowleft" | "left" => "ArrowLeft".to_owned(),
        "arrowright" | "right" => "ArrowRight".to_owned(),
        "arrowup" | "up" => "ArrowUp".to_owned(),
        "arrowdown" | "down" => "ArrowDown".to_owned(),
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
            value.to_ascii_uppercase()
        }
        value
            if value.len() == 2
                && value.starts_with('f')
                && value[1..]
                    .parse::<u8>()
                    .is_ok_and(|number| (1..=12).contains(&number)) =>
        {
            value.to_ascii_uppercase()
        }
        _ => return None,
    };
    Some(normalized)
}

pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SONO_RENDERER_WORKSPACE_PREFERENCES") {
        return PathBuf::from(path);
    }
    crate::store_paths::config_root().join("workspace_preferences.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_search_supports_exact_partial_keywords_and_fuzzy_names() {
        assert_eq!(
            search_commands("timeline.split")[0].id,
            CommandId::SplitClip
        );
        assert_eq!(
            search_commands("subtitle")[0].id,
            CommandId::GenerateSubtitles
        );
        assert_eq!(search_commands("bpm")[0].id, CommandId::AnalyzeTempo);
        assert_eq!(
            search_commands("url")[0].id,
            CommandId::OpenMediaAcquisition
        );
        assert!(search_commands("spclp")
            .iter()
            .any(|command| command.id == CommandId::SplitClip));
        assert_eq!(
            search_commands("performance lab")[0].id,
            CommandId::GoPerformanceLab
        );
        assert_eq!(
            search_commands("render history")[0].id,
            CommandId::GoRenderHistory
        );
    }

    #[test]
    fn shortcut_rebinding_detects_conflicts_and_replaces_without_ambiguity() {
        let mut preferences = WorkspacePreferences::default();
        let binding = ShortcutBinding::parse("Ctrl+S").unwrap();
        let conflict = preferences
            .assign_binding("presentation.export", binding.clone(), false)
            .unwrap();
        assert_eq!(conflict, vec!["presentation.save"]);
        preferences
            .assign_binding("presentation.export", binding.clone(), true)
            .unwrap();
        assert_eq!(
            preferences.binding_for(command_by_stable_id("presentation.export").unwrap()),
            Some(binding.clone())
        );
        assert_ne!(
            preferences.binding_for(command_by_stable_id("presentation.save").unwrap()),
            Some(binding)
        );
    }

    #[test]
    fn rejects_unmodified_single_letter_shortcuts_and_unknown_commands() {
        assert!(ShortcutBinding::parse("S").unwrap().validate().is_err());
        let mut preferences = WorkspacePreferences::default();
        assert!(preferences
            .assign_binding(
                "exec.shell",
                ShortcutBinding::parse("Ctrl+Q").unwrap(),
                false
            )
            .is_err());
    }

    #[test]
    fn workspace_preferences_round_trip_and_recover_defaults_for_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let mut preferences = WorkspacePreferences::load(&path).unwrap();
        preferences.workflow = WorkflowId::Tools;
        preferences.active = WorkspaceId::Voiceover;
        preferences.panels.voiceover = true;
        preferences.save_custom("Narration").unwrap();
        preferences
            .assign_binding(
                "timeline.split",
                ShortcutBinding::parse("Alt+X").unwrap(),
                false,
            )
            .unwrap();
        preferences.save(&path).unwrap();
        let loaded = WorkspacePreferences::load(&path).unwrap();
        assert_eq!(loaded, preferences);
        assert_eq!(loaded.workflow, WorkflowId::Tools);
        assert_eq!(
            WorkspacePreferences::load(&directory.path().join("missing.json")).unwrap(),
            WorkspacePreferences::default()
        );
    }

    #[test]
    fn locale_preference_migrates_persists_and_rejects_untrusted_ids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let mut old = serde_json::to_value(WorkspacePreferences::default()).unwrap();
        old.as_object_mut().unwrap().remove("locale");
        old["schema_version"] = serde_json::json!(2);
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let migrated = WorkspacePreferences::load(&path).unwrap();
        assert_eq!(migrated.locale, "en");
        assert_eq!(migrated.schema_version, SCHEMA_VERSION);

        let mut selected = migrated;
        selected.locale = "ja".into();
        selected.save(&path).unwrap();
        assert_eq!(WorkspacePreferences::load(&path).unwrap().locale, "ja");

        let mut tampered =
            serde_json::to_value(WorkspacePreferences::load(&path).unwrap()).unwrap();
        tampered["locale"] = serde_json::json!("../../run-this");
        fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
        assert_eq!(WorkspacePreferences::load(&path).unwrap().locale, "en");
    }

    #[test]
    fn localized_command_search_keeps_canonical_ids_and_accepts_unicode_queries() {
        let command = command_by_stable_id("workspace.chart_studio").unwrap();
        assert_eq!(command.name, "Create or Edit a Chart");
        assert_eq!(localized_command_name(command, "ja"), "譜面を作成・編集");
        assert!(search_commands_localized("譜面", "ja")
            .iter()
            .any(|candidate| candidate.id == CommandId::GoChartStudio));
        assert!(search_commands_localized("workspace.chart_studio", "ja")
            .iter()
            .any(|candidate| candidate.id == CommandId::GoChartStudio));
    }

    #[test]
    fn custom_workspaces_validate_names_and_restore_panel_visibility() {
        let mut preferences = WorkspacePreferences::default();
        preferences.panels.media = false;
        preferences.save_custom("Compact Edit").unwrap();
        preferences.panels.media = true;
        assert!(preferences.apply_saved("Compact Edit"));
        assert!(!preferences.panels.media);
        assert!(preferences.save_custom("\n").is_err());
    }

    #[test]
    fn each_builtin_workspace_remembers_its_own_layout_and_reset_restores_defaults() {
        let mut preferences = WorkspacePreferences::default();
        preferences.activate(WorkspaceId::Editing);
        preferences.panels.media = false;
        preferences.timeline_zoom = 130;
        preferences.activate(WorkspaceId::Charting);
        assert!(preferences.panels.media);
        preferences.activate(WorkspaceId::Editing);
        assert!(!preferences.panels.media);
        assert_eq!(preferences.timeline_zoom, 130);
        preferences.reset_active_layout();
        assert!(preferences.panels.media);
        assert_eq!(preferences.timeline_zoom, 45);
    }

    #[test]
    fn older_layout_snapshot_migrates_and_future_schema_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.json");
        let mut old = serde_json::to_value(WorkspacePreferences::default()).unwrap();
        old.as_object_mut().unwrap().remove("layouts");
        old.as_object_mut().unwrap().remove("workflow");
        old["schema_version"] = serde_json::json!(1);
        old["active"] = serde_json::json!("editing");
        old["panels"]["media"] = serde_json::json!(false);
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let loaded = WorkspacePreferences::load(&path).unwrap();
        assert!(!loaded.layouts[&WorkspaceId::Editing].panels.media);
        assert_eq!(loaded.workflow, WorkflowId::PresentationStudio);
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);

        old["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(WorkspacePreferences::load(&path)
            .unwrap_err()
            .to_string()
            .contains("unsupported schema version"));
    }
}
