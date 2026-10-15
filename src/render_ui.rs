//! Renderer-owned, deliberately generic HUD overlays.
//!
//! This module does not reproduce Sonolus Runtime UI behavior. Values are
//! supplied by explicit providers and drawn with a small built-in bitmap font.

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const UI_PROFILE_FORMAT: &str = "sono-renderer-gameplay-ui-profile";
const UI_PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GameplayUiMode {
    #[default]
    Custom,
    Legacy,
    Off,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiFitMode {
    #[default]
    Contain,
    Cover,
    Stretch,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiAnchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiFlowDirection {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiLayout {
    /// Position and size are fractions of the parent content box.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub anchor: UiAnchor,
    pub rotation: f64,
    pub scale: [f64; 2],
    /// Left, top, right, bottom fractions of the parent box.
    pub margin: [f64; 4],
    /// Left, top, right, bottom fractions reserved for children.
    pub padding: [f64; 4],
    pub flow: Option<UiFlow>,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            anchor: UiAnchor::TopLeft,
            rotation: 0.0,
            scale: [1.0, 1.0],
            margin: [0.0; 4],
            padding: [0.0; 4],
            flow: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiFlow {
    pub direction: UiFlowDirection,
    pub gap: f64,
}

impl Default for UiFlow {
    fn default() -> Self {
        Self {
            direction: UiFlowDirection::Row,
            gap: 0.0,
        }
    }
}

/// Safe expression tree used by text, gauges, and visibility rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiExpression {
    Constant {
        value: f64,
    },
    Source {
        source: UiStateSource,
    },
    Variable {
        name: String,
    },
    Add {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Subtract {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Multiply {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Divide {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Min {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Max {
        left: Box<UiExpression>,
        right: Box<UiExpression>,
    },
    Clamp {
        value: Box<UiExpression>,
        minimum: f64,
        maximum: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiStateSource {
    Time,
    DeltaTime,
    Progress,
    RuntimeEntities,
    ActiveEntities,
    CallbackCount,
    DrawCount,
    ScheduledEventCount,
    Score,
    /// Sono-Overlay's rank-bar progress for simulated gameplay state.
    ScoreProgress,
    Combo,
    Life,
    /// Timeline timestamp of the latest accepted gameplay event.
    LatestEventTime,
    /// Timeline timestamp of the most recent state event with a judgement.
    JudgmentTime,
    GameplayVariable {
        name: String,
    },
    /// Renderer-authored numeric value evaluated from the profile timeline.
    /// It is separate from the immutable Watch/runtime value sources above.
    PresentationValue {
        name: String,
    },
    Memory {
        block_id: i64,
        index: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiCondition {
    Truthy {
        value: UiExpression,
    },
    Greater {
        left: UiExpression,
        right: UiExpression,
    },
    GreaterOrEqual {
        left: UiExpression,
        right: UiExpression,
    },
    Less {
        left: UiExpression,
        right: UiExpression,
    },
    LessOrEqual {
        left: UiExpression,
        right: UiExpression,
    },
    Equal {
        left: UiExpression,
        right: UiExpression,
    },
    NotEqual {
        left: UiExpression,
        right: UiExpression,
    },
    StringEquals {
        source: UiStringSource,
        value: String,
    },
    All {
        conditions: Vec<UiCondition>,
    },
    Any {
        conditions: Vec<UiCondition>,
    },
    Not {
        condition: Box<UiCondition>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiStringSource {
    Event,
    NoteType,
    Judgment,
    PresentationCategory { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UiAnimationProperty {
    Opacity,
    OffsetX,
    OffsetY,
    ScaleX,
    ScaleY,
    Rotation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UiEasing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Cubic Bezier control points, quantized to 1/10000 for stable JSON
    /// equality and compact profile diffs. x coordinates must be monotonic.
    CubicBezier {
        x1: u16,
        y1: u16,
        x2: u16,
        y2: u16,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiTimeline {
    pub tracks: Vec<UiTimelineTrack>,
}

impl Default for UiTimeline {
    fn default() -> Self {
        Self { tracks: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiTimelineTrack {
    pub id: String,
    #[serde(flatten)]
    pub kind: UiTimelineTrackKind,
}

impl Default for UiTimelineTrack {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: UiTimelineTrackKind::Numeric {
                keyframes: Vec::new(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "track", rename_all = "kebab-case")]
pub enum UiTimelineTrackKind {
    Numeric {
        keyframes: Vec<UiNumericKeyframe>,
    },
    Categorical {
        keyframes: Vec<UiCategoricalKeyframe>,
    },
}

impl Default for UiTimelineTrackKind {
    fn default() -> Self {
        Self::Numeric {
            keyframes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiNumericKeyframe {
    pub time: f64,
    pub value: f64,
    /// Easing on this keyframe controls the segment leading to the next key.
    pub easing: UiEasing,
}

impl Default for UiNumericKeyframe {
    fn default() -> Self {
        Self {
            time: 0.0,
            value: 0.0,
            easing: UiEasing::Linear,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiCategoricalKeyframe {
    pub time: f64,
    pub value: String,
}

impl Default for UiCategoricalKeyframe {
    fn default() -> Self {
        Self {
            time: 0.0,
            value: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiAnimation {
    pub property: UiAnimationProperty,
    pub from: f64,
    pub to: f64,
    pub start: f64,
    /// Optional timeline source for event-anchored animations.
    pub start_from: Option<UiStateSource>,
    pub duration: f64,
    pub repeat: bool,
    pub easing: UiEasing,
    pub when: Option<UiCondition>,
    /// Optional deterministic state value. This lets domain adapters expose
    /// timestamp-derived animation curves without adding a second animator.
    pub value_from_state: Option<UiExpression>,
}

impl Default for UiAnimation {
    fn default() -> Self {
        Self {
            property: UiAnimationProperty::Opacity,
            from: 0.0,
            to: 1.0,
            start: 0.0,
            start_from: None,
            duration: 1.0,
            repeat: false,
            easing: UiEasing::Linear,
            when: None,
            value_from_state: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiElementKind {
    Group,
    Container,
    Overlay {
        color: [u8; 4],
    },
    Rectangle {
        color: [u8; 4],
        #[serde(default)]
        border: Option<[u8; 4]>,
        #[serde(default)]
        radius: f64,
    },
    Text {
        text: String,
        #[serde(default)]
        value: Option<UiExpression>,
        #[serde(default = "default_ui_text_color")]
        color: [u8; 4],
        #[serde(default = "default_ui_font_size")]
        font_size: f64,
        #[serde(default)]
        font: Option<String>,
        #[serde(default)]
        align: UiTextAlign,
    },
    Counter {
        value: UiExpression,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
        #[serde(default)]
        digits: usize,
        /// Optional right-side digit limit, matching displays such as the
        /// Sono-Overlay combo's configurable `last_digit` setting.
        #[serde(default)]
        max_digits: Option<usize>,
        #[serde(default = "default_ui_text_color")]
        color: [u8; 4],
        #[serde(default = "default_ui_font_size")]
        font_size: f64,
        #[serde(default)]
        font: Option<String>,
        #[serde(default)]
        align: UiTextAlign,
    },
    Gauge {
        value: UiExpression,
        #[serde(default = "default_one_expression")]
        maximum: UiExpression,
        #[serde(default)]
        orientation: UiGaugeOrientation,
        #[serde(default = "default_ui_accent")]
        fill: [u8; 4],
        #[serde(default = "default_ui_track")]
        track: [u8; 4],
        /// Optional profile asset used for the filled region instead of a flat color.
        #[serde(default)]
        fill_asset: Option<String>,
        /// Normalized source rectangle inside `fill_asset`; useful for layered HUD textures.
        #[serde(default)]
        fill_source: Option<[f64; 4]>,
        /// Optional alpha mask in the same source coordinate space as `fill_asset`.
        #[serde(default)]
        mask_asset: Option<String>,
    },
    ProgressBar {
        value: UiExpression,
        #[serde(default = "default_one_expression")]
        maximum: UiExpression,
        #[serde(default)]
        orientation: UiGaugeOrientation,
        #[serde(default = "default_ui_accent")]
        fill: [u8; 4],
        #[serde(default = "default_ui_track")]
        track: [u8; 4],
        #[serde(default)]
        fill_asset: Option<String>,
        #[serde(default)]
        fill_source: Option<[f64; 4]>,
        #[serde(default)]
        mask_asset: Option<String>,
    },
    Image {
        asset: String,
        #[serde(default)]
        tint: Option<[u8; 4]>,
    },
    Sprite {
        asset: String,
        source: [u32; 4],
        #[serde(default)]
        tint: Option<[u8; 4]>,
    },
    Icon {
        asset: String,
        #[serde(default)]
        tint: Option<[u8; 4]>,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiGaugeOrientation {
    #[default]
    LeftToRight,
    RightToLeft,
    BottomToTop,
    TopToBottom,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiTextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Blend operation for a UI element's output. This currently keeps the
/// normal alpha path and the additive path used by the Sono-Overlay combo pop.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UiBlendMode {
    #[default]
    Normal,
    Add,
}

/// Ordered state-driven overrides. Later matching rules replace earlier values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiStyleRule {
    pub when: UiCondition,
    #[serde(default)]
    pub color: Option<[u8; 4]>,
    #[serde(default)]
    pub border: Option<[u8; 4]>,
    #[serde(default)]
    pub tint: Option<[u8; 4]>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub opacity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiElement {
    pub id: String,
    pub enabled: bool,
    pub z_index: i32,
    pub opacity: f64,
    /// Optional compositing mode; normal alpha blending remains the default.
    #[serde(default)]
    pub blend_mode: UiBlendMode,
    /// Elements sharing a group are rendered into one transparent surface
    /// before that surface is composited onto the frame. This preserves local
    /// blend semantics such as Sono-Overlay's additive combo pop.
    #[serde(default)]
    pub isolation_group: Option<String>,
    pub layout: UiLayout,
    /// Optional reference-layout group used when a profile supplies responsive
    /// group positions (for example, distinct Sono-Overlay 16:9 and 4:3 layouts).
    #[serde(default)]
    pub responsive_group: Option<String>,
    pub visible_when: Option<UiCondition>,
    pub animations: Vec<UiAnimation>,
    pub style_rules: Vec<UiStyleRule>,
    #[serde(flatten)]
    pub kind: UiElementKind,
    pub children: Vec<UiElement>,
}

impl Default for UiElement {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: true,
            z_index: 0,
            opacity: 1.0,
            blend_mode: UiBlendMode::Normal,
            isolation_group: None,
            layout: UiLayout::default(),
            responsive_group: None,
            visible_when: None,
            animations: Vec::new(),
            style_rules: Vec::new(),
            kind: UiElementKind::Group,
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiBitmapFont {
    #[serde(default)]
    pub asset: String,
    pub glyph_width: u32,
    pub glyph_height: u32,
    pub columns: u32,
    #[serde(default = "default_first_glyph")]
    pub first_codepoint: u32,
    #[serde(default = "default_ui_font_advance")]
    pub advance: f64,
    #[serde(default)]
    pub glyph_map: BTreeMap<String, u32>,
    /// Per-glyph image assets keyed by the exact character they render.
    #[serde(default)]
    pub glyph_assets: BTreeMap<String, String>,
    /// Optional relative scale for image-backed glyphs with intentionally
    /// different source canvas dimensions. Unspecified characters use 1.0.
    #[serde(default)]
    pub glyph_scales: BTreeMap<String, f64>,
    /// Optional per-glyph advance factors, expressed as a fraction of that
    /// glyph's measured width.
    #[serde(default)]
    pub glyph_advances: BTreeMap<String, f64>,
    /// Optional fixed distance between glyph centers, in reference-canvas
    /// pixels. This models image-based counters whose source layout positions
    /// each glyph around a shared center rather than flowing glyph advances.
    #[serde(default)]
    pub center_spacing: Option<f64>,
    /// Maximum decode dimension for image-backed glyphs. Most interface glyphs
    /// remain capped at 128 px; reference artwork can opt into its native
    /// source resolution when downsampling would change the rendered pixels.
    #[serde(default)]
    pub glyph_max_dimension: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlyphFamilyImport {
    pub mapped_characters: Vec<String>,
    pub ambiguous_characters: Vec<String>,
    pub unassigned_assets: Vec<String>,
}

/// Human-readable outcome of converting an asset-pack manifest into the
/// existing gameplay UI profile model. It deliberately distinguishes asset
/// registration from configured/renderable elements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetPackImportReport {
    pub pack_id: String,
    pub pack_name: String,
    pub manifest_path: PathBuf,
    pub asset_root: PathBuf,
    pub total_assets: usize,
    pub loaded_assets: Vec<String>,
    pub missing_assets: Vec<String>,
    pub invalid_assets: Vec<String>,
    pub unsupported_assets: Vec<String>,
    pub glyph_families: BTreeMap<String, GlyphFamilyImport>,
    pub unassigned_glyph_assets: Vec<String>,
    pub unresolved_glyph_assets: Vec<String>,
    pub unconfigured_element_families: Vec<String>,
    pub configured_elements: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiGameplayEvent {
    pub time: f64,
    pub event: String,
    pub note_type: Option<String>,
    pub payload: BTreeMap<String, f64>,
}

impl Default for UiGameplayEvent {
    fn default() -> Self {
        Self {
            time: 0.0,
            event: String::new(),
            note_type: None,
            payload: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum UiComboChange {
    Preserve,
    Increment,
    Reset,
    Set(i64),
}

impl Default for UiComboChange {
    fn default() -> Self {
        Self::Increment
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiScoreRule {
    pub event: String,
    pub note_type: Option<String>,
    pub score: f64,
    pub combo: UiComboChange,
    pub life: f64,
    pub judgement: String,
    pub variables: BTreeMap<String, f64>,
}

impl Default for UiScoreRule {
    fn default() -> Self {
        Self {
            event: String::new(),
            note_type: None,
            score: 0.0,
            combo: UiComboChange::Increment,
            life: 0.0,
            judgement: String::new(),
            variables: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiScoringConfig {
    /// `Automatic` preserves historical event-file behavior and otherwise
    /// selects the verified Sono-Overlay chart forecast.
    pub source: UiScoringSource,
    pub initial_score: f64,
    pub initial_combo: i64,
    pub initial_life: f64,
    pub minimum_life: f64,
    pub maximum_life: f64,
    pub rules: Vec<UiScoreRule>,
    pub forecast: crate::sono_overlay_scoring::ForecastConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UiScoringSource {
    #[default]
    Automatic,
    SonoOverlay,
    EventTrack,
    Disabled,
}

impl Default for UiScoringConfig {
    fn default() -> Self {
        Self {
            source: UiScoringSource::Automatic,
            initial_score: 0.0,
            initial_combo: 0,
            initial_life: 100.0,
            minimum_life: 0.0,
            maximum_life: 100.0,
            rules: Vec::new(),
            forecast: crate::sono_overlay_scoring::ForecastConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiGameplayState {
    pub score: f64,
    pub combo: i64,
    pub life: f64,
    pub judgment: Option<String>,
    /// Time of the most recent accepted gameplay event.
    pub latest_event_time: Option<f64>,
    /// Event time associated with `judgment`, when supplied by the adapter.
    pub judgment_time: Option<f64>,
    pub event: Option<String>,
    pub note_type: Option<String>,
    pub variables: BTreeMap<String, f64>,
}

impl UiGameplayState {
    pub fn from_config(config: &UiScoringConfig) -> Self {
        Self {
            score: config.initial_score,
            combo: config.initial_combo,
            life: config
                .initial_life
                .clamp(config.minimum_life, config.maximum_life),
            judgment: None,
            latest_event_time: None,
            judgment_time: None,
            event: None,
            note_type: None,
            variables: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, config: &UiScoringConfig, event: &UiGameplayEvent) -> Result<bool> {
        if !event.time.is_finite() || event.time < 0.0 {
            bail!("gameplay event has an invalid timestamp: {}", event.time);
        }
        let rule = config
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.event == event.event)
            .filter(|(_, rule)| {
                rule.note_type
                    .as_ref()
                    .is_none_or(|note_type| event.note_type.as_ref() == Some(note_type))
            })
            .max_by_key(|(index, rule)| (rule.note_type.is_some(), std::cmp::Reverse(*index)))
            .map(|(_, rule)| rule);
        let Some(rule) = rule else {
            return Ok(false);
        };
        self.score += rule.score;
        self.combo = match rule.combo {
            UiComboChange::Preserve => self.combo,
            UiComboChange::Increment => self.combo.saturating_add(1),
            UiComboChange::Reset => 0,
            UiComboChange::Set(value) => value,
        };
        self.life = (self.life + rule.life).clamp(config.minimum_life, config.maximum_life);
        self.judgment = (!rule.judgement.is_empty()).then(|| rule.judgement.clone());
        self.latest_event_time = Some(event.time);
        self.judgment_time = self.judgment.as_ref().map(|_| event.time);
        self.event = Some(event.event.clone());
        self.note_type = event.note_type.clone();
        for (name, value) in &rule.variables {
            *self.variables.entry(name.clone()).or_default() += value;
        }
        for (name, value) in &event.payload {
            if value.is_finite() {
                self.variables.insert(name.clone(), *value);
            }
        }
        if !self.score.is_finite() || self.variables.values().any(|value| !value.is_finite()) {
            bail!("gameplay scoring event produced a non-finite state value");
        }
        Ok(true)
    }
}

#[derive(Debug, Clone, Default)]
pub struct UiFrameState {
    pub time: f64,
    /// Output frame rate used by frame-based reference animations.
    pub frame_rate: f64,
    pub delta_time: f64,
    pub progress: f64,
    pub runtime_entities: usize,
    pub active_entities: usize,
    pub callbacks: usize,
    pub draws: usize,
    pub scheduled_events: usize,
    pub memory: Option<crate::runtime::Memory>,
    pub gameplay: Option<UiGameplayState>,
    /// User-readable status from the selected score provider. This is not
    /// inferred from Watch state and is omitted when no provider was selected.
    pub scoring_status: Option<String>,
    /// Expanded, deduplicated scoring warnings; kept separate from the compact
    /// status so a single unknown archetype cannot flood the preview.
    pub scoring_details: Vec<String>,
    /// Resolved renderer-authored values; these never write back to Watch.
    pub presentation_values: BTreeMap<String, f64>,
    pub presentation_categories: BTreeMap<String, String>,
}

fn default_ui_text_color() -> [u8; 4] {
    [245, 249, 255, 255]
}
fn default_ui_accent() -> [u8; 4] {
    [35, 220, 172, 255]
}
fn default_ui_track() -> [u8; 4] {
    [25, 36, 54, 230]
}
fn default_ui_font_size() -> f64 {
    28.0
}
fn default_ui_font_advance() -> f64 {
    1.0
}
fn default_first_glyph() -> u32 {
    32
}

fn recommended_glyph_family(fonts: &BTreeMap<String, UiBitmapFont>, role: &str) -> Option<String> {
    let candidates: Vec<_> = fonts
        .iter()
        .filter(|(name, font)| {
            let tokens: Vec<_> = name
                .split(['.', '-', '_'])
                .map(str::to_ascii_lowercase)
                .collect();
            tokens.first().is_some_and(|token| token == role)
                && tokens.iter().any(|token| token.contains("glyph"))
                && font.glyph_assets.keys().any(|character| {
                    character.chars().count() == 1
                        && character.chars().all(|ch| ch.is_ascii_digit())
                })
        })
        .map(|(name, _)| name.clone())
        .collect();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn apply_reference_glyph_import_report(
    report: &mut AssetPackImportReport,
    config: &RendererUiConfig,
    pack: &crate::presentation_assets::PresentationAssetPack,
) {
    let mut mapped_ids = BTreeSet::new();
    for (family_name, font_name) in [
        ("score.glyphs", "score"),
        ("score.glyphs", "score-shadow"),
        ("life.glyphs", "life"),
        ("life.glyphs", "life-shadow"),
        ("combo.glyph-family.p", "combo"),
        ("combo.glyph-family.b", "combo-burst"),
        // The reference object also selects `n0..n9` when AP Combo is off.
        // Keep that source-verified alternate family editable in an imported
        // profile without changing the AP layout's active `p`/`b` layers.
        ("combo.glyph-family.n", "combo.glyph-family.n"),
    ] {
        let Some(font) = config.fonts.get(font_name) else {
            continue;
        };
        let mut characters = BTreeSet::new();
        for (character, alias) in &font.glyph_assets {
            let Some(reference) = config.assets.get(alias) else {
                continue;
            };
            let Some(id) = reference
                .to_string_lossy()
                .strip_prefix("pack://")
                .map(str::to_owned)
            else {
                continue;
            };
            if pack
                .assets
                .iter()
                .any(|entry| entry.id == id && entry.family.as_deref() == Some(family_name))
            {
                characters.insert(character.clone());
                mapped_ids.insert(id);
            }
        }
        if let Some(family) = report.glyph_families.get_mut(family_name) {
            family.mapped_characters.extend(characters);
            family.mapped_characters.sort();
            family.mapped_characters.dedup();
            family.unassigned_assets.retain(|asset| {
                !mapped_ids
                    .iter()
                    .any(|id| asset == id || asset.starts_with(&format!("{id} (")))
            });
        }
    }
    report.unresolved_glyph_assets.retain(|asset| {
        !mapped_ids
            .iter()
            .any(|id| asset == id || asset.starts_with(&format!("{id} (")))
    });
    report.unconfigured_element_families.retain(|family| {
        report
            .glyph_families
            .get(family)
            .is_none_or(|mapping| mapping.mapped_characters.is_empty())
    });
}

fn validate_ui_import_image(path: &Path) -> Result<()> {
    const MAX_IMPORT_IMAGE_DIMENSION: u32 = 16_384;
    const MAX_IMPORT_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
    let reader = image::ImageReader::open(path)
        .with_context(|| format!("opening image {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying image format for {}", path.display()))?;
    let (width, height) = reader
        .into_dimensions()
        .with_context(|| format!("reading image dimensions for {}", path.display()))?;
    if width == 0
        || height == 0
        || width.max(height) > MAX_IMPORT_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMPORT_IMAGE_PIXELS
    {
        bail!("image dimensions {width}×{height} exceed the import safety limit");
    }
    image::ImageReader::open(path)
        .with_context(|| format!("opening image {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("identifying image format for {}", path.display()))?
        .decode()
        .with_context(|| format!("decoding image {}", path.display()))?;
    Ok(())
}

fn collect_ui_element_assets(elements: &[UiElement], assets: &mut BTreeSet<String>) {
    for element in elements {
        match &element.kind {
            UiElementKind::Image { asset, .. }
            | UiElementKind::Sprite { asset, .. }
            | UiElementKind::Icon { asset, .. } => {
                assets.insert(asset.clone());
            }
            UiElementKind::Gauge {
                fill_asset,
                mask_asset,
                ..
            }
            | UiElementKind::ProgressBar {
                fill_asset,
                mask_asset,
                ..
            } => {
                assets.extend(fill_asset.iter().cloned());
                assets.extend(mask_asset.iter().cloned());
            }
            _ => {}
        }
        assets.extend(
            element
                .style_rules
                .iter()
                .filter_map(|rule| rule.asset.clone()),
        );
        collect_ui_element_assets(&element.children, assets);
    }
}

fn slugify_ui_id(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "item".into()
    } else {
        result.into()
    }
}

pub(crate) fn unique_ui_element_id(elements: &[UiElement], base: &str) -> String {
    let occupied: BTreeSet<&str> = elements.iter().map(|element| element.id.as_str()).collect();
    if !occupied.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|index| format!("{base}-{index}"))
        .find(|candidate| !occupied.contains(candidate.as_str()))
        .unwrap_or_else(|| format!("{base}-extra"))
}

fn default_one_expression() -> UiExpression {
    UiExpression::Constant { value: 1.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiValueProvider {
    Fixed {
        value: f64,
    },
    External {
        value: f64,
    },
    Memory {
        block_id: i64,
        index: usize,
    },
    Progress,
    /// Reserved until an authoritative engine metric source is available.
    EngineMetric {
        key: String,
    },
    /// Reserved until generic replay judgment aggregation is implemented.
    Accuracy,
    /// Reserved for a caller-defined judgment-to-value mapping.
    JudgmentDerived {
        mapping: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiMetric {
    pub enabled: bool,
    pub label: String,
    pub provider: UiValueProvider,
    pub maximum: Option<f64>,
}

impl Default for UiMetric {
    fn default() -> Self {
        Self {
            enabled: true,
            label: "METRIC".into(),
            provider: UiValueProvider::Fixed { value: 0.0 },
            maximum: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiValueComponent {
    pub enabled: bool,
    pub label: String,
    pub provider: UiValueProvider,
}

impl Default for UiValueComponent {
    fn default() -> Self {
        Self {
            enabled: false,
            label: String::new(),
            provider: UiValueProvider::Fixed { value: 0.0 },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RendererUiConfig {
    pub mode: GameplayUiMode,
    pub enabled: bool,
    pub profile_name: String,
    /// True only when the user deliberately chose an empty custom HUD.
    /// Unmarked legacy empty defaults recover to the built-in HUD.
    #[serde(default)]
    pub intentionally_empty: bool,
    pub design_size: [u32; 2],
    pub fit: UiFitMode,
    /// Source-derived alternate canvas layouts. Group centers are expressed
    /// in their respective reference pixel coordinates; `alternate_scale`
    /// is relative to the profile's primary reference group scale.
    #[serde(default)]
    pub responsive_layouts: BTreeMap<String, UiResponsiveLayout>,
    pub elements: Vec<UiElement>,
    pub variables: BTreeMap<String, UiExpression>,
    /// Generic deterministic profile timeline for numeric and categorical values.
    pub timeline: UiTimeline,
    /// Optional versioned asset pack used by `pack://logical-id` references.
    pub asset_pack: Option<PathBuf>,
    /// Optional user-selected root for pack-relative sources. Missing means
    /// the manifest's directory, preserving profiles written before this field.
    #[serde(default)]
    pub asset_root: Option<PathBuf>,
    /// Asset ids map to image files relative to the profile file.
    pub assets: BTreeMap<String, PathBuf>,
    /// Bitmap-font atlases use fixed-size cells; no platform font runtime is required.
    pub fonts: BTreeMap<String, UiBitmapFont>,
    pub scoring: Option<UiScoringConfig>,
    /// Optional JSON array of time-stamped GameplayEvent values supplied by a gameplay host.
    pub events_file: Option<PathBuf>,
    pub primary_metric: UiMetric,
    pub secondary_metric: UiMetric,
    pub combo: UiValueComponent,
    pub judgment: UiValueComponent,
    pub progress: bool,
    pub theme: UiTheme,
    #[serde(skip, default)]
    pub(crate) loaded_assets: BTreeMap<String, Arc<UiBitmap>>,
    #[serde(skip, default)]
    loaded_asset_paths: BTreeMap<String, PathBuf>,
    #[serde(skip, default)]
    pub(crate) loaded_events: Vec<UiGameplayEvent>,
    #[serde(skip, default)]
    loaded_forecast: Option<crate::sono_overlay_scoring::Forecast>,
    #[serde(skip, default)]
    scoring_status: Option<String>,
    #[serde(skip, default)]
    scoring_diagnostics: Vec<crate::sono_overlay_scoring::ScoringDiagnostic>,
    #[serde(skip, default)]
    profile_directory: Option<PathBuf>,
    #[serde(skip, default)]
    external_data_loaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiResponsiveLayout {
    pub reference_size: [u32; 2],
    pub alternate_size: [u32; 2],
    pub groups: BTreeMap<String, UiResponsiveGroup>,
}

impl Default for UiResponsiveLayout {
    fn default() -> Self {
        Self {
            reference_size: [1920, 1080],
            alternate_size: [1440, 1080],
            groups: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiResponsiveGroup {
    pub reference_center: [f64; 2],
    pub alternate_center: [f64; 2],
    pub alternate_scale: f64,
}

impl Default for UiResponsiveGroup {
    fn default() -> Self {
        Self {
            reference_center: [0.0; 2],
            alternate_center: [0.0; 2],
            alternate_scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiTheme {
    pub panel: [u8; 3],
    pub panel_alpha: u8,
    pub text: [u8; 3],
    pub label: [u8; 3],
    pub accent: [u8; 3],
    pub progress_track: [u8; 3],
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            panel: [12, 16, 26],
            panel_alpha: 205,
            text: [255, 255, 255],
            label: [210, 220, 235],
            accent: [70, 210, 220],
            progress_track: [25, 29, 40],
        }
    }
}

#[derive(Clone)]
struct BuiltInUiProfileSeed {
    profile_name: String,
    design_size: [u32; 2],
    fit: UiFitMode,
    responsive_layouts: BTreeMap<String, UiResponsiveLayout>,
    elements: Vec<UiElement>,
    variables: BTreeMap<String, UiExpression>,
    timeline: UiTimeline,
    assets: BTreeMap<String, PathBuf>,
    fonts: BTreeMap<String, UiBitmapFont>,
    scoring: Option<UiScoringConfig>,
    primary_metric: UiMetric,
    secondary_metric: UiMetric,
    combo: UiValueComponent,
    judgment: UiValueComponent,
    progress: bool,
    theme: UiTheme,
}

fn decode_builtin_ui_field<T: DeserializeOwned>(
    document: &serde_json::Value,
    name: &str,
    fallback: T,
) -> T {
    document
        .get(name)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(fallback)
}

/// This checked-in profile is the executable reference implementation for
/// Sono-Overlay's 16:9 gameplay HUD. Keeping its existing profile schema as
/// the seed prevents a second hand-maintained Rust layout from drifting.
fn built_in_ui_profile_seed() -> &'static BuiltInUiProfileSeed {
    static SEED: OnceLock<BuiltInUiProfileSeed> = OnceLock::new();
    SEED.get_or_init(|| {
        let document: serde_json::Value =
            serde_json::from_str(include_str!("../ui-profiles/default.json"))
                .expect("the bundled gameplay UI reference profile must be valid JSON");
        BuiltInUiProfileSeed {
            profile_name: document
                .get("profile_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Asset-backed Gameplay HUD")
                .to_owned(),
            design_size: decode_builtin_ui_field(&document, "design_size", [1920, 1080]),
            fit: decode_builtin_ui_field(&document, "fit", UiFitMode::Contain),
            responsive_layouts: decode_builtin_ui_field(
                &document,
                "responsive_layouts",
                BTreeMap::new(),
            ),
            elements: decode_builtin_ui_field(&document, "elements", Vec::new()),
            variables: decode_builtin_ui_field(&document, "variables", BTreeMap::new()),
            timeline: decode_builtin_ui_field(&document, "timeline", UiTimeline::default()),
            assets: decode_builtin_ui_field(&document, "assets", BTreeMap::new()),
            fonts: decode_builtin_ui_field(&document, "fonts", BTreeMap::new()),
            scoring: decode_builtin_ui_field(&document, "scoring", None),
            primary_metric: decode_builtin_ui_field(
                &document,
                "primary_metric",
                UiMetric::default(),
            ),
            secondary_metric: decode_builtin_ui_field(
                &document,
                "secondary_metric",
                UiMetric {
                    enabled: false,
                    label: "SECONDARY".into(),
                    ..UiMetric::default()
                },
            ),
            combo: decode_builtin_ui_field(
                &document,
                "combo",
                UiValueComponent {
                    label: "COMBO".into(),
                    ..UiValueComponent::default()
                },
            ),
            judgment: decode_builtin_ui_field(
                &document,
                "judgment",
                UiValueComponent {
                    label: "JUDGMENT".into(),
                    ..UiValueComponent::default()
                },
            ),
            progress: decode_builtin_ui_field(&document, "progress", true),
            theme: decode_builtin_ui_field(&document, "theme", UiTheme::default()),
        }
    })
}

impl RendererUiConfig {
    /// Reference-based starter layout for application defaults and migration.
    /// `Default` remains an empty document so older partial profiles deserialize
    /// without acquiring new assets or presentation elements implicitly.
    pub fn reference_default() -> Self {
        let seed = built_in_ui_profile_seed();
        let resource_root = crate::store_paths::application_resource_root();
        Self {
            mode: GameplayUiMode::Custom,
            enabled: true,
            profile_name: seed.profile_name.clone(),
            intentionally_empty: false,
            design_size: seed.design_size,
            fit: seed.fit,
            responsive_layouts: seed.responsive_layouts.clone(),
            elements: seed.elements.clone(),
            variables: seed.variables.clone(),
            timeline: seed.timeline.clone(),
            asset_pack: Some(resource_root.join("assets/presentation-pack.json")),
            asset_root: None,
            assets: seed.assets.clone(),
            fonts: seed.fonts.clone(),
            scoring: seed.scoring.clone(),
            events_file: None,
            primary_metric: seed.primary_metric.clone(),
            secondary_metric: seed.secondary_metric.clone(),
            combo: seed.combo.clone(),
            judgment: seed.judgment.clone(),
            progress: seed.progress,
            theme: seed.theme.clone(),
            loaded_assets: BTreeMap::new(),
            loaded_asset_paths: BTreeMap::new(),
            loaded_events: Vec::new(),
            loaded_forecast: None,
            scoring_status: None,
            scoring_diagnostics: Vec::new(),
            profile_directory: Some(resource_root.join("ui-profiles")),
            external_data_loaded: false,
        }
    }
}

impl Default for RendererUiConfig {
    fn default() -> Self {
        Self {
            mode: GameplayUiMode::Custom,
            enabled: false,
            profile_name: "Custom".into(),
            intentionally_empty: false,
            design_size: [1920, 1080],
            fit: UiFitMode::Contain,
            responsive_layouts: BTreeMap::new(),
            elements: Vec::new(),
            variables: BTreeMap::new(),
            timeline: UiTimeline::default(),
            asset_pack: None,
            asset_root: None,
            assets: BTreeMap::new(),
            fonts: BTreeMap::new(),
            scoring: None,
            events_file: None,
            primary_metric: UiMetric::default(),
            secondary_metric: UiMetric {
                enabled: false,
                label: "SECONDARY".into(),
                ..UiMetric::default()
            },
            combo: UiValueComponent {
                label: "COMBO".into(),
                ..UiValueComponent::default()
            },
            judgment: UiValueComponent {
                label: "JUDGMENT".into(),
                ..UiValueComponent::default()
            },
            progress: true,
            theme: UiTheme::default(),
            loaded_assets: BTreeMap::new(),
            loaded_asset_paths: BTreeMap::new(),
            loaded_events: Vec::new(),
            loaded_forecast: None,
            scoring_status: None,
            scoring_diagnostics: Vec::new(),
            profile_directory: None,
            external_data_loaded: false,
        }
    }
}

fn push_default_hud_text(
    elements: &mut Vec<UiElement>,
    id: &str,
    z_index: i32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    text: String,
    color: [u8; 4],
    font_size: f64,
    align: UiTextAlign,
    visible_when: Option<UiCondition>,
) {
    elements.push(UiElement {
        id: id.into(),
        z_index,
        layout: UiLayout {
            x,
            y,
            width,
            height,
            ..UiLayout::default()
        },
        visible_when,
        kind: UiElementKind::Text {
            text,
            value: None,
            color,
            font_size,
            font: None,
            align,
        },
        ..UiElement::default()
    });
}

/// Create the built-in HUD using the existing generic UI primitives. Runtime
/// telemetry is always available; score/combo/life values are shown only when
/// an authoritative gameplay-state provider supplied them.
pub fn default_gameplay_ui_elements(
    width: u32,
    height: u32,
    score_font: Option<&str>,
    combo_font: Option<&str>,
    life_font: Option<&str>,
) -> Vec<UiElement> {
    let portrait = height > width.saturating_mul(6) / 5;
    let short_edge = f64::from(width.min(height).max(1));
    let font_size = (short_edge * if portrait { 0.027 } else { 0.035 }).clamp(14.0, 38.0);
    let small_font = (font_size * 0.52).clamp(10.0, 20.0);
    let panel = if portrait {
        UiLayout {
            x: 0.035,
            y: 0.018,
            width: 0.93,
            height: 0.145,
            ..UiLayout::default()
        }
    } else {
        UiLayout {
            x: 0.02,
            y: 0.025,
            width: 0.32,
            height: 0.31,
            ..UiLayout::default()
        }
    };
    let mut elements = vec![UiElement {
        id: "default-hud-panel".into(),
        z_index: 0,
        layout: panel,
        kind: UiElementKind::Rectangle {
            color: [7, 12, 22, 210],
            border: Some([91, 130, 166, 245]),
            radius: 16.0,
        },
        ..UiElement::default()
    }];

    if portrait {
        push_default_hud_text(
            &mut elements,
            "default-hud-title",
            2,
            0.055,
            0.024,
            0.45,
            0.033,
            "GAMEPLAY".into(),
            [255, 255, 255, 255],
            small_font,
            UiTextAlign::Left,
            None,
        );
    } else {
        push_default_hud_text(
            &mut elements,
            "default-hud-title",
            2,
            0.04,
            0.042,
            0.27,
            0.05,
            "GAMEPLAY HUD".into(),
            [255, 255, 255, 255],
            small_font,
            UiTextAlign::Left,
            None,
        );
    }

    let rows = if portrait {
        [
            ("score", "SCORE", UiStateSource::Score, score_font, 0.055),
            ("combo", "COMBO", UiStateSource::Combo, combo_font, 0.365),
            ("life", "LIFE", UiStateSource::Life, life_font, 0.675),
        ]
    } else {
        [
            ("score", "SCORE", UiStateSource::Score, score_font, 0.075),
            ("combo", "COMBO", UiStateSource::Combo, combo_font, 0.145),
            ("life", "LIFE", UiStateSource::Life, life_font, 0.215),
        ]
    };
    for (index, (id, label, source, font, position)) in rows.into_iter().enumerate() {
        let available = UiCondition::Equal {
            left: UiExpression::Source {
                source: source.clone(),
            },
            right: UiExpression::Source {
                source: source.clone(),
            },
        };
        let unavailable = UiCondition::Not {
            condition: Box::new(available.clone()),
        };
        if portrait {
            let column_width = 0.28;
            push_default_hud_text(
                &mut elements,
                &format!("default-hud-{id}-label"),
                3 + index as i32,
                position,
                0.068,
                column_width,
                0.026,
                label.into(),
                [170, 195, 220, 255],
                small_font,
                UiTextAlign::Left,
                None,
            );
            let x = position;
            let value_width = column_width;
            elements.push(UiElement {
                id: format!("default-hud-{id}-value"),
                z_index: 6 + index as i32,
                layout: UiLayout {
                    x,
                    y: 0.094,
                    width: value_width,
                    height: 0.045,
                    ..UiLayout::default()
                },
                visible_when: Some(available),
                kind: UiElementKind::Counter {
                    value: UiExpression::Source {
                        source: source.clone(),
                    },
                    prefix: String::new(),
                    suffix: String::new(),
                    digits: 0,
                    max_digits: None,
                    color: [255, 255, 255, 255],
                    font_size,
                    font: font.map(str::to_owned),
                    align: UiTextAlign::Left,
                },
                ..UiElement::default()
            });
            push_default_hud_text(
                &mut elements,
                &format!("default-hud-{id}-unavailable"),
                9 + index as i32,
                x,
                0.094,
                value_width,
                0.045,
                "N/A".into(),
                [185, 195, 210, 255],
                font_size,
                UiTextAlign::Left,
                Some(unavailable),
            );
        } else {
            let index = index as i32;
            let label_y = position;
            push_default_hud_text(
                &mut elements,
                &format!("default-hud-{id}-label"),
                3 + index,
                0.045,
                label_y,
                0.12,
                0.052,
                label.into(),
                [170, 195, 220, 255],
                small_font,
                UiTextAlign::Left,
                None,
            );
            elements.push(UiElement {
                id: format!("default-hud-{id}-value"),
                z_index: 6 + index,
                layout: UiLayout {
                    x: 0.17,
                    y: label_y,
                    width: 0.15,
                    height: 0.052,
                    ..UiLayout::default()
                },
                visible_when: Some(available),
                kind: UiElementKind::Counter {
                    value: UiExpression::Source {
                        source: source.clone(),
                    },
                    prefix: String::new(),
                    suffix: String::new(),
                    digits: 0,
                    max_digits: None,
                    color: [255, 255, 255, 255],
                    font_size,
                    font: font.map(str::to_owned),
                    align: UiTextAlign::Right,
                },
                ..UiElement::default()
            });
            push_default_hud_text(
                &mut elements,
                &format!("default-hud-{id}-unavailable"),
                9 + index,
                0.17,
                label_y,
                0.15,
                0.052,
                "N/A".into(),
                [185, 195, 210, 255],
                font_size,
                UiTextAlign::Right,
                Some(unavailable),
            );
        }
    }

    push_default_hud_text(
        &mut elements,
        "default-hud-runtime-status",
        20,
        if portrait { 0.055 } else { 0.045 },
        if portrait { 0.174 } else { 0.285 },
        if portrait { 0.89 } else { 0.27 },
        0.035,
        "TIME {time}s  ACTIVE {active_entities}  DRAWS {draws}".into(),
        [190, 210, 232, 255],
        small_font,
        UiTextAlign::Left,
        None,
    );

    let judgment_rect = if portrait {
        [0.15, 0.22, 0.70, 0.065]
    } else {
        [0.40, 0.035, 0.24, 0.085]
    };
    elements.push(UiElement {
        id: "default-hud-judgment-panel".into(),
        z_index: 30,
        layout: UiLayout {
            x: judgment_rect[0],
            y: judgment_rect[1],
            width: judgment_rect[2],
            height: judgment_rect[3],
            ..UiLayout::default()
        },
        kind: UiElementKind::Rectangle {
            color: [8, 12, 22, 205],
            border: Some([91, 130, 166, 245]),
            radius: 12.0,
        },
        ..UiElement::default()
    });
    push_default_hud_text(
        &mut elements,
        "default-hud-judgment",
        31,
        judgment_rect[0],
        judgment_rect[1],
        judgment_rect[2],
        judgment_rect[3],
        "{judgment}".into(),
        [255, 255, 255, 255],
        font_size,
        UiTextAlign::Center,
        None,
    );
    elements.push(UiElement {
        id: "default-hud-progress".into(),
        z_index: 40,
        layout: UiLayout {
            x: 0.02,
            y: 0.96,
            width: 0.96,
            height: 0.012,
            ..UiLayout::default()
        },
        kind: UiElementKind::ProgressBar {
            value: UiExpression::Source {
                source: UiStateSource::Progress,
            },
            maximum: UiExpression::Constant { value: 1.0 },
            orientation: UiGaugeOrientation::LeftToRight,
            fill: [35, 220, 172, 255],
            track: [25, 36, 54, 230],
            fill_asset: None,
            fill_source: None,
            mask_asset: None,
        },
        ..UiElement::default()
    });
    elements
}

impl RendererUiConfig {
    fn use_reference_hud(&mut self, keep_pack: bool) {
        let current_pack = self.asset_pack.clone();
        let current_root = self.asset_root.clone();
        let current_directory = self.profile_directory.clone();
        let current_assets = self.assets.clone();
        let profile = Self::reference_default();
        self.profile_name = profile.profile_name;
        self.mode = GameplayUiMode::Custom;
        self.enabled = true;
        self.intentionally_empty = false;
        self.design_size = profile.design_size;
        self.fit = profile.fit;
        self.responsive_layouts = profile.responsive_layouts;
        self.elements = profile.elements;
        self.variables = profile.variables;
        self.timeline = profile.timeline;
        self.scoring = profile.scoring;
        self.primary_metric = profile.primary_metric;
        self.secondary_metric = profile.secondary_metric;
        self.combo = profile.combo;
        self.judgment = profile.judgment;
        self.progress = profile.progress;
        self.theme = profile.theme;
        if keep_pack {
            self.asset_pack = current_pack;
            self.asset_root = current_root;
            self.profile_directory = current_directory;
            self.assets = profile.assets;
            self.assets.extend(current_assets);
            self.fonts = profile.fonts;
        } else {
            self.asset_pack = profile.asset_pack;
            self.asset_root = profile.asset_root;
            self.profile_directory = profile.profile_directory;
            self.assets = profile.assets;
            self.fonts = profile.fonts;
        }
        self.events_file = None;
        self.loaded_assets.clear();
        self.loaded_asset_paths.clear();
        self.loaded_events.clear();
        self.loaded_forecast = None;
        self.scoring_status = None;
        self.scoring_diagnostics.clear();
        self.external_data_loaded = false;
    }

    fn has_complete_reference_asset_set(&self) -> bool {
        let seed = built_in_ui_profile_seed();
        seed.assets.iter().all(|(alias, reference)| {
            let Some(id) = reference
                .to_string_lossy()
                .strip_prefix("pack://")
                .map(str::to_owned)
            else {
                return self.assets.contains_key(alias);
            };
            self.assets.contains_key(alias) || self.assets.contains_key(&id)
        })
    }

    /// Replace only an empty document with the normal starter HUD. Existing
    /// custom elements are never discarded by this operation.
    pub fn populate_with_default_hud(&mut self) -> bool {
        if !self.elements.is_empty() {
            return false;
        }
        if self.has_complete_reference_asset_set() {
            self.use_reference_hud(true);
        } else if Self::reference_default().has_complete_reference_asset_set() {
            // The current pack is unrelated or incomplete. Switch to the
            // bundled Sono-Overlay pack instead of pairing its layout with
            // whatever pack happened to be imported.
            self.use_reference_hud(false);
        } else {
            let score = recommended_glyph_family(&self.fonts, "score");
            let combo = recommended_glyph_family(&self.fonts, "combo");
            let life = recommended_glyph_family(&self.fonts, "life");
            self.elements = default_gameplay_ui_elements(
                self.design_size[0],
                self.design_size[1],
                score.as_deref(),
                combo.as_deref(),
                life.as_deref(),
            );
            self.profile_name = "Built-in Default HUD".into();
            self.mode = GameplayUiMode::Custom;
            self.enabled = true;
            self.intentionally_empty = false;
        }
        true
    }

    /// Upgrade the exact old, untouched empty default shape. A deliberately
    /// empty profile saved by current versions carries `intentionally_empty`.
    pub fn migrate_legacy_empty_default(&mut self) -> bool {
        let default_secondary = UiMetric {
            enabled: false,
            label: "SECONDARY".into(),
            ..UiMetric::default()
        };
        let default_combo = UiValueComponent {
            label: "COMBO".into(),
            ..UiValueComponent::default()
        };
        let default_judgment = UiValueComponent {
            label: "JUDGMENT".into(),
            ..UiValueComponent::default()
        };
        if self.mode != GameplayUiMode::Custom
            || self.profile_name != "Custom"
            || self.intentionally_empty
            || !self.elements.is_empty()
            || !self.variables.is_empty()
            || !self.timeline.tracks.is_empty()
            || self.asset_pack.is_some()
            || self.asset_root.is_some()
            || !self.assets.is_empty()
            || !self.fonts.is_empty()
            || self.scoring.is_some()
            || self.events_file.is_some()
            || self.primary_metric != UiMetric::default()
            || self.secondary_metric != default_secondary
            || self.combo != default_combo
            || self.judgment != default_judgment
            || !self.progress
            || self.theme != UiTheme::default()
        {
            return false;
        }
        self.use_reference_hud(false);
        true
    }

    /// Convert profile-relative resources into explicit paths before a profile
    /// is embedded in the shared renderer configuration.
    pub(crate) fn resolve_profile_resources(&mut self, profile_path: &Path) {
        let base = profile_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .unwrap_or_else(|_| {
                profile_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });
        self.resolve_config_resources(&base);
    }

    /// Resolve resources in an embedded config relative to its containing
    /// renderer configuration file.
    pub(crate) fn resolve_config_resources(&mut self, base: &Path) {
        fn resolve(path: &mut PathBuf, base: &Path) {
            if !path.as_os_str().is_empty() && !path.is_absolute() {
                *path = base.join(&*path);
            }
        }
        for path in self.assets.values_mut() {
            if !path.to_string_lossy().starts_with("pack://") {
                resolve(path, base);
            }
        }
        if let Some(path) = &mut self.asset_pack {
            resolve(path, base);
        }
        if let Some(path) = &mut self.asset_root {
            resolve(path, base);
        }
        if let Some(path) = &mut self.events_file {
            resolve(path, base);
        }
        self.profile_directory = None;
        self.loaded_assets.clear();
        self.loaded_events.clear();
        self.loaded_forecast = None;
        self.scoring_status = None;
        self.scoring_diagnostics.clear();
        self.external_data_loaded = false;
    }

    pub(crate) fn relativize_config_resources(&mut self, base: &Path) {
        for path in self.assets.values_mut() {
            if !path.to_string_lossy().starts_with("pack://") {
                crate::render::relativize_path(path, base);
            }
        }
        if let Some(path) = &mut self.asset_pack {
            crate::render::relativize_path(path, base);
        }
        if let Some(path) = &mut self.asset_root {
            crate::render::relativize_path(path, base);
        }
        if let Some(path) = &mut self.events_file {
            crate::render::relativize_path(path, base);
        }
    }

    /// Return the decoded dimensions of an asset that has been loaded for this profile.
    pub fn loaded_asset_dimensions(&self, id: &str) -> Option<(u32, u32)> {
        self.loaded_assets
            .get(id)
            .map(|bitmap| (bitmap.width, bitmap.height))
    }

    /// Resolved source path for manifesting, diagnostics, and editor asset details.
    pub fn loaded_asset_path(&self, id: &str) -> Option<&Path> {
        self.loaded_asset_paths.get(id).map(PathBuf::as_path)
    }

    pub fn resolved_asset_pack_path(&self) -> Option<PathBuf> {
        self.asset_pack.as_ref().map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            }
        })
    }

    pub fn load_profile(path: impl AsRef<Path>) -> Result<Self> {
        let mut config = Self::load_profile_for_repair(path)?;
        config.migrate_legacy_empty_default();
        config.load_external_data()?;
        config.validate()?;
        Ok(config)
    }

    /// Parse and validate the profile's durable document without opening its
    /// external files. GUI editors use this only after strict loading fails so
    /// the user can inspect the saved layout and repair a moved asset root.
    pub fn load_profile_for_repair(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path)
            .map_err(anyhow::Error::from)
            .map_err(|error| {
                error.context(format!("reading gameplay UI profile {}", path.display()))
            })?;
        let mut document: serde_json::Value = serde_json::from_slice(&data).map_err(|error| {
            anyhow::anyhow!("parsing gameplay UI profile {}: {error}", path.display())
        })?;
        crate::render::strip_optional_flat_schema(
            &mut document,
            UI_PROFILE_FORMAT,
            UI_PROFILE_SCHEMA_VERSION,
            "gameplay UI profile",
        )?;
        let mut config: Self = serde_json::from_value(document).map_err(|error| {
            anyhow::anyhow!("parsing gameplay UI profile {}: {error}", path.display())
        })?;
        config.profile_directory = path
            .parent()
            .map(Path::to_path_buf)
            .map(|parent| parent.canonicalize().unwrap_or(parent));
        config.mode = GameplayUiMode::Custom;
        config.enabled = true;
        config.validate()?;
        Ok(config)
    }

    /// Save this gameplay UI profile atomically in its versioned standalone
    /// profile format. External assets remain external; paths inside the
    /// profile directory are stored relatively for portability.
    pub fn save_profile(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self.validate()?;
        let base = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut stored = self.clone();
        stored.relativize_config_resources(base);
        let mut document =
            serde_json::to_value(stored).context("serializing gameplay UI profile")?;
        let object = document
            .as_object_mut()
            .context("gameplay UI profile did not serialize as an object")?;
        object.insert(
            "format".into(),
            serde_json::Value::String(UI_PROFILE_FORMAT.into()),
        );
        object.insert(
            "schema_version".into(),
            serde_json::Value::from(UI_PROFILE_SCHEMA_VERSION),
        );
        let bytes =
            serde_json::to_vec_pretty(&document).context("formatting gameplay UI profile")?;
        crate::render::write_config_atomic(path, &bytes)
            .with_context(|| format!("saving gameplay UI profile {}", path.display()))
    }

    /// Import a versioned presentation asset pack into this module's existing
    /// profile model. The manifest remains the source of identity and paths;
    /// only explicitly labeled glyph characters and visible-label tags are
    /// assigned automatically.
    pub fn import_asset_pack(
        manifest_path: impl AsRef<Path>,
        asset_root: Option<PathBuf>,
    ) -> Result<(Self, AssetPackImportReport)> {
        let manifest_path = manifest_path.as_ref().canonicalize().with_context(|| {
            format!(
                "resolving asset-pack manifest {}",
                manifest_path.as_ref().display()
            )
        })?;
        let pack = crate::presentation_assets::PresentationAssetPack::load(&manifest_path)?;
        let inferred_root = asset_root.or_else(|| manifest_path.parent().map(Path::to_path_buf));
        let root = inferred_root
            .as_deref()
            .context("asset-pack manifest has no parent directory")?
            .canonicalize()
            .with_context(|| {
                format!(
                    "resolving selected asset root {}",
                    inferred_root.as_deref().unwrap().display()
                )
            })?;

        let mut config = Self {
            profile_name: pack.name.clone(),
            asset_pack: Some(manifest_path.clone()),
            asset_root: Some(root.clone()),
            enabled: true,
            mode: GameplayUiMode::Custom,
            scoring: Some(UiScoringConfig::default()),
            elements: Vec::new(),
            ..Self::default()
        };
        let mut report = AssetPackImportReport {
            pack_id: pack.id.clone(),
            pack_name: pack.name.clone(),
            manifest_path: manifest_path.clone(),
            asset_root: root.clone(),
            total_assets: pack.assets.len(),
            ..AssetPackImportReport::default()
        };
        let mut glyph_families: BTreeMap<
            String,
            Vec<&crate::presentation_assets::PresentationAssetEntry>,
        > = BTreeMap::new();

        for entry in &pack.assets {
            match &entry.kind {
                crate::presentation_assets::PresentationAssetKind::Image
                | crate::presentation_assets::PresentationAssetKind::Sprite
                | crate::presentation_assets::PresentationAssetKind::Glyph => {
                    let resolved =
                        pack.resolve_source_with_root(&manifest_path, &entry.source, Some(&root));
                    let validation = resolved.and_then(|resolved| {
                        validate_ui_import_image(&resolved).map(|()| resolved)
                    });
                    match validation {
                        Ok(_) => {
                            config.assets.insert(
                                entry.id.clone(),
                                PathBuf::from(format!("pack://{}", entry.id)),
                            );
                            report.loaded_assets.push(entry.id.clone());
                        }
                        Err(error) => {
                            let detail =
                                format!("{} ({}) — {error:#}", entry.id, entry.source.display());
                            if root.join(&entry.source).exists() {
                                report.invalid_assets.push(detail);
                            } else {
                                report.missing_assets.push(detail);
                            }
                        }
                    }
                    if entry.kind == crate::presentation_assets::PresentationAssetKind::Glyph {
                        if let Some(family) = entry
                            .family
                            .as_deref()
                            .filter(|family| !family.trim().is_empty())
                        {
                            glyph_families
                                .entry(family.to_owned())
                                .or_default()
                                .push(entry);
                        } else {
                            report.unassigned_glyph_assets.push(entry.id.clone());
                        }
                    }
                }
                _ => report
                    .unsupported_assets
                    .push(format!("{} ({:?})", entry.id, entry.kind)),
            }
        }

        for (family, entries) in glyph_families {
            let mut candidates: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut family_unassigned = Vec::new();
            for entry in entries {
                match entry.character.as_deref() {
                    Some(character)
                        if character.chars().count() == 1
                            && config.assets.contains_key(&entry.id) =>
                    {
                        candidates
                            .entry(character.to_owned())
                            .or_default()
                            .push(entry.id.clone());
                    }
                    Some(character) if character.chars().count() != 1 => {
                        let detail = format!("{} (character must be one Unicode scalar)", entry.id);
                        report.unresolved_glyph_assets.push(detail.clone());
                        family_unassigned.push(detail);
                    }
                    _ => {
                        report.unresolved_glyph_assets.push(entry.id.clone());
                        family_unassigned.push(entry.id.clone());
                    }
                }
            }
            let mut glyph_assets = BTreeMap::new();
            let mut ambiguous = Vec::new();
            for (character, assets) in candidates {
                if assets.len() == 1 {
                    glyph_assets.insert(character, assets[0].clone());
                } else {
                    ambiguous.push(character);
                }
            }
            let mapped_characters = glyph_assets.keys().cloned().collect();
            if !glyph_assets.is_empty() {
                // Per-character bitmap fonts do not use atlas cell dimensions;
                // positive placeholders satisfy the existing shared schema.
                config.fonts.insert(
                    family.clone(),
                    UiBitmapFont {
                        asset: String::new(),
                        glyph_width: 1,
                        glyph_height: 1,
                        columns: 1,
                        first_codepoint: default_first_glyph(),
                        advance: 1.0,
                        glyph_map: BTreeMap::new(),
                        glyph_assets,
                        glyph_scales: BTreeMap::new(),
                        glyph_advances: BTreeMap::new(),
                        center_spacing: None,
                        glyph_max_dimension: None,
                    },
                );
            }
            report.glyph_families.insert(
                family,
                GlyphFamilyImport {
                    mapped_characters,
                    ambiguous_characters: ambiguous,
                    unassigned_assets: family_unassigned,
                },
            );
        }

        let score_font = recommended_glyph_family(&config.fonts, "score");
        let combo_font = recommended_glyph_family(&config.fonts, "combo");
        let life_font = recommended_glyph_family(&config.fonts, "life");
        config.elements = default_gameplay_ui_elements(
            config.design_size[0],
            config.design_size[1],
            score_font.as_deref(),
            combo_font.as_deref(),
            life_font.as_deref(),
        );

        let mut judgment_labels = BTreeSet::new();
        for entry in &pack.assets {
            if !config.assets.contains_key(&entry.id) {
                continue;
            }
            // `visible-label` alone only describes content. Bind it to the
            // judgement state only when the pack's family explicitly declares
            // that this is judgement artwork.
            let is_judgment_family = entry
                .family
                .as_deref()
                .is_some_and(|family| family.to_ascii_lowercase().contains("judg"));
            if !is_judgment_family {
                continue;
            }
            for tag in &entry.tags {
                if let Some(label) = tag
                    .strip_prefix("visible-label:")
                    .filter(|label| !label.trim().is_empty())
                {
                    if judgment_labels.insert(label.to_owned()) {
                        config.elements.push(UiElement {
                            id: unique_ui_element_id(
                                &config.elements,
                                &format!("judgment-{}", slugify_ui_id(label)),
                            ),
                            z_index: 100,
                            layout: UiLayout {
                                x: 0.40,
                                y: 0.08,
                                width: 0.20,
                                height: 0.10,
                                anchor: UiAnchor::TopLeft,
                                ..UiLayout::default()
                            },
                            visible_when: Some(UiCondition::StringEquals {
                                source: UiStringSource::Judgment,
                                value: label.to_owned(),
                            }),
                            kind: UiElementKind::Image {
                                asset: entry.id.clone(),
                                tint: None,
                            },
                            ..UiElement::default()
                        });
                    }
                }
            }
        }
        let non_ap_combo_font = config.fonts.get("combo.glyph-family.n").cloned();
        if config.has_complete_reference_asset_set() {
            config.use_reference_hud(true);
            if let Some(mut font) = non_ap_combo_font {
                // Sono-Overlay lays out the `n` digits in the same fixed
                // character slots as the AP foreground and burst layers.
                font.center_spacing = Some(108.0);
                font.glyph_max_dimension = Some(256);
                config.fonts.insert("combo.glyph-family.n".into(), font);
            }
            config.profile_name = format!("{} — Sono-Overlay reference HUD", pack.name);
            apply_reference_glyph_import_report(&mut report, &config, &pack);
        }
        report.configured_elements = config.elements.len();
        report.unconfigured_element_families.extend(
            report
                .glyph_families
                .iter()
                .filter(|(_, family)| family.mapped_characters.is_empty())
                .map(|(name, _)| name.clone()),
        );
        config.profile_directory = manifest_path.parent().map(Path::to_path_buf);
        config
            .load_external_data()
            .context("loading imported gameplay UI images")?;
        config
            .validate()
            .context("validating imported gameplay UI profile")?;
        Ok((config, report))
    }

    pub fn load_external_data(&mut self) -> Result<()> {
        self.loaded_assets.clear();
        self.loaded_asset_paths.clear();
        let pack_path = self.asset_pack.as_ref().map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            }
        });
        let asset_root = self.asset_root.as_ref().map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            }
        });
        let pack = pack_path
            .as_ref()
            .map(crate::presentation_assets::PresentationAssetPack::load)
            .transpose()?;
        let mut glyph_asset_max_dimensions = BTreeMap::<String, u32>::new();
        for font in self.fonts.values() {
            let max_dimension = font.glyph_max_dimension.unwrap_or(128);
            for asset in font.glyph_assets.values() {
                glyph_asset_max_dimensions
                    .entry(asset.clone())
                    .and_modify(|current| *current = (*current).max(max_dimension))
                    .or_insert(max_dimension);
            }
        }
        let glyph_asset_ids: BTreeSet<_> = glyph_asset_max_dimensions.keys().cloned().collect();
        let mut required_asset_ids = glyph_asset_ids.clone();
        collect_ui_element_assets(&self.elements, &mut required_asset_ids);
        for (id, path) in &self.assets {
            if id.trim().is_empty() {
                bail!("gameplay UI asset id cannot be empty");
            }
            let path_text = path.to_string_lossy();
            let resolved = if path_text.starts_with("pack://") {
                let (pack, pack_path) =
                    pack.as_ref().zip(pack_path.as_ref()).with_context(|| {
                        format!(
                            "gameplay UI asset {id:?} uses pack:// but no asset_pack is configured"
                        )
                    })?;
                let resolved = match pack.resolve_reference_with_root(
                    pack_path,
                    &path_text,
                    None,
                    asset_root.as_deref(),
                ) {
                    Ok(path) => path,
                    Err(_error) if !required_asset_ids.contains(id) => continue,
                    Err(error) => return Err(error),
                };
                let ref_id = path_text
                    .strip_prefix("pack://")
                    .unwrap_or_default()
                    .split('@')
                    .next()
                    .unwrap_or_default();
                let entry = pack
                    .assets
                    .iter()
                    .find(|entry| entry.id == ref_id)
                    .with_context(|| format!("asset pack has no asset id {ref_id:?}"))?;
                if required_asset_ids.contains(id)
                    && !matches!(
                        entry.kind,
                        crate::presentation_assets::PresentationAssetKind::Image
                            | crate::presentation_assets::PresentationAssetKind::Sprite
                            | crate::presentation_assets::PresentationAssetKind::Glyph
                    )
                {
                    bail!(
                        "gameplay UI image asset {id:?} references non-image pack asset {ref_id:?}"
                    );
                }
                resolved
            } else if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            };
            self.loaded_asset_paths.insert(id.clone(), resolved.clone());
            // The pack manifest is an asset catalog, not a command to decode
            // every image into RAM. Keep verified paths for manifests/diagnostics
            // and decode pack images only when an element or font uses them.
            if path_text.starts_with("pack://") && !required_asset_ids.contains(id) {
                continue;
            }
            let mut image = image::ImageReader::open(&resolved)
                .map_err(|error| {
                    anyhow::anyhow!("opening gameplay UI asset {}: {error}", resolved.display())
                })?
                .with_guessed_format()
                .map_err(|error| {
                    anyhow::anyhow!(
                        "identifying gameplay UI asset {}: {error}",
                        resolved.display()
                    )
                })?
                .decode()
                .map_err(|error| {
                    anyhow::anyhow!("decoding gameplay UI asset {}: {error}", resolved.display())
                })?
                .to_rgba8();
            let max_dimension = glyph_asset_max_dimensions.get(id).copied().unwrap_or(1024);
            let (source_width, source_height) = image.dimensions();
            if source_width.max(source_height) > max_dimension {
                let scale = f64::from(max_dimension) / f64::from(source_width.max(source_height));
                let target_width = (f64::from(source_width) * scale).round().max(1.0) as u32;
                let target_height = (f64::from(source_height) * scale).round().max(1.0) as u32;
                image = image::imageops::resize(
                    &image,
                    target_width,
                    target_height,
                    image::imageops::FilterType::Triangle,
                );
            }
            let (width, height) = image.dimensions();
            self.loaded_assets.insert(
                id.clone(),
                Arc::new(UiBitmap {
                    width,
                    height,
                    rgba: Arc::new(image.into_raw()),
                }),
            );
        }
        for (name, font) in &self.fonts {
            if name.trim().is_empty()
                || font.glyph_width == 0
                || font.glyph_height == 0
                || font.columns == 0
                || !font.advance.is_finite()
                || font.advance <= 0.0
                || font.glyph_scales.iter().any(|(glyph, scale)| {
                    glyph.chars().count() != 1 || !scale.is_finite() || *scale <= 0.0
                })
                || font.glyph_advances.iter().any(|(glyph, advance)| {
                    glyph.chars().count() != 1 || !advance.is_finite() || *advance <= 0.0
                })
                || font
                    .center_spacing
                    .is_some_and(|spacing| !spacing.is_finite() || spacing <= 0.0)
                || font
                    .glyph_max_dimension
                    .is_some_and(|dimension| !(1..=4096).contains(&dimension))
                || (font.asset.is_empty() && font.glyph_assets.is_empty())
            {
                bail!("gameplay UI font {name:?} has invalid glyph dimensions or advance");
            }
            if !font.asset.is_empty() {
                let Some(atlas) = self.loaded_assets.get(&font.asset) else {
                    bail!(
                        "gameplay UI font {name:?} refers to missing image asset {:?}",
                        font.asset
                    );
                };
                if font.glyph_width > atlas.width
                    || font.glyph_height > atlas.height
                    || font.columns > atlas.width / font.glyph_width
                {
                    bail!("gameplay UI font {name:?} does not fit its image atlas");
                }
            }
            for (glyph, asset) in &font.glyph_assets {
                if glyph.chars().count() != 1 || !self.loaded_assets.contains_key(asset) {
                    bail!(
                        "gameplay UI font {name:?} has an invalid glyph asset mapping for {glyph:?}"
                    );
                }
            }
        }
        self.loaded_events.clear();
        if let Some(path) = &self.events_file {
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            };
            self.loaded_events =
                serde_json::from_slice(&std::fs::read(&resolved).map_err(|error| {
                    anyhow::anyhow!("reading gameplay event log {}: {error}", resolved.display())
                })?)
                .map_err(|error| {
                    anyhow::anyhow!("parsing gameplay event log {}: {error}", resolved.display())
                })?;
        }
        self.loaded_events
            .sort_by(|left, right| left.time.total_cmp(&right.time));
        if let Some(event) = self.loaded_events.iter().find(|event| {
            !event.time.is_finite()
                || event.time < 0.0
                || event.event.trim().is_empty()
                || event.payload.values().any(|value| !value.is_finite())
        }) {
            bail!(
                "gameplay event log contains an invalid event at time {}",
                event.time
            );
        }
        self.external_data_loaded = true;
        Ok(())
    }

    /// Load profile-relative resources once; call `load_external_data` to force a reload.
    pub fn prepare_external_data(&mut self) -> Result<()> {
        if !self.external_data_loaded {
            self.load_external_data()?;
        }
        Ok(())
    }

    pub(crate) fn resolved_scoring_source(&self) -> UiScoringSource {
        match self
            .scoring
            .as_ref()
            .map(|scoring| scoring.source)
            // An omitted optional scoring block is the standard automatic
            // behavior for older and minimal profiles. Explicit Disabled
            // remains available as an opt-out.
            .unwrap_or(UiScoringSource::Automatic)
        {
            UiScoringSource::Automatic if self.events_file.is_some() => UiScoringSource::EventTrack,
            UiScoringSource::Automatic => UiScoringSource::SonoOverlay,
            source => source,
        }
    }

    /// Build the chart-derived score timeline once for this immutable render
    /// request. Unsupported chart schemas disable only simulated HUD data;
    /// normal Watch rendering remains available.
    pub fn prepare_scoring_for_level(&mut self, level: &crate::formats::LevelData) {
        self.loaded_forecast = None;
        self.scoring_status = None;
        self.scoring_diagnostics.clear();
        match self.resolved_scoring_source() {
            UiScoringSource::Disabled => {
                self.scoring_status = Some("Gameplay scoring is disabled.".into());
            }
            UiScoringSource::EventTrack => {
                self.scoring_status = Some(if self.events_file.is_some() {
                    format!(
                        "Gameplay values: Manually loaded event track ({} events loaded).",
                        self.loaded_events.len()
                    )
                } else {
                    "Gameplay values: Event track selected, but no event file is configured.".into()
                });
            }
            UiScoringSource::SonoOverlay => {
                // An absent scoring block uses the canonical default forecast;
                // explicit Disabled remains handled above.
                let default_forecast = crate::sono_overlay_scoring::ForecastConfig::default();
                let forecast_config = self
                    .scoring
                    .as_ref()
                    .map(|scoring| &scoring.forecast)
                    .unwrap_or(&default_forecast);
                match crate::sono_overlay_scoring::simulate(level, forecast_config) {
                    Ok(forecast) => {
                        let rating_source = match forecast.rating_source {
                            crate::sono_overlay_scoring::RatingSource::UserOverride => {
                                "user override"
                            }
                            crate::sono_overlay_scoring::RatingSource::LevelMetadata => {
                                "level metadata"
                            }
                            crate::sono_overlay_scoring::RatingSource::DefaultZero => "default",
                        };
                        let incomplete_count = forecast
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| diagnostic.potentially_scoring)
                            .count();
                        self.scoring_status = Some(if incomplete_count > 0 {
                            format!(
                                "Gameplay values: Auto-generated All Perfect (best effort; {incomplete_count} unrecognized archetype{} skipped). Effective level rating {:.1} ({rating_source}); recognized notes remain active, but the forecast may be incomplete.",
                                if incomplete_count == 1 { "" } else { "s" },
                                forecast.level_rating,
                            )
                        } else {
                            format!(
                                "Gameplay values: Auto-generated All Perfect ({} recognized notes; {:.3} total weight). Effective level rating {:.1} ({rating_source}). This is not recorded player performance.",
                                forecast.supported_notes,
                                forecast.weighted_note_count,
                                forecast.level_rating,
                            )
                        });
                        self.scoring_diagnostics = forecast.diagnostics.clone();
                        self.loaded_forecast = Some(forecast);
                    }
                    Err(error) => {
                        self.scoring_status = Some(format!(
                            "Sono-Overlay forecast unavailable: {error:#}. Rendering will continue without simulated score state."
                        ));
                    }
                }
            }
            UiScoringSource::Automatic => unreachable!("automatic source is always resolved"),
        }
    }

    pub fn scoring_status(&self) -> Option<&str> {
        self.scoring_status.as_deref()
    }

    pub(crate) fn scoring_details(&self) -> Vec<String> {
        self.scoring_diagnostics
            .iter()
            .map(|diagnostic| {
                let examples = diagnostic
                    .entity_indices
                    .iter()
                    .take(8)
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let suffix = if diagnostic.entity_indices.len() > 8 {
                    format!(", … ({} entities total)", diagnostic.entity_indices.len())
                } else {
                    String::new()
                };
                format!(
                    "{}: skipped {} entit{}{}{}",
                    diagnostic.archetype,
                    examples,
                    if diagnostic.entity_indices.len() == 1 {
                        "y"
                    } else {
                        "ies"
                    },
                    suffix,
                    if diagnostic.potentially_scoring {
                        " — may affect the score forecast"
                    } else {
                        " — treated as non-scoring"
                    },
                )
            })
            .collect()
    }

    fn gameplay_state_from_forecast(
        forecast: &crate::sono_overlay_scoring::Forecast,
        time: f64,
    ) -> Option<UiGameplayState> {
        let frame = forecast.state_at(time)?;
        let mut variables = BTreeMap::from([
            ("score_progress".into(), frame.score_progress),
            ("rank_code".into(), frame.rank_code),
            ("score_delta".into(), frame.score_delta),
            // The reference HUD's transient score-award element is keyed by
            // the per-event score offset. Keep the profile's public name
            // while sourcing it from the forecast's authoritative delta.
            ("score_bonus".into(), frame.score_delta),
            ("note_weight".into(), frame.note_weight),
            (
                "skill_active".into(),
                if frame.skill_active { 1.0 } else { 0.0 },
            ),
        ]);
        if let Some(activation) = frame.skill_activation_time {
            variables.insert("skill_activation_time".into(), activation);
        }
        if let Some(source_entity) = frame.source_entity {
            variables.insert("source_entity".into(), source_entity as f64);
        }
        if let Some(event_time) = frame.combo_event_time {
            variables.insert("combo_event_time".into(), event_time);
        }
        if let Some(event_time) = frame.previous_combo_event_time {
            variables.insert("previous_combo_event_time".into(), event_time);
        }
        variables.insert("effective_level_rating".into(), forecast.level_rating);
        Some(UiGameplayState {
            score: frame.score,
            combo: frame.combo,
            life: frame.life,
            judgment: frame.judgment.clone(),
            latest_event_time: frame.source_entity.map(|_| frame.time),
            judgment_time: frame.judgment_time,
            event: frame.source_entity.map(|_| {
                if frame.skill_activation {
                    "skill-activation".into()
                } else {
                    "note".into()
                }
            }),
            note_type: frame.archetype.clone(),
            variables,
        })
    }

    pub fn gameplay_state_at(&self, time: f64) -> Result<Option<UiGameplayState>> {
        self.gameplay_state_at_with_frame_rate(time, 30.0)
    }

    pub fn gameplay_state_at_with_frame_rate(
        &self,
        time: f64,
        frame_rate: f64,
    ) -> Result<Option<UiGameplayState>> {
        if !time.is_finite() || time < 0.0 {
            bail!("gameplay state time must be finite and nonnegative");
        }
        let mut state = match self.resolved_scoring_source() {
            UiScoringSource::Disabled => None,
            UiScoringSource::SonoOverlay => self
                .loaded_forecast
                .as_ref()
                .and_then(|forecast| Self::gameplay_state_from_forecast(forecast, time)),
            UiScoringSource::EventTrack => {
                let (Some(scoring), Some(_events_path)) = (&self.scoring, &self.events_file) else {
                    return Ok(None);
                };
                let mut state = UiGameplayState::from_config(scoring);
                for event in self
                    .loaded_events
                    .iter()
                    .take_while(|event| event.time <= time)
                {
                    state.apply(scoring, event)?;
                }
                Some(state)
            }
            UiScoringSource::Automatic => unreachable!("automatic source is always resolved"),
        };
        if let Some(state) = &mut state {
            apply_note_hud_animations(state, time, frame_rate);
        }
        Ok(state)
    }

    /// Resolve authored timeline tracks over an input frame without changing
    /// the runtime or gameplay state supplied by Watch/event adapters.
    pub fn resolve_presentation_state(&self, input: &UiFrameState) -> Result<UiFrameState> {
        if !input.time.is_finite() {
            bail!("presentation timeline time must be finite");
        }
        let mut resolved = input.clone();
        for track in &self.timeline.tracks {
            resolved.presentation_values.remove(&track.id);
            resolved.presentation_categories.remove(&track.id);
            match &track.kind {
                UiTimelineTrackKind::Numeric { keyframes } => {
                    if let Some(value) = evaluate_numeric_track(keyframes, input.time) {
                        resolved.presentation_values.insert(track.id.clone(), value);
                    }
                }
                UiTimelineTrackKind::Categorical { keyframes } => {
                    if let Some(value) = evaluate_categorical_track(keyframes, input.time) {
                        resolved
                            .presentation_categories
                            .insert(track.id.clone(), value.to_owned());
                    }
                }
            }
        }
        Ok(resolved)
    }

    pub fn validate(&self) -> Result<()> {
        if self.design_size[0] == 0 || self.design_size[1] == 0 {
            bail!("gameplay UI design size must be nonzero");
        }
        for (name, layout) in &self.responsive_layouts {
            if name.trim().is_empty()
                || layout.reference_size.contains(&0)
                || layout.alternate_size.contains(&0)
                || layout.groups.iter().any(|(group_name, group)| {
                    group_name.trim().is_empty()
                        || group
                            .reference_center
                            .iter()
                            .any(|value| !value.is_finite())
                        || group
                            .alternate_center
                            .iter()
                            .any(|value| !value.is_finite())
                        || !group.alternate_scale.is_finite()
                        || group.alternate_scale <= 0.0
                })
            {
                bail!("gameplay UI responsive layout {name:?} is invalid");
            }
        }
        if self.timeline.tracks.len() > 4096 {
            bail!("gameplay UI timeline exceeds the 4096-track limit");
        }
        let mut timeline_ids = BTreeSet::new();
        let mut keyframe_count = 0usize;
        for track in &self.timeline.tracks {
            if track.id.trim().is_empty() || !timeline_ids.insert(track.id.as_str()) {
                bail!(
                    "gameplay UI timeline track ids must be nonempty and unique: {:?}",
                    track.id
                );
            }
            let mut validate_time = |time: f64, previous: Option<f64>| -> Result<()> {
                keyframe_count = keyframe_count.saturating_add(1);
                if !time.is_finite()
                    || time < 0.0
                    || previous.is_some_and(|previous| time <= previous)
                {
                    bail!("timeline track {:?} keyframes must have finite, strictly increasing nonnegative times", track.id);
                }
                if keyframe_count > 100_000 {
                    bail!("gameplay UI timeline exceeds the 100000-keyframe limit");
                }
                Ok(())
            };
            match &track.kind {
                UiTimelineTrackKind::Numeric { keyframes } => {
                    if keyframes.is_empty() {
                        bail!("numeric timeline track {:?} has no keyframes", track.id);
                    }
                    let mut previous = None;
                    for keyframe in keyframes {
                        validate_time(keyframe.time, previous)?;
                        previous = Some(keyframe.time);
                        if !keyframe.value.is_finite() || keyframe.value.abs() > 1.0e12 {
                            bail!(
                                "timeline track {:?} has a non-finite or unsupported value",
                                track.id
                            );
                        }
                        validate_easing(&keyframe.easing)?;
                    }
                }
                UiTimelineTrackKind::Categorical { keyframes } => {
                    if keyframes.is_empty() {
                        bail!("categorical timeline track {:?} has no keyframes", track.id);
                    }
                    let mut previous = None;
                    for keyframe in keyframes {
                        validate_time(keyframe.time, previous)?;
                        previous = Some(keyframe.time);
                        if keyframe.value.len() > 256 {
                            bail!("timeline track {:?} category exceeds 256 bytes", track.id);
                        }
                    }
                }
            }
        }
        for (name, font) in &self.fonts {
            if name.trim().is_empty()
                || font.glyph_width == 0
                || font.glyph_height == 0
                || font.columns == 0
                || !font.advance.is_finite()
                || font.advance <= 0.0
                || font.glyph_scales.iter().any(|(glyph, scale)| {
                    glyph.chars().count() != 1 || !scale.is_finite() || *scale <= 0.0
                })
                || font.glyph_advances.iter().any(|(glyph, advance)| {
                    glyph.chars().count() != 1 || !advance.is_finite() || *advance <= 0.0
                })
                || font
                    .center_spacing
                    .is_some_and(|spacing| !spacing.is_finite() || spacing <= 0.0)
                || font
                    .glyph_max_dimension
                    .is_some_and(|dimension| !(1..=4096).contains(&dimension))
                || (font.asset.is_empty() && font.glyph_assets.is_empty())
                || (!font.asset.is_empty() && !self.assets.contains_key(&font.asset))
                || font.glyph_assets.iter().any(|(glyph, asset)| {
                    glyph.chars().count() != 1 || !self.assets.contains_key(asset)
                })
            {
                bail!("gameplay UI font {name:?} has invalid atlas settings");
            }
        }
        for (name, metric) in [
            ("primary metric", &self.primary_metric),
            ("secondary metric", &self.secondary_metric),
        ] {
            if let Some(maximum) = metric.maximum {
                if !maximum.is_finite() || maximum <= 0.0 {
                    bail!("{name} maximum must be finite and positive");
                }
            }
            validate_provider(name, &metric.provider)?;
        }
        validate_provider("combo", &self.combo.provider)?;
        validate_provider("judgment", &self.judgment.provider)?;
        let mut ids = BTreeSet::new();
        for element in &self.elements {
            validate_element(element, 0, &mut ids, &self)?;
        }
        for (name, expression) in &self.variables {
            if name.trim().is_empty() {
                bail!("gameplay UI variable name cannot be empty");
            }
            validate_expression(expression)?;
        }
        if let Some(scoring) = &self.scoring {
            if !scoring.initial_score.is_finite()
                || !scoring.initial_life.is_finite()
                || !scoring.minimum_life.is_finite()
                || !scoring.maximum_life.is_finite()
                || scoring.minimum_life > scoring.maximum_life
            {
                bail!("gameplay scoring has invalid initial values or life bounds");
            }
            let forecast = &scoring.forecast;
            if !forecast.team_power.is_finite()
                || forecast.team_power < 0.0
                || forecast
                    .level_rating
                    .is_some_and(|rating| !rating.is_finite())
                || !forecast.initial_life.is_finite()
                || !forecast.minimum_life.is_finite()
                || !forecast.maximum_life.is_finite()
                || forecast.minimum_life > forecast.maximum_life
                || forecast
                    .judgment
                    .as_deref()
                    .is_some_and(|judgment| judgment != "PERFECT")
            {
                bail!("Sono-Overlay forecast settings are invalid; supported forecast judgment is PERFECT or disabled");
            }
            for rule in &scoring.rules {
                if rule.event.trim().is_empty()
                    || !rule.score.is_finite()
                    || !rule.life.is_finite()
                    || rule.variables.values().any(|value| !value.is_finite())
                {
                    bail!("gameplay scoring rule has an empty event or non-finite value");
                }
            }
        }
        Ok(())
    }
}

fn validate_easing(easing: &UiEasing) -> Result<()> {
    if let UiEasing::CubicBezier { x1, y1, x2, y2 } = easing {
        if *x1 > *x2 || [*x1, *y1, *x2, *y2].into_iter().any(|value| value > 10_000) {
            bail!("cubic Bezier easing control points must be ordered and in 0..=10000");
        }
    }
    Ok(())
}

fn evaluate_numeric_track(keyframes: &[UiNumericKeyframe], time: f64) -> Option<f64> {
    let first = keyframes.first()?;
    if time < first.time {
        return None;
    }
    let next = keyframes.partition_point(|keyframe| keyframe.time <= time);
    let left = &keyframes[next.saturating_sub(1)];
    let Some(right) = keyframes.get(next) else {
        return Some(left.value);
    };
    let progress = (time - left.time) / (right.time - left.time);
    let progress = apply_easing(&left.easing, progress);
    Some(left.value + (right.value - left.value) * progress)
}

fn evaluate_categorical_track(keyframes: &[UiCategoricalKeyframe], time: f64) -> Option<&str> {
    let last = keyframes.partition_point(|keyframe| keyframe.time <= time);
    last.checked_sub(1)
        .and_then(|index| keyframes.get(index))
        .map(|keyframe| keyframe.value.as_str())
}

fn apply_easing(easing: &UiEasing, progress: f64) -> f64 {
    let t = progress.clamp(0.0, 1.0);
    match easing {
        UiEasing::Linear => t,
        UiEasing::EaseIn => t * t,
        UiEasing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        UiEasing::EaseInOut => t * t * (3.0 - 2.0 * t),
        UiEasing::CubicBezier { x1, y1, x2, y2 } => {
            let scale = 1.0 / 10_000.0;
            cubic_bezier_y_for_x(
                t,
                f64::from(*x1) * scale,
                f64::from(*y1) * scale,
                f64::from(*x2) * scale,
                f64::from(*y2) * scale,
            )
        }
    }
}

fn cubic_bezier_component(t: f64, p1: f64, p2: f64) -> f64 {
    let inverse = 1.0 - t;
    3.0 * inverse * inverse * t * p1 + 3.0 * inverse * t * t * p2 + t * t * t
}

fn cubic_bezier_derivative(t: f64, p1: f64, p2: f64) -> f64 {
    3.0 * (1.0 - t).powi(2) * p1 + 6.0 * (1.0 - t) * t * (p2 - p1) + 3.0 * t.powi(2) * (1.0 - p2)
}

fn cubic_bezier_y_for_x(x: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let mut low = 0.0;
    let mut high = 1.0;
    let mut t = x;
    for _ in 0..8 {
        let delta = cubic_bezier_component(t, x1, x2) - x;
        if delta.abs() < 1e-8 {
            break;
        }
        let derivative = cubic_bezier_derivative(t, x1, x2);
        if derivative.abs() < 1e-8 {
            break;
        }
        let candidate = t - delta / derivative;
        if !(low..=high).contains(&candidate) {
            break;
        }
        t = candidate;
    }
    for _ in 0..24 {
        let current = cubic_bezier_component(t, x1, x2);
        if (current - x).abs() < 1e-8 {
            break;
        }
        if current < x {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) * 0.5;
    }
    cubic_bezier_component(t, y1, y2)
}

fn validate_element(
    element: &UiElement,
    depth: usize,
    ids: &mut BTreeSet<String>,
    config: &RendererUiConfig,
) -> Result<()> {
    if depth > 32 {
        bail!("gameplay UI element nesting exceeds 32 levels");
    }
    if element.id.trim().is_empty() || !ids.insert(element.id.clone()) {
        bail!(
            "gameplay UI element ids must be nonempty and unique: {:?}",
            element.id
        );
    }
    if element.responsive_group.as_ref().is_some_and(|group| {
        !config
            .responsive_layouts
            .values()
            .any(|layout| layout.groups.contains_key(group))
    }) {
        bail!(
            "gameplay UI element {:?} references a missing responsive layout group",
            element.id
        );
    }
    let layout = &element.layout;
    if [
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        layout.rotation,
    ]
    .into_iter()
    .chain(layout.scale)
    .chain(layout.margin)
    .chain(layout.padding)
    .any(|value| !value.is_finite())
        || layout.width < 0.0
        || layout.height < 0.0
        || layout.scale.iter().any(|value| *value < 0.0)
        || [layout.x, layout.y, layout.width, layout.height]
            .iter()
            .any(|value| value.abs() > 1.0e6)
        || layout.scale.iter().any(|value| *value > 1.0e4)
        || layout
            .margin
            .iter()
            .chain(layout.padding.iter())
            .any(|value| value.abs() > 1.0e4)
        || !element.opacity.is_finite()
        || !(0.0..=1.0).contains(&element.opacity)
    {
        bail!(
            "gameplay UI element {:?} has invalid layout or opacity",
            element.id
        );
    }
    if let Some(flow) = &layout.flow {
        if !flow.gap.is_finite() || flow.gap < 0.0 {
            bail!("gameplay UI element {:?} has invalid flow gap", element.id);
        }
    }
    match &element.kind {
        UiElementKind::Text {
            value,
            font_size,
            font,
            ..
        } => {
            if !font_size.is_finite() || *font_size <= 0.0 {
                bail!("gameplay UI text {:?} has invalid font size", element.id);
            }
            if let Some(value) = value {
                validate_expression(value)?;
            }
            if font
                .as_ref()
                .is_some_and(|font| !config.fonts.contains_key(font))
            {
                bail!(
                    "gameplay UI text {:?} refers to an unknown font",
                    element.id
                );
            }
        }
        UiElementKind::Counter {
            value,
            font_size,
            font,
            ..
        } => {
            validate_expression(value)?;
            if !font_size.is_finite() || *font_size <= 0.0 {
                bail!("gameplay UI counter {:?} has invalid font size", element.id);
            }
            if font
                .as_ref()
                .is_some_and(|font| !config.fonts.contains_key(font))
            {
                bail!(
                    "gameplay UI counter {:?} refers to an unknown font",
                    element.id
                );
            }
        }
        UiElementKind::Gauge {
            value,
            maximum,
            fill_asset,
            fill_source,
            mask_asset,
            ..
        }
        | UiElementKind::ProgressBar {
            value,
            maximum,
            fill_asset,
            fill_source,
            mask_asset,
            ..
        } => {
            validate_expression(value)?;
            validate_expression(maximum)?;
            if fill_asset
                .as_ref()
                .is_some_and(|asset| !config.assets.contains_key(asset))
                || mask_asset
                    .as_ref()
                    .is_some_and(|asset| !config.assets.contains_key(asset))
                || (mask_asset.is_some() && fill_asset.is_none())
            {
                bail!(
                    "gameplay UI gauge {:?} refers to an unknown fill or mask asset",
                    element.id
                );
            }
            if fill_source.is_some_and(|source| {
                source
                    .iter()
                    .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
                    || source[2] <= 0.0
                    || source[3] <= 0.0
                    || source[0] + source[2] > 1.0 + f64::EPSILON
                    || source[1] + source[3] > 1.0 + f64::EPSILON
            }) {
                bail!(
                    "gameplay UI gauge {:?} has an invalid fill source region",
                    element.id
                );
            }
            if let (Some(fill), Some(mask)) = (
                fill_asset
                    .as_ref()
                    .and_then(|asset| config.loaded_assets.get(asset)),
                mask_asset
                    .as_ref()
                    .and_then(|asset| config.loaded_assets.get(asset)),
            ) {
                if fill.width != mask.width || fill.height != mask.height {
                    bail!(
                        "gameplay UI gauge {:?} fill and mask dimensions differ",
                        element.id
                    );
                }
            }
        }
        UiElementKind::Image { asset, .. } | UiElementKind::Icon { asset, .. } => {
            if !config.assets.contains_key(asset) {
                bail!(
                    "gameplay UI element {:?} refers to unknown asset {asset:?}",
                    element.id
                );
            }
        }
        UiElementKind::Sprite { asset, source, .. } => {
            if !config.assets.contains_key(asset) {
                bail!(
                    "gameplay UI element {:?} refers to unknown asset {asset:?}",
                    element.id
                );
            }
            if source[2] == 0
                || source[3] == 0
                || source[0].checked_add(source[2]).is_none()
                || source[1].checked_add(source[3]).is_none()
            {
                bail!(
                    "gameplay UI sprite {:?} has invalid source bounds",
                    element.id
                );
            }
            if let Some(bitmap) = config.loaded_assets.get(asset) {
                if source[0] + source[2] > bitmap.width || source[1] + source[3] > bitmap.height {
                    bail!(
                        "gameplay UI sprite {:?} exceeds its image atlas",
                        element.id
                    );
                }
            }
        }
        UiElementKind::Rectangle { radius, .. } if !radius.is_finite() || *radius < 0.0 => {
            bail!(
                "gameplay UI rectangle {:?} has invalid corner radius",
                element.id
            );
        }
        _ => {}
    }
    if let Some(condition) = &element.visible_when {
        validate_condition(condition)?;
    }
    for style in &element.style_rules {
        validate_condition(&style.when)?;
        if style
            .asset
            .as_ref()
            .is_some_and(|asset| !config.assets.contains_key(asset))
        {
            bail!(
                "gameplay UI element {:?} style refers to an unknown asset",
                element.id
            );
        }
        if style
            .opacity
            .is_some_and(|opacity| !opacity.is_finite() || !(0.0..=1.0).contains(&opacity))
        {
            bail!(
                "gameplay UI element {:?} has an invalid style opacity",
                element.id
            );
        }
        if style
            .asset
            .as_ref()
            .is_some_and(|asset| !config.assets.contains_key(asset))
        {
            bail!(
                "gameplay UI element {:?} style refers to an unknown asset",
                element.id
            );
        }
    }
    for animation in &element.animations {
        if !animation.from.is_finite()
            || !animation.to.is_finite()
            || !animation.start.is_finite()
            || !animation.duration.is_finite()
            || animation.duration <= 0.0
            || animation.from.abs() > 1.0e6
            || animation.to.abs() > 1.0e6
            || animation.start.abs() > 1.0e9
            || animation.duration > 1.0e9
        {
            bail!(
                "gameplay UI element {:?} has an invalid animation",
                element.id
            );
        }
        validate_easing(&animation.easing)?;
        if let Some(expression) = &animation.value_from_state {
            validate_expression(expression)?;
        }
        if let Some(condition) = &animation.when {
            validate_condition(condition)?;
        }
    }
    for style in &element.style_rules {
        validate_condition(&style.when)?;
        if style
            .opacity
            .is_some_and(|opacity| !opacity.is_finite() || !(0.0..=1.0).contains(&opacity))
        {
            bail!(
                "gameplay UI element {:?} has an invalid style opacity",
                element.id
            );
        }
        if style
            .asset
            .as_ref()
            .is_some_and(|asset| !config.assets.contains_key(asset))
        {
            bail!(
                "gameplay UI element {:?} style refers to an unknown asset",
                element.id
            );
        }
    }
    for child in &element.children {
        validate_element(child, depth + 1, ids, config)?;
    }
    Ok(())
}

fn validate_expression(expression: &UiExpression) -> Result<()> {
    match expression {
        UiExpression::Constant { value } if !value.is_finite() => {
            bail!("gameplay UI expression constant must be finite")
        }
        UiExpression::Variable { name } if name.trim().is_empty() => {
            bail!("gameplay UI expression variable name cannot be empty")
        }
        UiExpression::Source {
            source:
                UiStateSource::GameplayVariable { name } | UiStateSource::PresentationValue { name },
        } if name.trim().is_empty() => bail!("gameplay UI state variable name cannot be empty"),
        UiExpression::Add { left, right }
        | UiExpression::Subtract { left, right }
        | UiExpression::Multiply { left, right }
        | UiExpression::Divide { left, right }
        | UiExpression::Min { left, right }
        | UiExpression::Max { left, right } => {
            validate_expression(left)?;
            validate_expression(right)?;
        }
        UiExpression::Clamp {
            value,
            minimum,
            maximum,
        } => {
            if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
                bail!("gameplay UI clamp has invalid bounds");
            }
            validate_expression(value)?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_condition(condition: &UiCondition) -> Result<()> {
    match condition {
        UiCondition::Truthy { value } => validate_expression(value),
        UiCondition::Greater { left, right }
        | UiCondition::GreaterOrEqual { left, right }
        | UiCondition::Less { left, right }
        | UiCondition::LessOrEqual { left, right }
        | UiCondition::Equal { left, right }
        | UiCondition::NotEqual { left, right } => {
            validate_expression(left)?;
            validate_expression(right)
        }
        UiCondition::StringEquals { source, value } => {
            if value.len() > 256 {
                bail!("gameplay UI string condition exceeds 256 bytes");
            }
            if let UiStringSource::PresentationCategory { name } = source {
                if name.trim().is_empty() {
                    bail!("presentation category name cannot be empty");
                }
            }
            Ok(())
        }
        UiCondition::All { conditions } | UiCondition::Any { conditions } => {
            for condition in conditions {
                validate_condition(condition)?;
            }
            Ok(())
        }
        UiCondition::Not { condition } => validate_condition(condition),
    }
}

fn validate_provider(name: &str, provider: &UiValueProvider) -> Result<()> {
    match provider {
        UiValueProvider::Fixed { value } | UiValueProvider::External { value }
            if !value.is_finite() =>
        {
            bail!("{name} provider value must be finite")
        }
        UiValueProvider::EngineMetric { key } if key.trim().is_empty() => {
            bail!("{name} engine metric key cannot be empty")
        }
        UiValueProvider::JudgmentDerived { mapping } if mapping.trim().is_empty() => {
            bail!("{name} judgment mapping cannot be empty")
        }
        _ => {}
    }
    Ok(())
}

pub fn render_overlay(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    time: f64,
    segment_start: f64,
    segment_duration: f64,
    config: &RendererUiConfig,
    mut value: impl FnMut(&UiValueProvider) -> Result<f64>,
) -> Result<()> {
    config.validate()?;
    if !config.enabled {
        return Ok(());
    }
    if rgb.len() != width as usize * height as usize * 3 {
        bail!("UI overlay frame buffer dimensions do not match RGB data");
    }
    let w = width as i32;
    let h = height as i32;
    let margin = (w.min(h) / 40).clamp(6, 24);
    let panel_w = (w / 3).max(150).min(w - margin * 2);
    let row_h = 34;
    if config.primary_metric.enabled {
        draw_value_panel(
            rgb,
            w,
            h,
            margin,
            margin,
            panel_w,
            row_h,
            &config.primary_metric.label,
            value(&config.primary_metric.provider)?,
            config.primary_metric.maximum,
            &config.theme,
        );
    }
    if config.secondary_metric.enabled {
        draw_value_panel(
            rgb,
            w,
            h,
            margin,
            margin + row_h + 5,
            panel_w,
            row_h,
            &config.secondary_metric.label,
            value(&config.secondary_metric.provider)?,
            config.secondary_metric.maximum,
            &config.theme,
        );
    }
    if config.combo.enabled {
        let panel_x = w - panel_w - margin;
        draw_value_panel(
            rgb,
            w,
            h,
            panel_x,
            margin,
            panel_w,
            row_h,
            &config.combo.label,
            value(&config.combo.provider)?,
            None,
            &config.theme,
        );
    }
    if config.judgment.enabled {
        let panel_x = w - panel_w - margin;
        draw_value_panel(
            rgb,
            w,
            h,
            panel_x,
            margin + row_h + 5,
            panel_w,
            row_h,
            &config.judgment.label,
            value(&config.judgment.provider)?,
            None,
            &config.theme,
        );
    }
    if config.progress {
        let x = margin;
        let y = h - margin - 10;
        let track_w = w - margin * 2;
        blend_rect(
            rgb,
            w,
            h,
            x,
            y,
            track_w,
            8,
            config.theme.progress_track,
            220,
        );
        let progress = if segment_duration > 0.0 {
            ((time - segment_start) / segment_duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        blend_rect(
            rgb,
            w,
            h,
            x,
            y,
            (f64::from(track_w) * progress).round() as i32,
            8,
            config.theme.accent,
            255,
        );
    }
    Ok(())
}

pub fn render_gameplay_ui(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> Result<()> {
    config.validate()?;
    match config.mode {
        GameplayUiMode::Off => Ok(()),
        GameplayUiMode::Legacy => {
            render_overlay(
                rgb,
                width,
                height,
                state.time,
                0.0,
                1.0,
                config,
                |provider| match provider {
                    UiValueProvider::Fixed { value } | UiValueProvider::External { value } => {
                        Ok(*value)
                    }
                    UiValueProvider::Memory { block_id, index } => Ok(state
                        .memory
                        .as_ref()
                        .map_or(0.0, |memory| memory.get(*block_id, *index))),
                    UiValueProvider::Progress => Ok(state.progress),
                    UiValueProvider::EngineMetric { key } => {
                        bail!("engine metric provider {key:?} has no authoritative Watch source")
                    }
                    UiValueProvider::Accuracy => {
                        bail!("accuracy provider is unavailable without a verified gameplay event source")
                    }
                    UiValueProvider::JudgmentDerived { mapping } => {
                        bail!("judgment mapping {mapping:?} is not configured by the legacy UI")
                    }
                },
            )
        }
        GameplayUiMode::Custom => {
            if !config.enabled {
                return Ok(());
            }
            if config.elements.is_empty() && !config.intentionally_empty {
                // Old/partially initialized custom profiles must never turn an
                // enabled Gameplay UI into a silent no-op. Keep this runtime
                // recovery non-destructive: saving still preserves the source
                // document until the user explicitly populates it.
                let mut fallback = config.clone();
                fallback.design_size = [width.max(1), height.max(1)];
                fallback.fit = UiFitMode::Stretch;
                fallback.elements = default_gameplay_ui_elements(width, height, None, None, None);
                fallback.intentionally_empty = false;
                let resolved = fallback.resolve_presentation_state(state)?;
                return render_custom_ui(rgb, width, height, &fallback, &resolved);
            }
            let resolved = config.resolve_presentation_state(state)?;
            render_custom_ui(rgb, width, height, config, &resolved)
        }
    }
}

fn render_custom_ui(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> Result<()> {
    if !config.enabled || config.elements.is_empty() {
        return Ok(());
    }
    if rgb.len() != width as usize * height as usize * 3 {
        bail!("gameplay UI frame buffer dimensions do not match RGB data");
    }
    let design_w = f64::from(config.design_size[0]);
    let design_h = f64::from(config.design_size[1]);
    let output_w = f64::from(width);
    let output_h = f64::from(height);
    let root = ui_root_matrix(output_w, output_h, config.design_size, config.fit);
    let screen = UiRect {
        x: 0.0,
        y: 0.0,
        width: design_w,
        height: design_h,
    };
    let mut ordered: Vec<_> = config.elements.iter().enumerate().collect();
    ordered.sort_by_key(|(_, element)| element.z_index);
    let mut rendered_isolation_groups = std::collections::BTreeSet::new();
    for (_, element) in ordered {
        if let Some(group_name) = element.isolation_group.as_deref() {
            let group_last_z = config
                .elements
                .iter()
                .filter(|candidate| candidate.isolation_group.as_deref() == Some(group_name))
                .map(|candidate| candidate.z_index)
                .max()
                .unwrap_or(element.z_index);
            if element.z_index != group_last_z {
                continue;
            }
            if !rendered_isolation_groups.insert(group_name.to_owned()) {
                continue;
            }
            let mut group: Vec<_> = config
                .elements
                .iter()
                .filter(|candidate| candidate.isolation_group.as_deref() == Some(group_name))
                .collect();
            group.sort_by_key(|candidate| candidate.z_index);
            render_isolated_ui_group(rgb, width, height, config, state, &group, screen, root)?;
            continue;
        }
        let element_root = element.responsive_group.as_deref().map_or(root, |group| {
            responsive_group_matrix(config, group, output_w, output_h, root)
        });
        render_element(
            rgb,
            width,
            height,
            config,
            state,
            element,
            screen,
            element_root,
            None,
            0,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_isolated_ui_group(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &RendererUiConfig,
    state: &UiFrameState,
    elements: &[&UiElement],
    screen: UiRect,
    root: UiMatrix,
) -> Result<()> {
    let mut bounds = None;
    for element in elements {
        let element_root = element.responsive_group.as_deref().map_or(root, |group| {
            responsive_group_matrix(config, group, f64::from(width), f64::from(height), root)
        });
        collect_element_output_bounds(
            element,
            screen,
            element_root,
            config,
            state,
            width,
            height,
            &mut bounds,
            0,
        );
    }
    let Some((left, top, right, bottom)) = bounds else {
        return Ok(());
    };
    let isolated_width = (right - left) as u32;
    let isolated_height = (bottom - top) as u32;
    let mut isolated_rgb = vec![0; isolated_width as usize * isolated_height as usize * 3];
    let mut isolated_alpha = vec![0; isolated_width as usize * isolated_height as usize];
    for element in elements {
        let mut element_root = element.responsive_group.as_deref().map_or(root, |group| {
            responsive_group_matrix(config, group, f64::from(width), f64::from(height), root)
        });
        element_root.tx -= f64::from(left);
        element_root.ty -= f64::from(top);
        render_element(
            &mut isolated_rgb,
            isolated_width,
            isolated_height,
            config,
            state,
            element,
            screen,
            element_root,
            Some(&mut isolated_alpha),
            0,
        )?;
    }
    for (index, (pixel, alpha)) in isolated_rgb
        .as_chunks::<3>()
        .0
        .iter()
        .zip(isolated_alpha.iter().copied())
        .enumerate()
    {
        if alpha == 0 {
            continue;
        }
        let local_x = index % isolated_width as usize;
        let local_y = index / isolated_width as usize;
        let target_x = left as usize + local_x;
        let target_y = top as usize + local_y;
        let offset = (target_y * width as usize + target_x) * 3;
        let inverse_alpha = 255_u16 - u16::from(alpha);
        for channel in 0..3 {
            rgb[offset + channel] = (u16::from(pixel[channel])
                + (u16::from(rgb[offset + channel]) * inverse_alpha + 127) / 255)
                .min(255) as u8;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_element_output_bounds(
    element: &UiElement,
    parent: UiRect,
    parent_matrix: UiMatrix,
    config: &RendererUiConfig,
    state: &UiFrameState,
    width: u32,
    height: u32,
    output_bounds: &mut Option<(i32, i32, i32, i32)>,
    depth: usize,
) {
    if depth > 32
        || !element.enabled
        || element
            .visible_when
            .as_ref()
            .is_some_and(|condition| !evaluate_condition(condition, config, state))
    {
        return;
    }
    let style_opacity = element
        .style_rules
        .iter()
        .filter(|style| evaluate_condition(&style.when, config, state))
        .filter_map(|style| style.opacity)
        .next_back()
        .unwrap_or(1.0);
    let (rect, world) =
        element_geometry(element, parent, parent_matrix, config, state, style_opacity);
    if let Some((left, top, right, bottom)) = transformed_bounds(world, rect, width, height) {
        *output_bounds = Some(match *output_bounds {
            Some((min_x, min_y, max_x, max_y)) => (
                min_x.min(left),
                min_y.min(top),
                max_x.max(right),
                max_y.max(bottom),
            ),
            None => (left, top, right, bottom),
        });
    }
    let pad = element.layout.padding;
    let child_parent = UiRect {
        x: pad[0] * rect.width,
        y: pad[1] * rect.height,
        width: (1.0 - pad[0] - pad[2]).max(0.0) * rect.width,
        height: (1.0 - pad[1] - pad[3]).max(0.0) * rect.height,
    };
    let mut children: Vec<_> = element.children.iter().enumerate().collect();
    children.sort_by_key(|(_, child)| child.z_index);
    let mut flow_cursor = 0.0;
    for (_, child) in children {
        let mut child = child.clone();
        if let Some(flow) = &element.layout.flow {
            match flow.direction {
                UiFlowDirection::Row => {
                    child.layout.x = flow_cursor;
                    flow_cursor += child.layout.width + flow.gap;
                }
                UiFlowDirection::Column => {
                    child.layout.y = flow_cursor;
                    flow_cursor += child.layout.height + flow.gap;
                }
            }
        }
        collect_element_output_bounds(
            &child,
            child_parent,
            world,
            config,
            state,
            width,
            height,
            output_bounds,
            depth + 1,
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct UiRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy)]
struct UiMatrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
    opacity: f64,
    blend_mode: UiBlendMode,
}

impl UiMatrix {
    fn compose(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            tx: self.a * other.tx + self.c * other.ty + self.tx,
            ty: self.b * other.tx + self.d * other.ty + self.ty,
            opacity: self.opacity * other.opacity,
            blend_mode: if other.blend_mode == UiBlendMode::Normal {
                self.blend_mode
            } else {
                other.blend_mode
            },
        }
    }

    fn inverse_point(self, x: f64, y: f64) -> Option<[f64; 2]> {
        let determinant = self.a * self.d - self.b * self.c;
        if determinant.abs() < f64::EPSILON {
            return None;
        }
        let px = x - self.tx;
        let py = y - self.ty;
        Some([
            (self.d * px - self.c * py) / determinant,
            (-self.b * px + self.a * py) / determinant,
        ])
    }
}

fn element_geometry(
    element: &UiElement,
    parent: UiRect,
    parent_matrix: UiMatrix,
    config: &RendererUiConfig,
    state: &UiFrameState,
    style_opacity: f64,
) -> (UiRect, UiMatrix) {
    let mut bounds = layout_rect(&element.layout, parent);
    let (opacity, offset_x, offset_y, scale_x, scale_y, rotation) =
        animation_values(element, config, state);
    bounds.x += offset_x;
    bounds.y += offset_y;
    let anchor = anchor_fraction(element.layout.anchor);
    let origin_x = bounds.x - bounds.width * anchor[0];
    let origin_y = bounds.y - bounds.height * anchor[1];
    let center_x = bounds.width * 0.5;
    let center_y = bounds.height * 0.5;
    let angle = (element.layout.rotation + rotation).to_radians();
    let cos = angle.cos();
    let sin = angle.sin();
    let local = UiMatrix {
        // Animation scale belongs in the element transform. Scaling only the
        // layout rectangle changes the alignment box, but bitmap-font glyphs
        // are sized from `font_size` and therefore remain visually static.
        // Apply it to the affine matrix around the anchor so text, counters,
        // images, and groups all receive the same actual transform.
        a: cos * element.layout.scale[0] * scale_x,
        b: sin * element.layout.scale[0] * scale_x,
        c: -sin * element.layout.scale[1] * scale_y,
        d: cos * element.layout.scale[1] * scale_y,
        tx: origin_x + center_x - cos * element.layout.scale[0] * scale_x * center_x
            + sin * element.layout.scale[1] * scale_y * center_y,
        ty: origin_y + center_y
            - sin * element.layout.scale[0] * scale_x * center_x
            - cos * element.layout.scale[1] * scale_y * center_y,
        opacity: (element.opacity * opacity * style_opacity).clamp(0.0, 1.0),
        blend_mode: element.blend_mode,
    };
    let world = parent_matrix.compose(local);
    let rect = UiRect {
        x: 0.0,
        y: 0.0,
        width: bounds.width,
        height: bounds.height,
    };
    (rect, world)
}

fn ui_root_matrix(output_w: f64, output_h: f64, design_size: [u32; 2], fit: UiFitMode) -> UiMatrix {
    let design_w = f64::from(design_size[0]);
    let design_h = f64::from(design_size[1]);
    let (scale_x, scale_y) = match fit {
        UiFitMode::Stretch => (output_w / design_w, output_h / design_h),
        UiFitMode::Contain | UiFitMode::Cover => {
            let scale = if fit == UiFitMode::Contain {
                (output_w / design_w).min(output_h / design_h)
            } else {
                (output_w / design_w).max(output_h / design_h)
            };
            (scale, scale)
        }
    };
    UiMatrix {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        tx: (output_w - design_w * scale_x) * 0.5,
        ty: (output_h - design_h * scale_y) * 0.5,
        opacity: 1.0,
        blend_mode: UiBlendMode::Normal,
    }
}

fn responsive_group_matrix(
    config: &RendererUiConfig,
    group_name: &str,
    output_w: f64,
    output_h: f64,
    root: UiMatrix,
) -> UiMatrix {
    let Some(layout) = config.responsive_layouts.get("4:3") else {
        return root;
    };
    let Some(group) = layout.groups.get(group_name) else {
        return root;
    };
    let reference_size = layout.reference_size;
    let alternate_size = layout.alternate_size;
    if reference_size[0] == 0 || reference_size[1] == 0 || alternate_size[0] == 0 {
        return root;
    }

    let reference_aspect = f64::from(reference_size[0]) / f64::from(reference_size[1]);
    let alternate_aspect = f64::from(alternate_size[0]) / f64::from(alternate_size[1]);
    let output_aspect = output_w / output_h.max(1.0);
    let blend = ((reference_aspect - output_aspect) / (reference_aspect - alternate_aspect))
        .clamp(0.0, 1.0);

    let source_design = [
        group.reference_center[0] * f64::from(config.design_size[0]) / f64::from(reference_size[0]),
        group.reference_center[1] * f64::from(config.design_size[1]) / f64::from(reference_size[1]),
    ];
    let source_output = [
        root.a * source_design[0] + root.tx,
        root.d * source_design[1] + root.ty,
    ];
    let source_normalized = [
        group.reference_center[0] / f64::from(reference_size[0]),
        group.reference_center[1] / f64::from(reference_size[1]),
    ];
    let alternate_normalized = [
        group.alternate_center[0] / f64::from(alternate_size[0]),
        group.alternate_center[1] / f64::from(alternate_size[1]),
    ];
    let target = [
        (source_normalized[0] + (alternate_normalized[0] - source_normalized[0]) * blend)
            * output_w,
        (source_normalized[1] + (alternate_normalized[1] - source_normalized[1]) * blend)
            * output_h,
    ];

    // The root contain fit already scales the 1920-wide reference by 0.75
    // at the alternate reference width (1440). Correct only the small
    // authored group-scale difference to avoid applying that width shrink
    // twice. For other output sizes, root scaling remains authoritative.
    let reference_resolution_scale = f64::from(alternate_size[0]) / f64::from(reference_size[0]);
    let alternate_correction = group.alternate_scale / reference_resolution_scale;
    let scale = 1.0 + (alternate_correction - 1.0) * blend;
    UiMatrix {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        tx: target[0] - scale * source_output[0],
        ty: target[1] - scale * source_output[1],
        opacity: 1.0,
        blend_mode: UiBlendMode::Normal,
    }
    .compose(root)
}

#[allow(clippy::too_many_arguments)]
fn render_element(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &RendererUiConfig,
    state: &UiFrameState,
    element: &UiElement,
    parent: UiRect,
    parent_matrix: UiMatrix,
    mut alpha_target: Option<&mut [u8]>,
    depth: usize,
) -> Result<()> {
    if depth > 32 || !element.enabled {
        return Ok(());
    }
    if element
        .visible_when
        .as_ref()
        .is_some_and(|condition| !evaluate_condition(condition, config, state))
    {
        return Ok(());
    }
    let (mut style_color, mut style_border, mut style_tint, mut style_asset, mut style_opacity) =
        (None, None, None, None, 1.0_f64);
    for style in &element.style_rules {
        if evaluate_condition(&style.when, config, state) {
            style_color = style.color.or(style_color);
            style_border = style.border.or(style_border);
            style_tint = style.tint.or(style_tint);
            style_asset = style.asset.as_deref().or(style_asset);
            if let Some(value) = style.opacity {
                style_opacity = value;
            }
        }
    }
    let (rect, world) =
        element_geometry(element, parent, parent_matrix, config, state, style_opacity);
    match &element.kind {
        UiElementKind::Group | UiElementKind::Container => {}
        UiElementKind::Overlay { color } => {
            draw_transformed_rect(
                rgb,
                width,
                height,
                world,
                rect,
                style_color.unwrap_or(*color),
                0.0,
                alpha_target.as_deref_mut(),
            );
        }
        UiElementKind::Rectangle {
            color,
            border,
            radius,
        } => {
            draw_transformed_rect(
                rgb,
                width,
                height,
                world,
                rect,
                style_color.unwrap_or(*color),
                *radius,
                alpha_target.as_deref_mut(),
            );
            if let Some(border) = style_border.or(*border) {
                draw_transformed_border(
                    rgb,
                    width,
                    height,
                    world,
                    rect,
                    border,
                    (*radius).max(0.0),
                    alpha_target.as_deref_mut(),
                );
            }
        }
        UiElementKind::Text {
            text,
            value,
            color,
            font_size,
            font,
            align,
        } => {
            let text = format_template(text, value.as_ref(), config, state);
            draw_ui_text(
                rgb,
                width,
                height,
                world,
                rect,
                &text,
                style_color.unwrap_or(*color),
                *font_size,
                *align,
                font.as_deref(),
                &config.fonts,
                &config.loaded_assets,
                false,
                alpha_target.as_deref_mut(),
            );
        }
        UiElementKind::Counter {
            value,
            prefix,
            suffix,
            digits,
            max_digits,
            color,
            font_size,
            font,
            align,
        } => {
            let text = evaluate_expression(value, config, state)
                .map(|value| {
                    format!(
                        "{prefix}{}{suffix}",
                        format_counter(value, *digits, *max_digits)
                    )
                })
                .unwrap_or_default();
            draw_ui_text(
                rgb,
                width,
                height,
                world,
                rect,
                &text,
                style_color.unwrap_or(*color),
                *font_size,
                *align,
                font.as_deref(),
                &config.fonts,
                &config.loaded_assets,
                false,
                alpha_target.as_deref_mut(),
            );
        }
        UiElementKind::Gauge {
            value,
            maximum,
            orientation,
            fill,
            track,
            fill_asset,
            fill_source,
            mask_asset,
        }
        | UiElementKind::ProgressBar {
            value,
            maximum,
            orientation,
            fill,
            track,
            fill_asset,
            fill_source,
            mask_asset,
        } => {
            draw_gauge(
                rgb,
                width,
                height,
                world,
                rect,
                *track,
                style_color.unwrap_or(*fill),
                *orientation,
                value,
                maximum,
                style_asset.or(fill_asset.as_deref()),
                *fill_source,
                mask_asset.as_deref(),
                config,
                state,
                alpha_target.as_deref_mut(),
            );
        }
        UiElementKind::Image { asset, tint } | UiElementKind::Icon { asset, tint } => {
            let asset = style_asset.unwrap_or(asset.as_str());
            if let Some(bitmap) = config.loaded_assets.get(asset) {
                draw_transformed_image(
                    rgb,
                    width,
                    height,
                    world,
                    rect,
                    bitmap,
                    [0, 0, bitmap.width, bitmap.height],
                    style_tint.or(*tint),
                    alpha_target.as_deref_mut(),
                );
            }
        }
        UiElementKind::Sprite {
            asset,
            source,
            tint,
        } => {
            let asset = style_asset.unwrap_or(asset.as_str());
            if let Some(bitmap) = config.loaded_assets.get(asset) {
                draw_transformed_image(
                    rgb,
                    width,
                    height,
                    world,
                    rect,
                    bitmap,
                    *source,
                    style_tint.or(*tint),
                    alpha_target.as_deref_mut(),
                );
            }
        }
    }
    let pad = element.layout.padding;
    let child_parent = UiRect {
        x: pad[0] * rect.width,
        y: pad[1] * rect.height,
        width: (1.0 - pad[0] - pad[2]).max(0.0) * rect.width,
        height: (1.0 - pad[1] - pad[3]).max(0.0) * rect.height,
    };
    let mut children: Vec<_> = element.children.iter().enumerate().collect();
    children.sort_by_key(|(_, child)| child.z_index);
    let mut flow_cursor = 0.0;
    for (_, child) in children {
        let mut child_layout = child.layout.clone();
        if let Some(flow) = &element.layout.flow {
            match flow.direction {
                UiFlowDirection::Row => {
                    child_layout.x = flow_cursor;
                    flow_cursor += child_layout.width + flow.gap;
                }
                UiFlowDirection::Column => {
                    child_layout.y = flow_cursor;
                    flow_cursor += child_layout.height + flow.gap;
                }
            }
        }
        let mut child = child.clone();
        child.layout = child_layout;
        render_element(
            rgb,
            width,
            height,
            config,
            state,
            &child,
            child_parent,
            world,
            alpha_target.as_deref_mut(),
            depth + 1,
        )?;
    }
    Ok(())
}

fn layout_rect(layout: &UiLayout, parent: UiRect) -> UiRect {
    let margin = layout.margin;
    UiRect {
        x: parent.x + layout.x * parent.width + margin[0] * parent.width,
        y: parent.y + layout.y * parent.height + margin[1] * parent.height,
        width: (layout.width - margin[0] - margin[2]).max(0.0) * parent.width,
        height: (layout.height - margin[1] - margin[3]).max(0.0) * parent.height,
    }
}

fn anchor_fraction(anchor: UiAnchor) -> [f64; 2] {
    match anchor {
        UiAnchor::TopLeft => [0.0, 0.0],
        UiAnchor::Top => [0.5, 0.0],
        UiAnchor::TopRight => [1.0, 0.0],
        UiAnchor::Left => [0.0, 0.5],
        UiAnchor::Center => [0.5, 0.5],
        UiAnchor::Right => [1.0, 0.5],
        UiAnchor::BottomLeft => [0.0, 1.0],
        UiAnchor::Bottom => [0.5, 1.0],
        UiAnchor::BottomRight => [1.0, 1.0],
    }
}

/// Evaluate Sono-Overlay's frame-based combo and judgment animations from
/// event timestamps. `frame_rate` is the output frame rate; no prior GUI frame
/// or accumulated delta time is needed, so direct seeks are deterministic.
pub(crate) fn apply_note_hud_animations(state: &mut UiGameplayState, time: f64, frame_rate: f64) {
    let fps = if frame_rate.is_finite() && frame_rate > 0.0 {
        frame_rate
    } else {
        30.0
    };
    let frame_delta = |event_time: f64| {
        let progress = (time - event_time) * fps;
        let nearest_frame = progress.round();
        if (progress - nearest_frame).abs() <= 1.0e-9 {
            nearest_frame
        } else {
            progress
        }
    };
    // sekai.obj2 pulses the AP title glow and shadow glyphs continuously,
    // independently of note events: min(1, (sin(time*pi*4/3)+1)/2).
    let ap_combo_alpha = ((time * std::f64::consts::PI * (4.0 / 3.0)).sin() + 1.0) * 0.5;
    state
        .variables
        .insert("ap_combo_alpha".into(), ap_combo_alpha.clamp(0.0, 1.0));
    // sekai.obj2 pulses only the overflow segment when life exceeds 1000.
    // The ordinary 0..1000 gauge is intentionally static.
    let life_overflow_alpha = ((time * std::f64::consts::PI).sin() + 1.0) * 0.5;
    state.variables.insert(
        "life_overflow_alpha".into(),
        life_overflow_alpha.clamp(0.0, 1.0),
    );
    let latest_note = state
        .variables
        .get("combo_event_time")
        .copied()
        .or(state.latest_event_time);
    if let Some(event_time) = latest_note {
        let mut progress = frame_delta(event_time);
        if progress < 1.0 {
            if let Some(previous_time) = state.variables.get("previous_combo_event_time") {
                progress = frame_delta(*previous_time);
            }
        }
        state.variables.insert(
            "combo_pop_scale".into(),
            (0.5 + progress / 16.0).min(1.0).max(0.0),
        );
        state
            .variables
            .insert("combo_pulse_scale".into(), 0.5 + progress / 16.0);
        state.variables.insert(
            "combo_pulse_opacity".into(),
            (1.0 - progress / 14.0).clamp(0.0, 1.0),
        );
        state.variables.insert(
            "combo_pulse_visible".into(),
            if progress < 14.0 { 1.0 } else { 0.0 },
        );
    } else {
        state.variables.insert("combo_pop_scale".into(), 1.0);
        state.variables.insert("combo_pulse_scale".into(), 1.0);
        state.variables.insert("combo_pulse_opacity".into(), 0.0);
        state.variables.insert("combo_pulse_visible".into(), 0.0);
    }
    state.variables.insert(
        "combo_shadow_pulse_opacity".into(),
        (state
            .variables
            .get("combo_pulse_opacity")
            .copied()
            .unwrap_or(0.0)
            * state.variables["ap_combo_alpha"])
            .clamp(0.0, 1.0),
    );
    let judgment_progress = state.judgment_time.map(frame_delta);
    // In sekai.obj2, the fourth `obj.draw` argument is SCALE, not opacity.
    // The judgment sprite is drawn at its 2/3 reference size with alpha zero
    // before frame 2; it then grows according to the fourth-power curve until
    // frame 5, stays at 2/3 scale through frame 19, and is no longer drawn at
    // frame 20. The image's opacity is 1 during its visible interval.
    let judgment_scale = judgment_progress.map_or(0.0, |progress| {
        if progress < 2.0 {
            1.0
        } else if progress < 5.0 {
            1.0 - (-1.45 + progress / 4.0).powi(4)
        } else if progress < 20.0 {
            1.0
        } else {
            0.0
        }
    });
    let judgment_visible =
        judgment_progress.is_some_and(|progress| (2.0..20.0).contains(&progress));
    state
        .variables
        .insert("judgment_scale".into(), judgment_scale.clamp(0.0, 1.0));
    // Keep the alpha signal meaningful for compatible user profiles. The
    // default profile leaves image opacity at 1 and animates only its scale.
    state.variables.insert(
        "judgment_opacity".into(),
        if judgment_visible { 1.0 } else { 0.0 },
    );
    state.variables.insert(
        "judgment_visible".into(),
        if judgment_visible { 1.0 } else { 0.0 },
    );
}

fn animation_values(
    element: &UiElement,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> (f64, f64, f64, f64, f64, f64) {
    let (mut opacity, mut x, mut y, mut sx, mut sy, mut rotation) = (1.0, 0.0, 0.0, 1.0, 1.0, 0.0);
    for animation in &element.animations {
        if animation
            .when
            .as_ref()
            .is_some_and(|condition| !evaluate_condition(condition, config, state))
        {
            continue;
        }
        let animation_start = animation
            .start_from
            .as_ref()
            .and_then(|source| {
                evaluate_expression(
                    &UiExpression::Source {
                        source: source.clone(),
                    },
                    config,
                    state,
                )
            })
            .unwrap_or(animation.start);
        let value = if let Some(expression) = &animation.value_from_state {
            let Some(value) = evaluate_expression(expression, config, state) else {
                continue;
            };
            value
        } else {
            let mut t = ((state.time - animation_start) / animation.duration).clamp(0.0, 1.0);
            if animation.repeat && state.time >= animation_start {
                t = ((state.time - animation_start) / animation.duration).fract();
            }
            t = apply_easing(&animation.easing, t);
            animation.from + (animation.to - animation.from) * t
        };
        match animation.property {
            UiAnimationProperty::Opacity => opacity *= value.clamp(0.0, 1.0),
            UiAnimationProperty::OffsetX => x += value,
            UiAnimationProperty::OffsetY => y += value,
            UiAnimationProperty::ScaleX => sx *= value.max(0.0),
            UiAnimationProperty::ScaleY => sy *= value.max(0.0),
            UiAnimationProperty::Rotation => rotation += value,
        }
    }
    (opacity, x, y, sx, sy, rotation)
}

fn evaluate_expression(
    expression: &UiExpression,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> Option<f64> {
    evaluate_expression_inner(expression, config, state, &mut BTreeSet::new(), 0)
}

fn evaluate_expression_inner(
    expression: &UiExpression,
    config: &RendererUiConfig,
    state: &UiFrameState,
    variables: &mut BTreeSet<String>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    let finite = |value: f64| value.is_finite().then_some(value);
    match expression {
        UiExpression::Constant { value } => finite(*value),
        UiExpression::Source { source } => match source {
            UiStateSource::Time => finite(state.time),
            UiStateSource::DeltaTime => finite(state.delta_time),
            UiStateSource::Progress => finite(state.progress),
            UiStateSource::RuntimeEntities => Some(state.runtime_entities as f64),
            UiStateSource::ActiveEntities => Some(state.active_entities as f64),
            UiStateSource::CallbackCount => Some(state.callbacks as f64),
            UiStateSource::DrawCount => Some(state.draws as f64),
            UiStateSource::ScheduledEventCount => Some(state.scheduled_events as f64),
            UiStateSource::Score => state.gameplay.as_ref().map(|gameplay| gameplay.score),
            UiStateSource::ScoreProgress => state.gameplay.as_ref().map(|gameplay| {
                gameplay
                    .variables
                    .get("score_progress")
                    .copied()
                    .unwrap_or_else(|| (gameplay.score / 800_000.0).clamp(0.0, 1.0))
            }),
            UiStateSource::Combo => state
                .gameplay
                .as_ref()
                .map(|gameplay| gameplay.combo as f64),
            UiStateSource::Life => state.gameplay.as_ref().map(|gameplay| gameplay.life),
            UiStateSource::LatestEventTime => state
                .gameplay
                .as_ref()
                .and_then(|gameplay| gameplay.latest_event_time),
            UiStateSource::JudgmentTime => state
                .gameplay
                .as_ref()
                .and_then(|gameplay| gameplay.judgment_time),
            UiStateSource::GameplayVariable { name } => state
                .gameplay
                .as_ref()
                .and_then(|gameplay| gameplay.variables.get(name).copied()),
            UiStateSource::PresentationValue { name } => {
                state.presentation_values.get(name).copied()
            }
            UiStateSource::Memory { block_id, index } => state
                .memory
                .as_ref()
                .map(|memory| memory.get(*block_id, *index)),
        },
        UiExpression::Variable { name } => {
            if !variables.insert(name.clone()) {
                return None;
            }
            let value = config.variables.get(name).and_then(|expression| {
                evaluate_expression_inner(expression, config, state, variables, depth + 1)
            });
            variables.remove(name);
            value
        }
        UiExpression::Add { left, right } => finite(
            evaluate_expression_inner(left, config, state, variables, depth + 1)?
                + evaluate_expression_inner(right, config, state, variables, depth + 1)?,
        ),
        UiExpression::Subtract { left, right } => finite(
            evaluate_expression_inner(left, config, state, variables, depth + 1)?
                - evaluate_expression_inner(right, config, state, variables, depth + 1)?,
        ),
        UiExpression::Multiply { left, right } => finite(
            evaluate_expression_inner(left, config, state, variables, depth + 1)?
                * evaluate_expression_inner(right, config, state, variables, depth + 1)?,
        ),
        UiExpression::Divide { left, right } => {
            let denominator =
                evaluate_expression_inner(right, config, state, variables, depth + 1)?;
            if denominator.abs() <= f64::EPSILON {
                None
            } else {
                finite(
                    evaluate_expression_inner(left, config, state, variables, depth + 1)?
                        / denominator,
                )
            }
        }
        UiExpression::Min { left, right } => finite(
            evaluate_expression_inner(left, config, state, variables, depth + 1)?.min(
                evaluate_expression_inner(right, config, state, variables, depth + 1)?,
            ),
        ),
        UiExpression::Max { left, right } => finite(
            evaluate_expression_inner(left, config, state, variables, depth + 1)?.max(
                evaluate_expression_inner(right, config, state, variables, depth + 1)?,
            ),
        ),
        UiExpression::Clamp {
            value,
            minimum,
            maximum,
        } => finite(
            evaluate_expression_inner(value, config, state, variables, depth + 1)?
                .clamp(*minimum, *maximum),
        ),
    }
}

fn evaluate_condition(
    condition: &UiCondition,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> bool {
    let compare = |left: &UiExpression, right: &UiExpression| {
        Some((
            evaluate_expression(left, config, state)?,
            evaluate_expression(right, config, state)?,
        ))
    };
    match condition {
        UiCondition::Truthy { value } => {
            evaluate_expression(value, config, state).is_some_and(|value| value != 0.0)
        }
        UiCondition::Greater { left, right } => compare(left, right).is_some_and(|(a, b)| a > b),
        UiCondition::GreaterOrEqual { left, right } => {
            compare(left, right).is_some_and(|(a, b)| a >= b)
        }
        UiCondition::Less { left, right } => compare(left, right).is_some_and(|(a, b)| a < b),
        UiCondition::LessOrEqual { left, right } => {
            compare(left, right).is_some_and(|(a, b)| a <= b)
        }
        UiCondition::Equal { left, right } => compare(left, right).is_some_and(|(a, b)| a == b),
        UiCondition::NotEqual { left, right } => compare(left, right).is_some_and(|(a, b)| a != b),
        UiCondition::StringEquals { source, value } => {
            let actual = match source {
                UiStringSource::Event => state
                    .gameplay
                    .as_ref()
                    .and_then(|gameplay| gameplay.event.as_deref()),
                UiStringSource::NoteType => state
                    .gameplay
                    .as_ref()
                    .and_then(|gameplay| gameplay.note_type.as_deref()),
                UiStringSource::Judgment => state
                    .gameplay
                    .as_ref()
                    .and_then(|gameplay| gameplay.judgment.as_deref()),
                UiStringSource::PresentationCategory { name } => {
                    state.presentation_categories.get(name).map(String::as_str)
                }
            };
            actual.is_some_and(|actual| actual == value)
        }
        UiCondition::All { conditions } => conditions
            .iter()
            .all(|condition| evaluate_condition(condition, config, state)),
        UiCondition::Any { conditions } => conditions
            .iter()
            .any(|condition| evaluate_condition(condition, config, state)),
        UiCondition::Not { condition } => !evaluate_condition(condition, config, state),
    }
}

fn format_template(
    template: &str,
    value: Option<&UiExpression>,
    config: &RendererUiConfig,
    state: &UiFrameState,
) -> String {
    let mut output = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        output.push_str(&remaining[..start]);
        let after_open = &remaining[start + 1..];
        let Some(end) = after_open.find('}') else {
            output.push_str(&remaining[start..]);
            remaining = "";
            break;
        };
        let key = &after_open[..end];
        let replacement = if key == "value" {
            value
                .and_then(|expression| evaluate_expression(expression, config, state))
                .map(format_value)
        } else {
            template_value(key, config, state)
        };
        if let Some(replacement) = replacement {
            output.push_str(&replacement);
        }
        remaining = &after_open[end + 1..];
    }
    output.push_str(remaining);
    output
}

fn template_value(key: &str, config: &RendererUiConfig, state: &UiFrameState) -> Option<String> {
    let source = match key {
        "time" => UiStateSource::Time,
        "delta_time" => UiStateSource::DeltaTime,
        "progress" => UiStateSource::Progress,
        "runtime_entities" => UiStateSource::RuntimeEntities,
        "active_entities" => UiStateSource::ActiveEntities,
        "callbacks" => UiStateSource::CallbackCount,
        "draws" => UiStateSource::DrawCount,
        "scheduled_events" => UiStateSource::ScheduledEventCount,
        "score" => UiStateSource::Score,
        "score_progress" => UiStateSource::ScoreProgress,
        "combo" => UiStateSource::Combo,
        "life" => UiStateSource::Life,
        "event" | "note_type" | "judgment" => {
            let gameplay = state.gameplay.as_ref()?;
            return match key {
                "event" => gameplay.event.clone(),
                "note_type" => gameplay.note_type.clone(),
                _ => gameplay.judgment.clone(),
            };
        }
        _ => {
            if let Some(name) = key.strip_prefix("variable.") {
                let expression = config.variables.get(name)?;
                return evaluate_expression(expression, config, state).map(format_value);
            }
            return state
                .gameplay
                .as_ref()?
                .variables
                .get(key)
                .copied()
                .map(format_value);
        }
    };
    evaluate_expression(&UiExpression::Source { source }, config, state).map(format_value)
}

fn format_counter(value: f64, digits: usize, max_digits: Option<usize>) -> String {
    let mut value = format_value(value);
    if let Some(max_digits) = max_digits.filter(|limit| *limit > 0) {
        let character_count = value.chars().count();
        if character_count > max_digits {
            value = value.chars().skip(character_count - max_digits).collect();
        }
    }
    if digits == 0 {
        value
    } else {
        format!("{value:0>digits$}")
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gauge(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    track: [u8; 4],
    fill: [u8; 4],
    orientation: UiGaugeOrientation,
    value: &UiExpression,
    maximum: &UiExpression,
    fill_asset: Option<&str>,
    fill_source: Option<[f64; 4]>,
    mask_asset: Option<&str>,
    config: &RendererUiConfig,
    state: &UiFrameState,
    mut alpha_target: Option<&mut [u8]>,
) {
    draw_transformed_rect(
        rgb,
        width,
        height,
        matrix,
        rect,
        track,
        0.0,
        alpha_target.as_deref_mut(),
    );
    let Some(value) = evaluate_expression(value, config, state) else {
        return;
    };
    let Some(maximum) = evaluate_expression(maximum, config, state) else {
        return;
    };
    if maximum <= 0.0 {
        return;
    }
    let fraction = (value / maximum).clamp(0.0, 1.0);
    let mut filled = rect;
    let source = if let Some(asset) = fill_asset.and_then(|asset| config.loaded_assets.get(asset)) {
        let source = fill_source
            .and_then(|source| normalized_source_rect(source, asset))
            .unwrap_or([0, 0, asset.width, asset.height]);
        let mut source_region = source;
        match orientation {
            UiGaugeOrientation::LeftToRight => {
                filled.width *= fraction;
                source_region[2] = (f64::from(source[2]) * fraction).round().max(1.0) as u32;
            }
            UiGaugeOrientation::RightToLeft => {
                filled.x += rect.width * (1.0 - fraction);
                filled.width *= fraction;
                source_region[0] += (f64::from(source[2]) * (1.0 - fraction)).round() as u32;
                source_region[2] = (f64::from(source[2]) * fraction).round().max(1.0) as u32;
            }
            UiGaugeOrientation::BottomToTop => {
                filled.y += rect.height * (1.0 - fraction);
                filled.height *= fraction;
                source_region[1] += (f64::from(source[3]) * (1.0 - fraction)).round() as u32;
                source_region[3] = (f64::from(source[3]) * fraction).round().max(1.0) as u32;
            }
            UiGaugeOrientation::TopToBottom => {
                filled.height *= fraction;
                source_region[3] = (f64::from(source[3]) * fraction).round().max(1.0) as u32;
            }
        }
        Some((asset, source_region))
    } else {
        match orientation {
            UiGaugeOrientation::LeftToRight => filled.width *= fraction,
            UiGaugeOrientation::RightToLeft => {
                filled.x += rect.width * (1.0 - fraction);
                filled.width *= fraction;
            }
            UiGaugeOrientation::BottomToTop => {
                filled.y += rect.height * (1.0 - fraction);
                filled.height *= fraction;
            }
            UiGaugeOrientation::TopToBottom => filled.height *= fraction,
        }
        None
    };
    if let Some((bitmap, source)) = source {
        let mask = mask_asset
            .and_then(|asset| config.loaded_assets.get(asset))
            .map(|bitmap| bitmap.as_ref());
        draw_transformed_image_masked(
            rgb,
            width,
            height,
            matrix,
            filled,
            bitmap,
            source,
            None,
            mask,
            alpha_target.as_deref_mut(),
        );
    } else {
        draw_transformed_rect(rgb, width, height, matrix, filled, fill, 0.0, alpha_target);
    }
}

fn normalized_source_rect(source: [f64; 4], bitmap: &UiBitmap) -> Option<[u32; 4]> {
    if source
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || source[2] <= 0.0
        || source[3] <= 0.0
        || source[0] + source[2] > 1.0 + f64::EPSILON
        || source[1] + source[3] > 1.0 + f64::EPSILON
    {
        return None;
    }
    let x = (source[0] * f64::from(bitmap.width)).round() as u32;
    let y = (source[1] * f64::from(bitmap.height)).round() as u32;
    let right = ((source[0] + source[2]) * f64::from(bitmap.width)).round() as u32;
    let bottom = ((source[1] + source[3]) * f64::from(bitmap.height)).round() as u32;
    (right > x && bottom > y && right <= bitmap.width && bottom <= bitmap.height).then_some([
        x,
        y,
        right - x,
        bottom - y,
    ])
}

#[allow(clippy::too_many_arguments)]
fn draw_ui_text(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    text: &str,
    color: [u8; 4],
    font_size: f64,
    align: UiTextAlign,
    font_name: Option<&str>,
    fonts: &BTreeMap<String, UiBitmapFont>,
    loaded_assets: &BTreeMap<String, Arc<UiBitmap>>,
    preserve_builtin_case: bool,
    mut alpha_target: Option<&mut [u8]>,
) {
    let custom_font = font_name.and_then(|name| fonts.get(name));
    let scale = font_size / 7.0;
    let mut chars_drawn = 0;
    let lines: Vec<_> = text.split('\n').take(16).collect();
    for (line_index, line) in lines.into_iter().enumerate() {
        let chars: Vec<char> = line.chars().take(256 - chars_drawn).collect();
        chars_drawn += chars.len();
        if chars.is_empty() && chars_drawn >= 256 {
            break;
        }
        let metrics: Vec<_> = chars
            .iter()
            .map(|ch| text_glyph_metrics(*ch, font_size, custom_font, loaded_assets))
            .collect();
        let line_height = metrics
            .iter()
            .map(|(_, glyph_height, _)| *glyph_height)
            .fold(font_size, f64::max);
        let block_height = line_height * text.split('\n').take(16).count() as f64;
        let glyph_positions = text_glyph_positions(
            rect.width,
            &metrics,
            align,
            custom_font.is_some(),
            custom_font.and_then(|font| font.center_spacing),
            custom_font.map(|font| {
                font_size * f64::from(font.glyph_width) / f64::from(font.glyph_height.max(1))
            }),
        );
        let line_y = (rect.height - block_height) * 0.5 + line_index as f64 * line_height;
        for (index, (ch, (glyph_width, glyph_height, _))) in
            chars.into_iter().zip(metrics).enumerate()
        {
            let x = glyph_positions[index];
            if let Some(font) = custom_font {
                if let Some((bitmap, source)) = bitmap_glyph(font, ch, loaded_assets) {
                    draw_transformed_image(
                        rgb,
                        width,
                        height,
                        matrix,
                        UiRect {
                            x,
                            y: line_y + (line_height - glyph_height) * 0.5,
                            width: glyph_width,
                            height: glyph_height,
                        },
                        bitmap,
                        source,
                        Some(color),
                        alpha_target.as_deref_mut(),
                    );
                } else {
                    draw_builtin_glyph(
                        rgb,
                        width,
                        height,
                        matrix,
                        x,
                        line_y,
                        scale,
                        builtin_case(ch, preserve_builtin_case),
                        color,
                        alpha_target.as_deref_mut(),
                    );
                }
            } else {
                draw_builtin_glyph(
                    rgb,
                    width,
                    height,
                    matrix,
                    x,
                    line_y,
                    scale,
                    builtin_case(ch, preserve_builtin_case),
                    color,
                    alpha_target.as_deref_mut(),
                );
            }
        }
    }
}

fn text_glyph_positions(
    rect_width: f64,
    metrics: &[(f64, f64, f64)],
    align: UiTextAlign,
    bitmap_font: bool,
    center_spacing: Option<f64>,
    center_anchor_glyph_width: Option<f64>,
) -> Vec<f64> {
    if metrics.is_empty() {
        return Vec::new();
    }

    if let Some(spacing) = center_spacing.filter(|spacing| spacing.is_finite() && *spacing > 0.0) {
        let count = metrics.len();
        // Some bitmap counters use larger shadow canvases than their
        // foreground glyphs. Their centers still share the authored digit
        // slots, so align the first/last slot using the font's canonical
        // glyph width rather than each image's expanded shadow bounds.
        let anchor_glyph_width = center_anchor_glyph_width
            .filter(|width| width.is_finite() && *width > 0.0)
            .unwrap_or(metrics[0].0);
        let first_center = match align {
            UiTextAlign::Center => {
                rect_width * 0.5 - (count.saturating_sub(1) as f64) * spacing * 0.5
            }
            UiTextAlign::Left => anchor_glyph_width * 0.5,
            UiTextAlign::Right => {
                rect_width - anchor_glyph_width * 0.5 - count.saturating_sub(1) as f64 * spacing
            }
        };
        return metrics
            .iter()
            .enumerate()
            .map(|(index, (glyph_width, _, _))| {
                first_center + index as f64 * spacing - glyph_width * 0.5
            })
            .collect();
    }

    let advance_width = metrics.iter().map(|(_, _, advance)| advance).sum::<f64>();
    // Center/right align bitmap glyphs by their visible bounds. The last
    // glyph may extend beyond its advance; aligning advance width alone shifts
    // the rendered word away from its actual group center.
    let text_width = if bitmap_font {
        metrics
            .last()
            .map_or(advance_width, |(last_width, _, last_advance)| {
                advance_width - last_advance + last_width
            })
    } else {
        advance_width
    };
    let mut x = match align {
        UiTextAlign::Left => 0.0,
        UiTextAlign::Center => (rect_width - text_width) * 0.5,
        UiTextAlign::Right => rect_width - text_width,
    };
    metrics
        .iter()
        .map(|(_, _, advance)| {
            let position = x;
            x += advance;
            position
        })
        .collect()
}

fn builtin_case(ch: char, preserve_case: bool) -> char {
    if preserve_case {
        ch
    } else {
        ch.to_ascii_uppercase()
    }
}

fn text_glyph_metrics(
    ch: char,
    font_size: f64,
    font: Option<&UiBitmapFont>,
    assets: &BTreeMap<String, Arc<UiBitmap>>,
) -> (f64, f64, f64) {
    if let Some(font) = font {
        if let Some((_, source)) = bitmap_glyph(font, ch, assets) {
            let glyph_scale = font
                .glyph_scales
                .get(&ch.to_string())
                .copied()
                .unwrap_or(1.0);
            let glyph_height = font_size * glyph_scale;
            let glyph_width = glyph_height * f64::from(source[2]) / f64::from(source[3].max(1));
            let advance = font
                .glyph_advances
                .get(&ch.to_string())
                .copied()
                .unwrap_or(font.advance);
            return (glyph_width, glyph_height, glyph_width * advance);
        }
    }
    let glyph_width = font_size * 5.0 / 7.0;
    (glyph_width, font_size, font_size * 6.0 / 7.0)
}

fn bitmap_glyph<'a>(
    font: &UiBitmapFont,
    ch: char,
    assets: &'a BTreeMap<String, Arc<UiBitmap>>,
) -> Option<(&'a UiBitmap, [u32; 4])> {
    if let Some(bitmap) = font
        .glyph_assets
        .get(&ch.to_string())
        .and_then(|asset| assets.get(asset))
    {
        return Some((bitmap, [0, 0, bitmap.width, bitmap.height]));
    }
    let bitmap = assets.get(&font.asset)?;
    if font.asset.is_empty() {
        return None;
    }
    let index = font
        .glyph_map
        .get(&ch.to_string())
        .copied()
        .unwrap_or_else(|| (ch as u32).saturating_sub(font.first_codepoint));
    let source_x = (index % font.columns).checked_mul(font.glyph_width)?;
    let source_y = (index / font.columns).checked_mul(font.glyph_height)?;
    if source_x.checked_add(font.glyph_width)? > bitmap.width
        || source_y.checked_add(font.glyph_height)? > bitmap.height
    {
        return None;
    }
    Some((
        bitmap,
        [source_x, source_y, font.glyph_width, font.glyph_height],
    ))
}

#[allow(clippy::too_many_arguments)]
fn draw_builtin_glyph(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    x: f64,
    y: f64,
    scale: f64,
    ch: char,
    color: [u8; 4],
    mut alpha_target: Option<&mut [u8]>,
) {
    for (row, bits) in builtin_glyph(ch).into_iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                draw_transformed_rect(
                    rgb,
                    width,
                    height,
                    matrix,
                    UiRect {
                        x: x + col as f64 * scale,
                        y: y + row as f64 * scale,
                        width: scale,
                        height: scale,
                    },
                    color,
                    0.0,
                    alpha_target.as_deref_mut(),
                );
            }
        }
    }
}

pub(crate) fn measure_watermark_text(
    text: &str,
    font_size: f64,
    font_name: Option<&str>,
    fonts: &BTreeMap<String, UiBitmapFont>,
    assets: &BTreeMap<String, Arc<UiBitmap>>,
) -> [f64; 2] {
    let font = font_name.and_then(|name| fonts.get(name));
    let lines: Vec<_> = text.split('\n').take(16).collect();
    let width = lines
        .iter()
        .map(|line| {
            line.chars()
                .take(256)
                .map(|ch| text_glyph_metrics(ch, font_size, font, assets).2)
                .sum::<f64>()
        })
        .fold(0.0, f64::max);
    [width, font_size * lines.len() as f64]
}

pub(crate) fn render_watermark_text(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    text: &str,
    color: [u8; 4],
    font_size: f64,
    font_name: Option<&str>,
    fonts: &BTreeMap<String, UiBitmapFont>,
    assets: &BTreeMap<String, Arc<UiBitmap>>,
    measured_size: [f64; 2],
    center: [f64; 2],
    scale: f64,
    rotation_degrees: f64,
    opacity: f64,
) {
    let [text_width, text_height] = measured_size;
    if text_width <= 0.0 || text_height <= 0.0 {
        return;
    }
    let angle = rotation_degrees.to_radians();
    let cosine = angle.cos() * scale;
    let sine = angle.sin() * scale;
    let local_center = [text_width * 0.5, text_height * 0.5];
    let matrix = UiMatrix {
        a: cosine,
        b: sine,
        c: -sine,
        d: cosine,
        tx: center[0] - cosine * local_center[0] + sine * local_center[1],
        ty: center[1] - sine * local_center[0] - cosine * local_center[1],
        opacity: opacity.clamp(0.0, 1.0),
        blend_mode: UiBlendMode::Normal,
    };
    draw_ui_text(
        rgb,
        width,
        height,
        matrix,
        UiRect {
            x: 0.0,
            y: 0.0,
            width: text_width,
            height: text_height,
        },
        text,
        color,
        font_size,
        UiTextAlign::Left,
        font_name,
        fonts,
        assets,
        true,
        None,
    );
}

pub(crate) fn render_watermark_image(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    bitmap: &UiBitmap,
    source: [u32; 4],
    tint: [u8; 4],
    center: [f64; 2],
    size: [f64; 2],
    rotation_degrees: f64,
    opacity: f64,
) {
    let angle = rotation_degrees.to_radians();
    let cosine = angle.cos();
    let sine = angle.sin();
    let local_center = [size[0] * 0.5, size[1] * 0.5];
    let matrix = UiMatrix {
        a: cosine,
        b: sine,
        c: -sine,
        d: cosine,
        tx: center[0] - cosine * local_center[0] + sine * local_center[1],
        ty: center[1] - sine * local_center[0] - cosine * local_center[1],
        opacity: opacity.clamp(0.0, 1.0),
        blend_mode: UiBlendMode::Normal,
    };
    draw_transformed_image(
        rgb,
        width,
        height,
        matrix,
        UiRect {
            x: 0.0,
            y: 0.0,
            width: size[0],
            height: size[1],
        },
        bitmap,
        source,
        Some(tint),
        None,
    );
}

/*
 * The bitmap font renderer uses a tiny built-in 5x7 font for missing glyphs.
 * Custom atlases remain cached by the renderer UI/watermark profile.
 */

fn builtin_glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        'a' => [0, 0, 14, 1, 15, 17, 15],
        'b' => [16, 16, 30, 17, 17, 17, 30],
        'c' => [0, 0, 14, 17, 16, 17, 14],
        'd' => [1, 1, 15, 17, 17, 17, 15],
        'e' => [0, 0, 14, 17, 31, 16, 14],
        'f' => [6, 9, 8, 28, 8, 8, 8],
        'g' => [0, 15, 17, 17, 15, 1, 14],
        'h' => [16, 16, 30, 17, 17, 17, 17],
        'i' => [4, 0, 12, 4, 4, 4, 14],
        'j' => [2, 0, 6, 2, 2, 18, 12],
        'k' => [16, 16, 18, 20, 24, 20, 18],
        'l' => [12, 4, 4, 4, 4, 4, 14],
        'm' => [0, 0, 26, 21, 21, 21, 21],
        'n' => [0, 0, 30, 17, 17, 17, 17],
        'o' => [0, 0, 14, 17, 17, 17, 14],
        'p' => [0, 30, 17, 17, 30, 16, 16],
        'q' => [0, 15, 17, 17, 15, 1, 1],
        'r' => [0, 0, 22, 25, 16, 16, 16],
        's' => [0, 15, 16, 14, 1, 30, 0],
        't' => [8, 8, 28, 8, 8, 9, 6],
        'u' => [0, 0, 17, 17, 17, 19, 13],
        'v' => [0, 0, 17, 17, 17, 10, 4],
        'w' => [0, 0, 17, 17, 21, 21, 10],
        'x' => [0, 0, 17, 10, 4, 10, 17],
        'y' => [0, 17, 17, 17, 15, 1, 14],
        'z' => [0, 0, 31, 2, 4, 8, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '※' => [21, 10, 31, 10, 21, 0, 0],
        '「' => [31, 16, 16, 16, 16, 16, 16],
        '」' => [1, 1, 1, 1, 1, 1, 31],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '%' => [17, 2, 4, 8, 17, 0, 0],
        ' ' => [0; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_transformed_rect(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    color: [u8; 4],
    radius: f64,
    alpha_target: Option<&mut [u8]>,
) {
    draw_transformed_shape(
        rgb,
        width,
        height,
        matrix,
        rect,
        radius,
        |_, _| true,
        color,
        alpha_target,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_transformed_border(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    color: [u8; 4],
    radius: f64,
    alpha_target: Option<&mut [u8]>,
) {
    let border = 2.0_f64.min(rect.width.min(rect.height) * 0.5);
    draw_transformed_shape(
        rgb,
        width,
        height,
        matrix,
        rect,
        radius,
        |x, y| x < border || y < border || x >= rect.width - border || y >= rect.height - border,
        color,
        alpha_target,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_transformed_shape(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    radius: f64,
    predicate: impl Fn(f64, f64) -> bool,
    color: [u8; 4],
    mut alpha_target: Option<&mut [u8]>,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let Some(bounds) = transformed_bounds(matrix, rect, width, height) else {
        return;
    };
    let radius = radius.min(rect.width.min(rect.height) * 0.5);
    for y in bounds.1..bounds.3 {
        for x in bounds.0..bounds.2 {
            let Some([local_x, local_y]) =
                matrix.inverse_point(f64::from(x) + 0.5, f64::from(y) + 0.5)
            else {
                continue;
            };
            let shape_x = local_x - rect.x;
            let shape_y = local_y - rect.y;
            if shape_x < 0.0
                || shape_y < 0.0
                || shape_x >= rect.width
                || shape_y >= rect.height
                || !predicate(shape_x, shape_y)
            {
                continue;
            }
            if radius > 0.0 {
                let cx = if shape_x < radius {
                    radius
                } else if shape_x > rect.width - radius {
                    rect.width - radius
                } else {
                    shape_x
                };
                let cy = if shape_y < radius {
                    radius
                } else if shape_y > rect.height - radius {
                    rect.height - radius
                } else {
                    shape_y
                };
                if (shape_x - cx).powi(2) + (shape_y - cy).powi(2) > radius.powi(2) {
                    continue;
                }
            }
            blend_pixel(
                rgb,
                width,
                height,
                x,
                y,
                color,
                matrix.opacity,
                matrix.blend_mode,
                alpha_target.as_deref_mut(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_transformed_image(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    bitmap: &UiBitmap,
    source: [u32; 4],
    tint: Option<[u8; 4]>,
    alpha_target: Option<&mut [u8]>,
) {
    draw_transformed_image_masked(
        rgb,
        width,
        height,
        matrix,
        rect,
        bitmap,
        source,
        tint,
        None,
        alpha_target,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_transformed_image_masked(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    bitmap: &UiBitmap,
    source: [u32; 4],
    tint: Option<[u8; 4]>,
    mask: Option<&UiBitmap>,
    mut alpha_target: Option<&mut [u8]>,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 || source[2] == 0 || source[3] == 0 {
        return;
    }
    if source[0]
        .checked_add(source[2])
        .is_none_or(|right| right > bitmap.width)
        || source[1]
            .checked_add(source[3])
            .is_none_or(|bottom| bottom > bitmap.height)
    {
        return;
    }
    let Some(bounds) = transformed_bounds(matrix, rect, width, height) else {
        return;
    };
    for y in bounds.1..bounds.3 {
        for x in bounds.0..bounds.2 {
            let Some([local_x, local_y]) =
                matrix.inverse_point(f64::from(x) + 0.5, f64::from(y) + 0.5)
            else {
                continue;
            };
            let image_x = local_x - rect.x;
            let image_y = local_y - rect.y;
            if image_x < 0.0 || image_y < 0.0 || image_x >= rect.width || image_y >= rect.height {
                continue;
            }
            let source_x = f64::from(source[0]) + image_x / rect.width * f64::from(source[2]);
            let source_y = f64::from(source[1]) + image_y / rect.height * f64::from(source[3]);
            let mut color = sample_rgba_bilinear(bitmap, source_x, source_y, source);
            if let Some(mask) = mask {
                let mask_alpha = if mask.width == bitmap.width && mask.height == bitmap.height {
                    sample_rgba_bilinear(mask, source_x, source_y, source)[3]
                } else {
                    255
                };
                color[3] = (u16::from(color[3]) * u16::from(mask_alpha) / 255) as u8;
            }
            if let Some(tint) = tint {
                for channel in 0..4 {
                    color[channel] =
                        (u16::from(color[channel]) * u16::from(tint[channel]) / 255) as u8;
                }
            }
            blend_pixel(
                rgb,
                width,
                height,
                x,
                y,
                color,
                matrix.opacity,
                matrix.blend_mode,
                alpha_target.as_deref_mut(),
            );
        }
    }
}

fn sample_rgba_bilinear(bitmap: &UiBitmap, x: f64, y: f64, source: [u32; 4]) -> [u8; 4] {
    let left = source[0] as i32;
    let top = source[1] as i32;
    let right = (source[0] + source[2] - 1) as i32;
    let bottom = (source[1] + source[3] - 1) as i32;
    let sample_x = (x - 0.5).clamp(f64::from(left), f64::from(right));
    let sample_y = (y - 0.5).clamp(f64::from(top), f64::from(bottom));
    let x0 = sample_x.floor() as i32;
    let y0 = sample_y.floor() as i32;
    let x1 = (x0 + 1).min(right);
    let y1 = (y0 + 1).min(bottom);
    let tx = sample_x - f64::from(x0);
    let ty = sample_y - f64::from(y0);
    let samples = [
        (x0, y0, (1.0 - tx) * (1.0 - ty)),
        (x1, y0, tx * (1.0 - ty)),
        (x0, y1, (1.0 - tx) * ty),
        (x1, y1, tx * ty),
    ];
    let mut alpha = 0.0;
    let mut premultiplied = [0.0; 3];
    for (sx, sy, weight) in samples {
        let offset = (sy as usize * bitmap.width as usize + sx as usize) * 4;
        let sample_alpha = f64::from(bitmap.rgba[offset + 3]) / 255.0;
        alpha += sample_alpha * weight;
        for channel in 0..3 {
            premultiplied[channel] +=
                f64::from(bitmap.rgba[offset + channel]) * sample_alpha * weight;
        }
    }
    if alpha <= f64::EPSILON {
        return [0, 0, 0, 0];
    }
    [
        (premultiplied[0] / alpha).round().clamp(0.0, 255.0) as u8,
        (premultiplied[1] / alpha).round().clamp(0.0, 255.0) as u8,
        (premultiplied[2] / alpha).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn transformed_bounds(
    matrix: UiMatrix,
    rect: UiRect,
    output_width: u32,
    output_height: u32,
) -> Option<(i32, i32, i32, i32)> {
    let points = [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x + rect.width, rect.y + rect.height),
        (rect.x, rect.y + rect.height),
    ]
    .map(|(x, y)| {
        (
            matrix.a * x + matrix.c * y + matrix.tx,
            matrix.b * x + matrix.d * y + matrix.ty,
        )
    });
    let min_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0.0, f64::from(output_width)) as i32;
    let max_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, f64::from(output_width)) as i32;
    let min_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .clamp(0.0, f64::from(output_height)) as i32;
    let max_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .clamp(0.0, f64::from(output_height)) as i32;
    (max_x > min_x && max_y > min_y).then_some((min_x, min_y, max_x, max_y))
}

#[allow(clippy::too_many_arguments)]
fn blend_pixel(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    color: [u8; 4],
    opacity: f64,
    blend_mode: UiBlendMode,
    alpha_target: Option<&mut [u8]>,
) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let alpha = (f64::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u16;
    if alpha == 0 {
        return;
    }
    let offset = (y as usize * width as usize + x as usize) * 3;
    if let Some(alpha_plane) = alpha_target {
        let alpha_offset = y as usize * width as usize + x as usize;
        let source_alpha = f64::from(alpha) / 255.0;
        let destination_alpha = f64::from(alpha_plane[alpha_offset]) / 255.0;
        let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
        let output_alpha_byte = (output_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        for channel in 0..3 {
            let source_premultiplied = f64::from(color[channel]) * source_alpha;
            let destination_premultiplied = f64::from(rgb[offset + channel]);
            let output = match blend_mode {
                UiBlendMode::Normal => {
                    source_premultiplied + destination_premultiplied * (1.0 - source_alpha)
                }
                UiBlendMode::Add => source_premultiplied + destination_premultiplied,
            };
            rgb[offset + channel] = output.round().clamp(0.0, 255.0) as u8;
        }
        alpha_plane[alpha_offset] = output_alpha_byte;
        return;
    }
    for channel in 0..3 {
        rgb[offset + channel] = match blend_mode {
            UiBlendMode::Normal => {
                ((u16::from(rgb[offset + channel]) * (255 - alpha)
                    + u16::from(color[channel]) * alpha)
                    / 255) as u8
            }
            UiBlendMode::Add => (u16::from(rgb[offset + channel])
                + (u16::from(color[channel]) * alpha + 127) / 255)
                .min(255) as u8,
        };
    }
}

fn draw_value_panel(
    rgb: &mut [u8],
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    panel_w: i32,
    panel_h: i32,
    label: &str,
    value: f64,
    maximum: Option<f64>,
    theme: &UiTheme,
) {
    blend_rect(
        rgb,
        width,
        height,
        x,
        y,
        panel_w,
        panel_h,
        theme.panel,
        theme.panel_alpha,
    );
    draw_text(
        rgb,
        width,
        height,
        x + 7,
        y + 5,
        &label.to_uppercase(),
        theme.label,
    );
    draw_text(
        rgb,
        width,
        height,
        x + panel_w / 2,
        y + 5,
        &format_value(value),
        theme.text,
    );
    if let Some(maximum) = maximum {
        let fraction = (value / maximum).clamp(0.0, 1.0);
        let bar_w = panel_w - 14;
        blend_rect(
            rgb,
            width,
            height,
            x + 7,
            y + panel_h - 5,
            bar_w,
            2,
            theme.progress_track,
            255,
        );
        blend_rect(
            rgb,
            width,
            height,
            x + 7,
            y + panel_h - 5,
            (f64::from(bar_w) * fraction).round() as i32,
            2,
            theme.accent,
            255,
        );
    }
}

fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn blend_rect(
    rgb: &mut [u8],
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    rect_w: i32,
    rect_h: i32,
    color: [u8; 3],
    alpha: u8,
) {
    for py in y.max(0)..(y + rect_h).min(height) {
        for px in x.max(0)..(x + rect_w).min(width) {
            let offset = (py as usize * width as usize + px as usize) * 3;
            for channel in 0..3 {
                rgb[offset + channel] = ((u16::from(rgb[offset + channel])
                    * u16::from(255 - alpha)
                    + u16::from(color[channel]) * u16::from(alpha))
                    / 255) as u8;
            }
        }
    }
}

fn draw_text(rgb: &mut [u8], width: i32, height: i32, x: i32, y: i32, text: &str, color: [u8; 3]) {
    let mut cursor = x;
    for ch in text.chars().take(32) {
        let rows = glyph(ch);
        for (row, bits) in rows.into_iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    blend_rect(
                        rgb,
                        width,
                        height,
                        cursor + col,
                        y + row as i32,
                        1,
                        1,
                        color,
                        255,
                    );
                }
            }
        }
        cursor += 6;
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '%' => [17, 2, 4, 8, 17, 0, 0],
        ' ' => [0; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> RendererUiConfig {
        RendererUiConfig::load_profile(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("ui-profiles")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn supplied_reference_profile_uses_the_versioned_asset_registry() {
        let loaded = profile("default.json");
        assert_eq!(
            loaded.asset_pack.as_deref(),
            Some(Path::new("../assets/presentation-pack.json"))
        );
        assert!(loaded
            .loaded_asset_path("score-panel")
            .is_some_and(|path| path.ends_with("assets/score/bg.png")));
        assert!(loaded
            .loaded_asset_dimensions("score-panel")
            .is_some_and(|(width, height)| width > height * 4));
        assert_eq!(
            loaded.assets["judge-perfect"],
            PathBuf::from("pack://judge.1")
        );
        for id in [
            "combo-shadow-value",
            "combo-value",
            "combo-event-pulse",
            "combo-event-pop",
        ] {
            assert_eq!(
                loaded
                    .elements
                    .iter()
                    .find(|element| element.id == id)
                    .and_then(|element| element.isolation_group.as_deref()),
                Some("combo-number")
            );
        }
        let restored: RendererUiConfig =
            serde_json::from_value(serde_json::to_value(&loaded).unwrap()).unwrap();
        assert_eq!(
            restored
                .elements
                .iter()
                .find(|element| element.id == "combo-event-pop")
                .and_then(|element| element.isolation_group.as_deref()),
            Some("combo-number")
        );
    }

    #[test]
    fn sono_overlay_combo_glyphs_match_packaged_overlay_art_at_native_resolution() {
        let config = profile("default.json");
        assert_eq!(config.fonts["combo"].glyph_max_dimension, Some(256));
        assert_eq!(config.fonts["combo-burst"].glyph_max_dimension, Some(256));
        // `sekai.obj2` resolves images relative to PED_DATA.path, which is
        // populated by Sono-Overlay's generated package. The adjacent
        // `build/assets` tree is that runnable package output. The raw
        // `Sono-Overlay/assets` authoring PNGs are a different art revision
        // for p0..p9 and are not the expected pixels in the reference video.
        let reference_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("TestingSuite/Sono-Overlay/build/assets/combo");

        for (font_name, asset_prefix, reference_prefix) in [
            ("combo", "combo-digit-", "p"),
            ("combo-burst", "combo-burst-", "b"),
        ] {
            let font = &config.fonts[font_name];
            for digit in '0'..='9' {
                let character = digit.to_string();
                let asset_id = &font.glyph_assets[&character];
                assert_eq!(asset_id, &format!("{asset_prefix}{digit}"));
                let actual = &config.loaded_assets[asset_id];
                let expected = image::ImageReader::open(
                    reference_root.join(format!("{reference_prefix}{digit}.png")),
                )
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .decode()
                .unwrap()
                .to_rgba8();
                assert_eq!(
                    (actual.width, actual.height),
                    expected.dimensions(),
                    "{font_name} digit {digit} was rescaled"
                );
                assert_eq!(
                    actual.rgba.as_slice(),
                    expected.as_raw().as_slice(),
                    "{font_name} digit {digit} differs from Sono-Overlay's source PNG"
                );
            }
        }
    }

    #[test]
    fn packaged_ap_combo_art_reaches_the_production_raster_path() {
        const WIDTH: u32 = 1920;
        const HEIGHT: u32 = 1080;
        let mut config = profile("default.json");
        config
            .elements
            .retain(|element| element.id == "combo-value");
        let state = UiFrameState {
            time: 1.0,
            frame_rate: 60.0,
            gameplay: Some(UiGameplayState {
                combo: 8,
                ..UiGameplayState::default()
            }),
            ..UiFrameState::default()
        };
        let mut frame = rgb(WIDTH, HEIGHT, [0, 0, 0]);
        render_gameplay_ui(&mut frame, WIDTH, HEIGHT, &config, &state).unwrap();

        // The packaged AP `p8` glyph has a broad light lavender/white fill.
        // The raw authoring p8 has a blue fill and falls below this count.
        // Count actual RGB output pixels in the source-defined combo region,
        // not just the profile's symbolic asset ID.
        let mut packaged_palette_pixels = 0usize;
        for y in 360..590 {
            for x in 1500..1770 {
                let offset = ((y * WIDTH + x) * 3) as usize;
                let red = frame[offset];
                let blue = frame[offset + 2];
                if red > 220 && blue > 220 {
                    packaged_palette_pixels += 1;
                }
            }
        }
        assert!(
            packaged_palette_pixels > 4_000,
            "expected the packaged AP combo-art palette in the actual compositor raster, found {packaged_palette_pixels} pixels"
        );
    }

    #[test]
    fn sono_overlay_combo_digits_share_the_source_center_slots_for_all_widths() {
        let config = profile("default.json");
        let values = ["8", "11", "46", "48", "100", "106", "111", "1000"];
        let reference_rect_width = 0.22 * 1920.0;
        let font_sizes = [("combo", 157.5), ("combo-burst", 221.55)];
        let mut measured_bounds = BTreeMap::new();

        for (font_name, font_size) in font_sizes {
            let font = &config.fonts[font_name];
            assert_eq!(font.center_spacing, Some(108.0));
            for value in values {
                let metrics: Vec<_> = value
                    .chars()
                    .map(|ch| text_glyph_metrics(ch, font_size, Some(font), &config.loaded_assets))
                    .collect();
                let positions = text_glyph_positions(
                    reference_rect_width,
                    &metrics,
                    UiTextAlign::Center,
                    true,
                    font.center_spacing,
                    Some(
                        font_size * f64::from(font.glyph_width)
                            / f64::from(font.glyph_height.max(1)),
                    ),
                );
                let first_center =
                    reference_rect_width * 0.5 - (metrics.len().saturating_sub(1) as f64) * 54.0;
                let centers: Vec<_> = positions
                    .iter()
                    .zip(&metrics)
                    .map(|(left, (glyph_width, _, _))| left + glyph_width * 0.5)
                    .collect();
                for (index, center) in centers.iter().enumerate() {
                    let expected = first_center + index as f64 * 108.0;
                    assert!(
                        (center - expected).abs() < 1e-9,
                        "{font_name} {value}: glyph {index} center {center} != {expected}"
                    );
                }
                assert!(
                    (centers.first().unwrap() + centers.last().unwrap() - reference_rect_width)
                        .abs()
                        < 1e-9,
                    "{font_name} {value} must stay centered as its digit count changes"
                );
                let bounds = (
                    positions[0],
                    positions
                        .iter()
                        .zip(&metrics)
                        .map(|(left, (glyph_width, _, _))| left + glyph_width)
                        .fold(f64::NEG_INFINITY, f64::max),
                );
                let expected_bounds_width = (metrics.len().saturating_sub(1) as f64) * 108.0
                    + (metrics.first().unwrap().0 + metrics.last().unwrap().0) * 0.5;
                assert!(
                    ((bounds.1 - bounds.0) - expected_bounds_width).abs() < 1e-9,
                    "{font_name} {value}: measured width {} does not match source-centered width {expected_bounds_width}",
                    bounds.1 - bounds.0
                );
                measured_bounds.insert((font_name, value), bounds);
            }
        }

        for value in values {
            let foreground_centers = {
                let font = &config.fonts["combo"];
                let metrics: Vec<_> = value
                    .chars()
                    .map(|ch| text_glyph_metrics(ch, 157.5, Some(font), &config.loaded_assets))
                    .collect();
                let positions = text_glyph_positions(
                    reference_rect_width,
                    &metrics,
                    UiTextAlign::Center,
                    true,
                    font.center_spacing,
                    Some(157.5 * f64::from(font.glyph_width) / f64::from(font.glyph_height.max(1))),
                );
                positions
                    .iter()
                    .zip(&metrics)
                    .map(|(left, (glyph_width, _, _))| left + glyph_width * 0.5)
                    .collect::<Vec<_>>()
            };
            let shadow_centers = {
                let font = &config.fonts["combo-burst"];
                let metrics: Vec<_> = value
                    .chars()
                    .map(|ch| text_glyph_metrics(ch, 221.55, Some(font), &config.loaded_assets))
                    .collect();
                let positions = text_glyph_positions(
                    reference_rect_width,
                    &metrics,
                    UiTextAlign::Center,
                    true,
                    font.center_spacing,
                    Some(
                        221.55 * f64::from(font.glyph_width) / f64::from(font.glyph_height.max(1)),
                    ),
                );
                positions
                    .iter()
                    .zip(&metrics)
                    .map(|(left, (glyph_width, _, _))| left + glyph_width * 0.5)
                    .collect::<Vec<_>>()
            };
            assert_eq!(foreground_centers, shadow_centers, "value={value}");
            assert!(
                measured_bounds[&("combo-burst", value)].1 > measured_bounds[&("combo", value)].1
            );
        }
    }

    #[test]
    fn sono_overlay_combo_slot_spacing_scales_once_with_the_reference_pop() {
        let config = profile("default.json");
        let output = [1920.0, 1080.0];
        let screen = UiRect {
            x: 0.0,
            y: 0.0,
            width: output[0],
            height: output[1],
        };
        let root = ui_root_matrix(output[0], output[1], config.design_size, config.fit);
        let combo_root = responsive_group_matrix(&config, "combo", output[0], output[1], root);
        let source_slot_spacing = 72.0 * 1.5;
        let values = ["8", "11", "46", "48", "100", "106", "111", "1000"];
        let ages_in_reference_frames = [0.0, 1.0, 4.0, 8.0, 11.0, 13.0, 14.0];

        for value in values {
            for age in ages_in_reference_frames {
                let time = age / 60.0;
                let mut gameplay = UiGameplayState {
                    combo: value.parse().unwrap(),
                    latest_event_time: Some(0.0),
                    ..UiGameplayState::default()
                };
                apply_note_hud_animations(&mut gameplay, time, 60.0);
                let state = UiFrameState {
                    time,
                    frame_rate: 60.0,
                    gameplay: Some(gameplay),
                    ..UiFrameState::default()
                };
                let count = value.chars().count();

                for (element_id, font_name, font_size, expected_scale) in [
                    ("combo-value", "combo", 157.5, (0.5 + age / 16.0).min(1.0)),
                    (
                        "combo-shadow-value",
                        "combo-burst",
                        221.55,
                        (0.5 + age / 16.0).min(1.0),
                    ),
                    ("combo-event-pop", "combo", 157.5, 0.5 + age / 16.0),
                    ("combo-event-pulse", "combo-burst", 221.55, 0.5 + age / 16.0),
                ] {
                    let element = config
                        .elements
                        .iter()
                        .find(|element| element.id == element_id)
                        .unwrap();
                    let (rect, world) =
                        element_geometry(element, screen, combo_root, &config, &state, 1.0);
                    let font = &config.fonts[font_name];
                    let metrics = value
                        .chars()
                        .map(|ch| {
                            text_glyph_metrics(ch, font_size, Some(font), &config.loaded_assets)
                        })
                        .collect::<Vec<_>>();
                    let positions = text_glyph_positions(
                        rect.width,
                        &metrics,
                        UiTextAlign::Center,
                        true,
                        font.center_spacing,
                        Some(
                            font_size * f64::from(font.glyph_width)
                                / f64::from(font.glyph_height.max(1)),
                        ),
                    );
                    let centers = positions
                        .iter()
                        .zip(&metrics)
                        .map(|(left, (glyph_width, _, _))| {
                            world.a * (left + glyph_width * 0.5)
                                + world.c * (rect.height * 0.5)
                                + world.tx
                        })
                        .collect::<Vec<_>>();
                    let output_spacing = source_slot_spacing * expected_scale;

                    for (index, center) in centers.iter().enumerate() {
                        let expected = 1633.5
                            + (index as f64 - (count.saturating_sub(1) as f64) * 0.5)
                                * output_spacing;
                        assert!(
                            (center - expected).abs() < 1e-7,
                            "{element_id} {value} age={age}: center {center} != {expected}"
                        );
                    }
                    assert!(
                        (centers.first().unwrap() + centers.last().unwrap() - 2.0 * 1633.5).abs()
                            < 1e-7,
                        "{element_id} {value} age={age} drifted away from the Lua group center"
                    );

                    if count > 1 {
                        for pair in centers.windows(2) {
                            assert!(
                                ((pair[1] - pair[0]) - output_spacing).abs() < 1e-7,
                                "{element_id} {value} age={age}: effective spacing {} != {output_spacing}",
                                pair[1] - pair[0]
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sono_overlay_ap_combo_assets_follow_the_lua_ap_branch() {
        let config = profile("default.json");
        let source = include_str!("../TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2");

        assert!(source.contains("--track@ap:APコンボ"));
        assert!(source.contains("obj.load(\"image\", PED_DATA.path..\"/combo/pe.png\")"));
        assert!(source.contains("obj.load(\"image\", PED_DATA.path..\"/combo/pt.png\")"));
        assert!(source.contains("obj.load(\"image\", PED_DATA.path..\"/combo/nt.png\")"));
        assert!(source.contains("PED_DATA.path..\"/combo/p\"..digit..\".png\""));
        assert!(source.contains("PED_DATA.path..\"/combo/n\"..digit..\".png\""));

        assert_eq!(
            config.assets["combo-glow"],
            PathBuf::from("pack://combo.pe")
        );
        assert_eq!(
            config.assets["combo-title"],
            PathBuf::from("pack://combo.pt")
        );
        for digit in '0'..='9' {
            assert_eq!(
                config.fonts["combo"].glyph_assets[&digit.to_string()],
                format!("combo-digit-{digit}")
            );
            assert_eq!(
                config.fonts["combo-burst"].glyph_assets[&digit.to_string()],
                format!("combo-burst-{digit}")
            );
        }
        assert!(config
            .elements
            .iter()
            .find(|element| element.id == "combo-special-burst")
            .is_some_and(|element| !element.enabled));
    }

    #[test]
    fn sono_overlay_score_and_life_shadows_share_lua_digit_centers() {
        let config = profile("default.json");
        let element = |id: &str| {
            config
                .elements
                .iter()
                .find(|element| element.id == id)
                .unwrap_or_else(|| panic!("missing {id}"))
        };
        for (shadow_id, foreground_id) in [
            ("score-shadow", "score-value"),
            ("life-value-shadow", "life-value"),
        ] {
            assert_eq!(
                element(shadow_id).layout,
                element(foreground_id).layout,
                "Sono-Overlay draws {shadow_id} and {foreground_id} at the same coordinates, anchor, and size"
            );
        }

        let score_spacing = 22.0 * 1.5;
        for font_name in ["score", "score-shadow"] {
            let font = &config.fonts[font_name];
            assert_eq!(font.center_spacing, Some(score_spacing));
            for digit in '0'..='9' {
                let source_height = if font_name == "score-shadow" {
                    56.0
                } else {
                    44.0
                };
                let glyph_scale = font
                    .glyph_scales
                    .get(&digit.to_string())
                    .copied()
                    .unwrap_or(1.0);
                assert!((44.0 * glyph_scale - source_height).abs() < 1e-6);
            }
        }

        let score_value = element("score-value");
        let score_shadow = element("score-shadow");
        let score_centers = |font_name: &str, font_size: f64, id: &str| {
            let font = &config.fonts[font_name];
            let text = "00089532";
            let metrics: Vec<_> = text
                .chars()
                .map(|ch| text_glyph_metrics(ch, font_size, Some(font), &config.loaded_assets))
                .collect();
            let width = element(id).layout.width * 1920.0;
            let positions = text_glyph_positions(
                width,
                &metrics,
                UiTextAlign::Left,
                true,
                font.center_spacing,
                Some(font_size * f64::from(font.glyph_width) / f64::from(font.glyph_height.max(1))),
            );
            positions
                .iter()
                .zip(&metrics)
                .map(|(left, (glyph_width, _, _))| left + glyph_width * 0.5)
                .collect::<Vec<_>>()
        };
        let score_foreground_centers = score_centers("score", 44.0, &score_value.id);
        let score_shadow_centers = score_centers("score-shadow", 44.0, &score_shadow.id);
        assert_eq!(score_foreground_centers.len(), score_shadow_centers.len());
        assert!(score_foreground_centers
            .iter()
            .zip(&score_shadow_centers)
            .all(|(foreground, shadow)| (foreground - shadow).abs() < 1e-9));
        assert!(score_foreground_centers.windows(2).all(|pair| (pair[1]
            - pair[0]
            - score_spacing)
            .abs()
            < 1e-9));

        let life_spacing = 127.0 * (17.33 / 100.0);
        let life_value = element("life-value");
        let life_shadow = element("life-value-shadow");
        let life_centers = |font_name: &str, id: &str| {
            let font = &config.fonts[font_name];
            let metrics: Vec<_> = "1000"
                .chars()
                .map(|ch| text_glyph_metrics(ch, 34.0, Some(font), &config.loaded_assets))
                .collect();
            let width = element(id).layout.width * 1920.0;
            let positions = text_glyph_positions(
                width,
                &metrics,
                UiTextAlign::Right,
                true,
                font.center_spacing,
                Some(34.0 * f64::from(font.glyph_width) / f64::from(font.glyph_height.max(1))),
            );
            positions
                .iter()
                .zip(&metrics)
                .map(|(left, (glyph_width, _, _))| left + glyph_width * 0.5)
                .collect::<Vec<_>>()
        };
        let life_foreground_centers = life_centers("life", &life_value.id);
        let life_shadow_centers = life_centers("life-shadow", &life_shadow.id);
        assert_eq!(life_foreground_centers.len(), life_shadow_centers.len());
        assert!(life_foreground_centers
            .iter()
            .zip(&life_shadow_centers)
            .all(|(foreground, shadow)| (foreground - shadow).abs() < 1e-9));
        assert!(life_foreground_centers
            .windows(2)
            .all(|pair| (pair[1] - pair[0] - life_spacing).abs() < 1e-9));

        for digit in '0'..='9' {
            let font = &config.fonts["life-shadow"];
            let scale = font.glyph_scales[&digit.to_string()];
            assert!((34.0 * scale - (34.0 * 1466.0 / 1333.0)).abs() < 1e-6);
        }

        // Opt-in visual evidence for the same compositor and reference assets.
        // Normal test runs remain filesystem-free.
        if let Some(capture_dir) = std::env::var_os("SONO_RENDERER_SHADOW_CAPTURE_DIR") {
            let capture_dir = std::path::PathBuf::from(capture_dir);
            std::fs::create_dir_all(&capture_dir).unwrap();
            let state = UiFrameState {
                gameplay: Some(UiGameplayState {
                    score: 89_532.0,
                    life: 1_000.0,
                    ..UiGameplayState::default()
                }),
                ..UiFrameState::default()
            };
            let render_pair = |capture_config: &RendererUiConfig| {
                let mut capture_config = capture_config.clone();
                capture_config.elements.retain(|element| {
                    matches!(
                        element.id.as_str(),
                        "score-shadow" | "score-value" | "life-value-shadow" | "life-value"
                    )
                });
                let mut rgb = vec![18_u8; 1920 * 1080 * 3];
                render_gameplay_ui(&mut rgb, 1920, 1080, &capture_config, &state).unwrap();
                image::RgbImage::from_raw(1920, 1080, rgb).unwrap()
            };

            let mut before = config.clone();
            before
                .elements
                .iter_mut()
                .find(|element| element.id == "score-shadow")
                .unwrap()
                .layout
                .x = 0.087;
            before
                .elements
                .iter_mut()
                .find(|element| element.id == "score-shadow")
                .unwrap()
                .layout
                .y = 0.077;
            before
                .elements
                .iter_mut()
                .find(|element| element.id == "life-value-shadow")
                .unwrap()
                .layout
                .x = 0.927;
            before
                .elements
                .iter_mut()
                .find(|element| element.id == "life-value-shadow")
                .unwrap()
                .layout
                .y = 0.013;
            for name in ["score", "score-shadow", "life", "life-shadow"] {
                let font = before.fonts.get_mut(name).unwrap();
                font.center_spacing = None;
                font.glyph_scales.clear();
            }
            before.fonts.get_mut("life-shadow").unwrap().advance = 1.0;

            render_pair(&before)
                .save(capture_dir.join("score-life-shadow-before.png"))
                .unwrap();
            render_pair(&config)
                .save(capture_dir.join("score-life-shadow-after.png"))
                .unwrap();
        }
    }

    #[test]
    fn bundled_life_demo_uses_authored_eased_values_and_bitmap_sign_glyphs() {
        let profile = profile("life-timeline-demo.json");
        assert_eq!(
            profile.fonts["life"].glyph_assets["-"],
            "life-negative-sign"
        );
        assert_eq!(
            profile.fonts["life-shadow"].glyph_assets["-"],
            "life-negative-shadow"
        );
        for (time, expected) in [(0.0, 1000.0), (10.0, 0.0), (100.0, -100.0)] {
            let state = profile
                .resolve_presentation_state(&UiFrameState {
                    time,
                    ..UiFrameState::default()
                })
                .unwrap();
            assert_eq!(state.presentation_values["life"], expected);
        }
        let halfway = profile
            .resolve_presentation_state(&UiFrameState {
                time: 5.0,
                ..UiFrameState::default()
            })
            .unwrap();
        assert_eq!(halfway.presentation_values["life"], 250.0);
    }

    #[test]
    fn timeline_keeps_runtime_life_unchanged_and_resolves_numeric_and_category_tracks() {
        let mut config = RendererUiConfig::default();
        config.timeline.tracks = vec![
            UiTimelineTrack {
                id: "life-display".into(),
                kind: UiTimelineTrackKind::Numeric {
                    keyframes: vec![
                        UiNumericKeyframe {
                            time: 0.0,
                            value: 1000.0,
                            easing: UiEasing::EaseOut,
                        },
                        UiNumericKeyframe {
                            time: 10.0,
                            value: 0.0,
                            easing: UiEasing::CubicBezier {
                                x1: 2500,
                                y1: 1000,
                                x2: 7500,
                                y2: 10_000,
                            },
                        },
                        UiNumericKeyframe {
                            time: 100.0,
                            value: -100.0,
                            easing: UiEasing::Linear,
                        },
                    ],
                },
            },
            UiTimelineTrack {
                id: "judgement-style".into(),
                kind: UiTimelineTrackKind::Categorical {
                    keyframes: vec![
                        UiCategoricalKeyframe {
                            time: 0.0,
                            value: "perfect-style".into(),
                        },
                        UiCategoricalKeyframe {
                            time: 50.0,
                            value: "great-style".into(),
                        },
                    ],
                },
            },
        ];
        config.validate().unwrap();

        let mut input = UiFrameState {
            time: 5.0,
            gameplay: Some(UiGameplayState {
                life: 900.0,
                ..UiGameplayState::default()
            }),
            ..UiFrameState::default()
        };
        let halfway = config.resolve_presentation_state(&input).unwrap();
        assert_eq!(halfway.presentation_values["life-display"], 250.0);
        assert_eq!(
            halfway.presentation_categories["judgement-style"],
            "perfect-style"
        );
        assert_eq!(halfway.gameplay.as_ref().unwrap().life, 900.0);

        input.time = 50.0;
        let later = config.resolve_presentation_state(&input).unwrap();
        assert!(later.presentation_values["life-display"] < 0.0);
        assert_eq!(
            later.presentation_categories["judgement-style"],
            "great-style"
        );
        assert_eq!(later.gameplay.as_ref().unwrap().life, 900.0);

        let json = serde_json::to_value(&config).unwrap();
        let round_trip: RendererUiConfig = serde_json::from_value(json).unwrap();
        assert_eq!(round_trip.timeline, config.timeline);
    }

    #[test]
    fn timeline_rejects_duplicate_times_and_unordered_bezier_controls() {
        let mut config = RendererUiConfig::default();
        config.timeline.tracks.push(UiTimelineTrack {
            id: "broken".into(),
            kind: UiTimelineTrackKind::Numeric {
                keyframes: vec![
                    UiNumericKeyframe {
                        time: 2.0,
                        value: 1.0,
                        easing: UiEasing::CubicBezier {
                            x1: 9000,
                            y1: 0,
                            x2: 1000,
                            y2: 10_000,
                        },
                    },
                    UiNumericKeyframe {
                        time: 2.0,
                        value: 0.0,
                        easing: UiEasing::Linear,
                    },
                ],
            },
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn standalone_profile_rejects_an_unsupported_future_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future-ui.json");
        std::fs::write(
            &path,
            br#"{"format":"sono-renderer-gameplay-ui-profile","schema_version":2}"#,
        )
        .unwrap();
        assert!(RendererUiConfig::load_profile(path)
            .unwrap_err()
            .to_string()
            .contains("newer than supported"));
    }

    fn rgb(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        color
            .into_iter()
            .cycle()
            .take(width as usize * height as usize * 3)
            .collect()
    }

    #[test]
    fn additive_ui_blend_matches_the_combo_pop_compositing_path() {
        let mut normal = vec![100, 100, 100];
        blend_pixel(
            &mut normal,
            1,
            1,
            0,
            0,
            [200, 40, 240, 128],
            1.0,
            UiBlendMode::Normal,
            None,
        );
        let mut additive = vec![100, 100, 100];
        blend_pixel(
            &mut additive,
            1,
            1,
            0,
            0,
            [200, 40, 240, 128],
            1.0,
            UiBlendMode::Add,
            None,
        );
        assert_eq!(normal, [150, 69, 170]);
        assert_eq!(additive, [200, 120, 220]);
    }

    #[test]
    fn isolated_additive_combo_group_composites_over_gameplay_after_internal_blend() {
        let mut config = RendererUiConfig::default();
        config.enabled = true;
        config.design_size = [1, 1];
        config.elements = vec![
            UiElement {
                id: "combo-base".into(),
                z_index: 0,
                isolation_group: Some("combo-number".into()),
                kind: UiElementKind::Rectangle {
                    color: [200, 0, 0, 128],
                    border: None,
                    radius: 0.0,
                },
                ..UiElement::default()
            },
            UiElement {
                id: "combo-pop".into(),
                z_index: 1,
                blend_mode: UiBlendMode::Add,
                isolation_group: Some("combo-number".into()),
                kind: UiElementKind::Rectangle {
                    color: [0, 200, 0, 128],
                    border: None,
                    radius: 0.0,
                },
                ..UiElement::default()
            },
        ];
        let mut frame = vec![10, 10, 10];
        render_custom_ui(&mut frame, 1, 1, &config, &UiFrameState::default()).unwrap();
        assert_eq!(frame, [102, 102, 2]);
    }

    #[test]
    fn combo_foreground_and_shadow_render_as_independent_reference_layers() {
        let mut config = profile("default.json");
        let state = UiFrameState {
            time: 0.375,
            frame_rate: 30.0,
            gameplay: Some(UiGameplayState {
                combo: 8,
                ..UiGameplayState::default()
            }),
            ..UiFrameState::default()
        };

        config
            .elements
            .retain(|element| element.id == "combo-shadow-value");
        let mut shadow_only = rgb(1920, 1080, [0, 0, 0]);
        render_gameplay_ui(&mut shadow_only, 1920, 1080, &config, &state).unwrap();
        let shadow_peak = shadow_only.iter().copied().max().unwrap_or(0);
        assert!(shadow_peak > 0, "the b8 shadow glyph must be visible");
        assert!(
            shadow_peak <= 104,
            "the shadow retains the source PNG's maximum alpha (104), got {shadow_peak}"
        );

        config = profile("default.json");
        config
            .elements
            .retain(|element| element.id == "combo-value");
        let mut foreground_only = rgb(1920, 1080, [0, 0, 0]);
        render_gameplay_ui(&mut foreground_only, 1920, 1080, &config, &state).unwrap();
        let foreground_peak = foreground_only.iter().copied().max().unwrap_or(0);
        assert!(
            foreground_peak > shadow_peak,
            "the p8 foreground ({foreground_peak}) must render independently above the b8 alpha peak ({shadow_peak})"
        );
        assert!(
            foreground_peak >= 250,
            "p8 should retain opaque foreground pixels"
        );
    }

    #[test]
    fn bundled_sono_overlay_hud_renders_at_landscape_and_portrait_sizes() {
        let mut config = RendererUiConfig::reference_default();
        assert_eq!(config.profile_name, "Asset-backed Gameplay HUD");
        assert!(config.enabled);
        assert_eq!(config.elements.len(), 28);
        assert_eq!(
            config.scoring.as_ref().map(|scoring| scoring.source),
            Some(UiScoringSource::SonoOverlay),
            "the reference HUD uses only the explicitly supported Sono-Overlay chart forecast"
        );
        let score_panel = config
            .elements
            .iter()
            .find(|element| element.id == "score-panel")
            .expect("reference score panel");
        assert_eq!(score_panel.layout.x, 0.02);
        assert_eq!(score_panel.layout.y, 0.0);
        assert_eq!(score_panel.layout.width, 0.345);
        assert_eq!(score_panel.layout.height, 0.1322);
        let combo_value = config
            .elements
            .iter()
            .find(|element| element.id == "combo-value")
            .expect("reference combo counter");
        assert_eq!(combo_value.layout.x, 0.85078125);
        assert_eq!(combo_value.layout.y, 0.4412037037037037);
        assert!(matches!(
            &combo_value.kind,
            UiElementKind::Counter { max_digits: Some(4), font: Some(font), font_size, .. }
                if font == "combo" && *font_size == 157.5
        ));
        assert!(combo_value.animations.iter().all(|animation| matches!(
            animation.value_from_state.as_ref(),
            Some(UiExpression::Source {
                source: UiStateSource::GameplayVariable { name }
            }) if name == "combo_pop_scale"
        )));
        let combo_pulse = config
            .elements
            .iter()
            .find(|element| element.id == "combo-event-pulse")
            .expect("reference combo pulse layer");
        assert_eq!(combo_pulse.layout.x, combo_value.layout.x);
        assert_eq!(combo_pulse.layout.y, combo_value.layout.y);
        assert_eq!(combo_pulse.z_index, 2);
        assert!(matches!(
            &combo_pulse.kind,
            UiElementKind::Counter { max_digits: Some(4), font: Some(font), font_size, .. }
                if font == "combo-burst" && *font_size == 221.55
        ));
        assert_eq!(combo_pulse.blend_mode, UiBlendMode::Normal);
        let combo_pop = config
            .elements
            .iter()
            .find(|element| element.id == "combo-event-pop")
            .expect("source-defined additive combo foreground pop layer");
        assert_eq!(combo_pop.blend_mode, UiBlendMode::Add);
        let combo_shadow = config
            .elements
            .iter()
            .find(|element| element.id == "combo-shadow-value")
            .expect("persistent AP shadow glyphs");
        assert_eq!(combo_shadow.z_index, 0);
        assert!(combo_shadow.animations.iter().any(|animation| matches!(
            animation.value_from_state.as_ref(),
            Some(UiExpression::Source {
                source: UiStateSource::GameplayVariable { name }
            }) if name == "ap_combo_alpha"
        )));
        let combo_pop = config
            .elements
            .iter()
            .find(|element| element.id == "combo-event-pop")
            .expect("foreground combo pop glyphs");
        assert_eq!(combo_pop.z_index, 3);
        assert_eq!(combo_pop.blend_mode, UiBlendMode::Add);
        assert!(matches!(
            &combo_pop.kind,
            UiElementKind::Counter { font: Some(font), font_size, .. }
                if font == "combo" && *font_size == 157.5
        ));
        let judgment = config
            .elements
            .iter()
            .find(|element| element.id == "judgement-perfect")
            .expect("reference judgment artwork");
        assert_eq!(judgment.layout.x, 0.5);
        assert_eq!(judgment.layout.y, 0.6180555555555556);
        assert_eq!(judgment.layout.width, 0.16145833333333334);
        assert_eq!(judgment.layout.height, 0.075);
        assert_eq!(judgment.opacity, 1.0);
        assert!(judgment.visible_when.is_some());
        assert_eq!(judgment.opacity, 1.0);
        assert!(judgment.animations.iter().any(|animation| {
            animation.property == UiAnimationProperty::ScaleX
                && matches!(
                    animation.value_from_state.as_ref(),
                    Some(UiExpression::Source {
                        source: UiStateSource::GameplayVariable { name }
                    }) if name == "judgment_scale"
                )
        }));
        assert!(judgment.animations.iter().any(|animation| {
            animation.property == UiAnimationProperty::ScaleY
                && matches!(
                    animation.value_from_state.as_ref(),
                    Some(UiExpression::Source {
                        source: UiStateSource::GameplayVariable { name }
                    }) if name == "judgment_scale"
                )
        }));
        assert!(judgment
            .animations
            .iter()
            .all(|animation| animation.property != UiAnimationProperty::Opacity));
        let life_gauge = config
            .elements
            .iter()
            .find(|element| element.id == "life-gauge")
            .expect("reference life gauge");
        assert!(life_gauge.style_rules.iter().any(|rule| {
            rule.asset.as_deref() == Some("life-danger")
                && matches!(
                    &rule.when,
                    UiCondition::LessOrEqual {
                        left: UiExpression::Source {
                            source: UiStateSource::Life
                        },
                        right: UiExpression::Constant { value }
                    } if *value == 200.0
                )
        }));
        config.enabled = true;
        config.load_external_data().unwrap();

        for (width, height) in [
            (1920, 1080),
            (1440, 1080),
            (1280, 720),
            (640, 360),
            (1080, 1920),
        ] {
            let mut frame = rgb(width, height, [0, 0, 0]);
            render_gameplay_ui(&mut frame, width, height, &config, &UiFrameState::default())
                .unwrap();
            assert!(
                frame.iter().any(|value| *value != 0),
                "default HUD produced no visible pixels at {width}x{height}"
            );
            assert!(
                config
                    .elements
                    .iter()
                    .any(|element| element.id == "score-panel"),
                "expected the Sono-Overlay score panel at {width}x{height}"
            );
        }
    }

    #[test]
    fn sono_overlay_reference_geometry_is_preserved_at_16_9_and_adapted_at_4_3() {
        let config = RendererUiConfig::reference_default();
        let reference = config.responsive_layouts.get("4:3").unwrap();
        let at_16_9 = ui_root_matrix(1920.0, 1080.0, config.design_size, config.fit);
        let at_4_3_root = ui_root_matrix(1440.0, 1080.0, config.design_size, config.fit);
        let expected_4_3 = [
            ("score", [376.5, 69.0], [277.5, 50.5], 112.0 / 150.0),
            ("life", [1665.0, 62.5], [1252.0, 47.0], 12.97 / 17.33),
            ("combo", [1633.5, 476.5], [1225.0, 493.0], 112.0 / 150.0),
            ("judgment", [960.0, 667.5], [720.0, 635.0], 110.0 / 150.0),
        ];
        for (name, source, target, source_scale) in expected_4_3 {
            let group = &reference.groups[name];
            assert_eq!(group.reference_center, source);
            assert_eq!(group.alternate_center, target);
            assert!((group.alternate_scale - source_scale).abs() < 1.0e-12);

            let matrix_16_9 = responsive_group_matrix(&config, name, 1920.0, 1080.0, at_16_9);
            let point_16_9 = [
                matrix_16_9.a * source[0] + matrix_16_9.tx,
                matrix_16_9.d * source[1] + matrix_16_9.ty,
            ];
            assert_eq!(point_16_9, source, "16:9 {name} group must be unchanged");

            let matrix_4_3 = responsive_group_matrix(&config, name, 1440.0, 1080.0, at_4_3_root);
            let point_4_3 = [
                matrix_4_3.a * source[0] + matrix_4_3.tx,
                matrix_4_3.d * source[1] + matrix_4_3.ty,
            ];
            assert!(
                (point_4_3[0] - target[0]).abs() < 1.0e-9
                    && (point_4_3[1] - target[1]).abs() < 1.0e-9,
                "4:3 {name} center {point_4_3:?} should equal {target:?}"
            );
            assert!((matrix_4_3.a - source_scale).abs() < 1.0e-12);
            assert!((matrix_4_3.d - source_scale).abs() < 1.0e-12);
        }

        // In the source Lua composition, the parent life object is centered at
        // (960 + 705, 540 - 477.5) with scale 17.33%. Its 2560x600 image is
        // drawn at native size. The fill region is the authored normalized
        // rectangle [0.146875, 0.4666667, 0.6, 0.14] within that image.
        // Keep the numbers at their separate authored profile coordinates.
        let element_layout = |id: &str| {
            config
                .elements
                .iter()
                .find(|element| element.id == id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .layout
                .clone()
        };
        let life_center = [960.0 + 705.0, 540.0 + (-477.5)];
        let life_scale = 17.33 / 100.0;
        let panel_size = [2560.0 * life_scale, 600.0 * life_scale];
        let panel_top_left = [
            life_center[0] - panel_size[0] / 2.0,
            life_center[1] - panel_size[1] / 2.0,
        ];
        let panel = element_layout("life-panel");
        assert!((panel.x * 1920.0 - panel_top_left[0]).abs() < 1.0e-6);
        assert!((panel.y * 1080.0 - panel_top_left[1]).abs() < 1.0e-6);
        assert!((panel.width * 1920.0 - panel_size[0]).abs() < 1.0e-6);
        assert!((panel.height * 1080.0 - panel_size[1]).abs() < 1.0e-6);

        let fill_source = [0.146875, 0.4666666666666667, 0.6, 0.14];
        let expected_gauge = [
            panel_top_left[0] + fill_source[0] * 2560.0 * life_scale,
            panel_top_left[1] + fill_source[1] * 600.0 * life_scale,
            fill_source[2] * 2560.0 * life_scale,
            fill_source[3] * 600.0 * life_scale,
        ];
        let gauge = element_layout("life-gauge");
        let overflow_gauge = element_layout("life-overflow-gauge");
        for layout in [gauge, overflow_gauge] {
            assert!((layout.x * 1920.0 - expected_gauge[0]).abs() < 1.0e-6);
            assert!((layout.y * 1080.0 - expected_gauge[1]).abs() < 1.0e-6);
            assert!((layout.width * 1920.0 - expected_gauge[2]).abs() < 1.0e-6);
            assert!((layout.height * 1080.0 - expected_gauge[3]).abs() < 1.0e-6);
        }
        assert!((element_layout("life-value").y * 1080.0 - 10.8).abs() < 1.0e-6);
        assert!((element_layout("life-value-shadow").y * 1080.0 - 10.8).abs() < 1.0e-6);

        // Intermediate aspect ratios interpolate between the authored source
        // layouts; outside the reference range they clamp to the nearest
        // layout while the root contain fit keeps sprites undistorted.
        let at_3_2 = ui_root_matrix(1920.0, 1280.0, config.design_size, config.fit);
        assert_eq!(at_3_2.a, 1.0);
        assert_eq!(at_3_2.d, 1.0);
        assert_eq!(at_3_2.ty, 100.0);
        let at_3_2_group = responsive_group_matrix(&config, "score", 1920.0, 1280.0, at_3_2);
        assert!(at_3_2_group.a < 1.0 && at_3_2_group.a > 0.746);

        assert_eq!(config.elements.len(), 28);
        assert!(config
            .elements
            .iter()
            .all(|element| element.responsive_group.is_some()));
    }

    #[test]
    fn sono_overlay_ap_glow_and_shadow_use_the_source_sinusoid() {
        let mut state = UiGameplayState::default();
        for (time, expected) in [(0.0, 0.5), (0.375, 1.0), (1.125, 0.0), (1.5, 0.5)] {
            apply_note_hud_animations(&mut state, time, 30.0);
            assert!(
                (state.variables["ap_combo_alpha"] - expected).abs() < 1.0e-12,
                "time={time} actual={}",
                state.variables["ap_combo_alpha"]
            );
            assert_eq!(state.variables["combo_shadow_pulse_opacity"], 0.0);
        }
        for (time, expected) in [(0.0, 0.5), (0.5, 1.0), (1.5, 0.0)] {
            apply_note_hud_animations(&mut state, time, 30.0);
            assert!((state.variables["life_overflow_alpha"] - expected).abs() < 1.0e-12);
        }
        state.combo = 1;
        state.variables.insert("combo_event_time".into(), 1.125);
        apply_note_hud_animations(&mut state, 1.125, 30.0);
        assert_eq!(state.variables["combo_pulse_opacity"], 1.0);
        assert_eq!(state.variables["ap_combo_alpha"], 0.0);
        assert_eq!(state.variables["combo_shadow_pulse_opacity"], 0.0);
    }

    #[test]
    fn overlay_combo_and_judgment_curves_are_timestamp_based_and_seekable() {
        let mut before = UiGameplayState::default();
        apply_note_hud_animations(&mut before, 2.0, 30.0);
        assert_eq!(before.variables["combo_pop_scale"], 1.0);
        assert_eq!(before.variables["combo_pulse_opacity"], 0.0);
        assert_eq!(before.variables["judgment_opacity"], 0.0);
        assert_eq!(before.variables["judgment_visible"], 0.0);

        let mut at_event = UiGameplayState {
            combo: 1,
            latest_event_time: Some(2.0),
            judgment: Some("PERFECT".into()),
            judgment_time: Some(2.0),
            ..UiGameplayState::default()
        };
        at_event.variables.insert("combo_event_time".into(), 2.0);
        apply_note_hud_animations(&mut at_event, 2.0, 30.0);
        assert_eq!(at_event.variables["combo_pop_scale"], 0.5);
        assert_eq!(at_event.variables["combo_pulse_scale"], 0.5);
        assert_eq!(at_event.variables["combo_pulse_opacity"], 1.0);
        assert_eq!(at_event.variables["judgment_opacity"], 0.0);
        assert_eq!(at_event.variables["judgment_scale"], 1.0);
        assert_eq!(at_event.variables["judgment_visible"], 0.0);

        let mut during = at_event.clone();
        apply_note_hud_animations(&mut during, 2.0 + 8.0 / 30.0, 30.0);
        assert!((during.variables["combo_pop_scale"] - 1.0).abs() < 1e-12);
        assert!((during.variables["combo_pulse_scale"] - 1.0).abs() < 1e-12);
        assert!((during.variables["combo_pulse_opacity"] - 6.0 / 14.0).abs() < 1e-12);
        assert_eq!(during.variables["judgment_opacity"], 1.0);
        assert_eq!(during.variables["judgment_scale"], 1.0);
        assert_eq!(during.variables["judgment_visible"], 1.0);

        let mut judgment_in = at_event.clone();
        apply_note_hud_animations(&mut judgment_in, 2.0 + 2.0 / 30.0, 30.0);
        let lua_scale_at_two = 1.0 - (-1.45 + 2.0 / 4.0_f64).powi(4);
        assert!((judgment_in.variables["judgment_scale"] - lua_scale_at_two).abs() < 1e-12);
        assert_eq!(judgment_in.variables["judgment_opacity"], 1.0);
        assert_eq!(judgment_in.variables["judgment_visible"], 1.0);

        let mut judgment_mid = at_event.clone();
        apply_note_hud_animations(&mut judgment_mid, 2.0 + 3.0 / 30.0, 30.0);
        assert!(
            (judgment_mid.variables["judgment_scale"] - (1.0 - (-0.7_f64).powi(4))).abs() < 1e-12
        );

        let mut judgment_full = at_event.clone();
        apply_note_hud_animations(&mut judgment_full, 2.0 + 5.0 / 30.0, 30.0);
        assert_eq!(judgment_full.variables["judgment_scale"], 1.0);
        assert_eq!(judgment_full.variables["judgment_opacity"], 1.0);

        let mut after = at_event.clone();
        apply_note_hud_animations(&mut after, 2.0 + 20.0 / 30.0, 30.0);
        assert_eq!(after.variables["judgment_opacity"], 0.0);
        assert_eq!(after.variables["judgment_scale"], 0.0);
        assert_eq!(after.variables["judgment_visible"], 0.0);
        assert_eq!(after.variables["combo_pulse_opacity"], 0.0);
        let mut pulse_end = at_event.clone();
        apply_note_hud_animations(&mut pulse_end, 2.0 + 14.0 / 30.0, 30.0);
        assert_eq!(pulse_end.variables["combo_pulse_opacity"], 0.0);
        assert_eq!(pulse_end.variables["combo_pulse_visible"], 0.0);
        assert_eq!(pulse_end.variables["combo_pulse_scale"], 1.375);

        let mut consecutive = UiGameplayState {
            combo: 2,
            latest_event_time: Some(2.01),
            judgment_time: Some(2.01),
            ..UiGameplayState::default()
        };
        consecutive
            .variables
            .insert("combo_event_time".into(), 2.01);
        consecutive
            .variables
            .insert("previous_combo_event_time".into(), 2.0);
        apply_note_hud_animations(&mut consecutive, 2.02, 30.0);
        assert!((consecutive.variables["combo_pop_scale"] - (0.5 + 0.6 / 16.0)).abs() < 1e-12);

        let mut simultaneous = consecutive.clone();
        simultaneous.combo = 3;
        simultaneous
            .variables
            .insert("previous_combo_event_time".into(), 2.01);
        apply_note_hud_animations(&mut simultaneous, 2.01, 30.0);
        assert_eq!(simultaneous.variables["combo_pop_scale"], 0.5);
        assert_eq!(simultaneous.variables["combo_pulse_opacity"], 1.0);

        let mut backward_seek = at_event;
        apply_note_hud_animations(&mut backward_seek, 2.0 + 1.0 / 30.0, 30.0);
        let earlier_value = backward_seek.variables["combo_pop_scale"];
        apply_note_hud_animations(&mut backward_seek, 2.0 + 20.0 / 30.0, 30.0);
        apply_note_hud_animations(&mut backward_seek, 2.0 + 1.0 / 30.0, 30.0);
        assert_eq!(backward_seek.variables["combo_pop_scale"], earlier_value);
    }

    #[test]
    fn overlay_frame_based_animation_ages_are_fps_aware_at_30_and_60() {
        for fps in [30.0, 60.0] {
            for age in [0.0, 1.0, 2.0, 4.0, 5.0, 8.0, 11.0, 13.0, 14.0, 19.0, 20.0] {
                let event_time = 7.25;
                let mut state = UiGameplayState {
                    combo: 48,
                    latest_event_time: Some(event_time),
                    judgment: Some("PERFECT".into()),
                    judgment_time: Some(event_time),
                    ..UiGameplayState::default()
                };
                state
                    .variables
                    .insert("combo_event_time".into(), event_time);
                apply_note_hud_animations(&mut state, event_time + age / fps, fps);

                let combo_base_scale = (0.5 + age / 16.0).min(1.0);
                let combo_pop_scale = 0.5 + age / 16.0;
                assert!((state.variables["combo_pop_scale"] - combo_base_scale).abs() < 1e-12);
                assert!((state.variables["combo_pulse_scale"] - combo_pop_scale).abs() < 1e-12);
                assert_eq!(
                    state.variables["combo_pulse_opacity"],
                    (1.0 - age / 14.0).clamp(0.0, 1.0)
                );
                assert_eq!(
                    state.variables["combo_pulse_visible"],
                    if age < 14.0 { 1.0 } else { 0.0 }
                );

                let expected_judgment_scale = if age < 2.0 {
                    1.0
                } else if age < 5.0 {
                    1.0 - (-1.45 + age / 4.0).powi(4)
                } else if age < 20.0 {
                    1.0
                } else {
                    0.0
                };
                let judgment_visible = (2.0..20.0).contains(&age);
                assert!(
                    (state.variables["judgment_scale"] - expected_judgment_scale).abs() < 1e-12,
                    "fps={fps} age={age}"
                );
                let expected_judgment_visibility = if judgment_visible { 1.0 } else { 0.0 };
                assert_eq!(
                    state.variables["judgment_visible"],
                    expected_judgment_visibility
                );
                assert_eq!(
                    state.variables["judgment_opacity"],
                    expected_judgment_visibility
                );
            }
        }
    }

    #[test]
    fn perfect_reference_draw_argument_animates_scale_not_transparency() {
        const WIDTH: u32 = 1920;
        const HEIGHT: u32 = 1080;
        let mut config = profile("default.json");
        config
            .elements
            .retain(|element| element.id == "judgement-perfect");

        let render_at_age = |age_frames: f64| {
            let time = age_frames / 30.0;
            let mut gameplay = UiGameplayState {
                judgment: Some("PERFECT".into()),
                judgment_time: Some(0.0),
                ..UiGameplayState::default()
            };
            apply_note_hud_animations(&mut gameplay, time, 30.0);
            let state = UiFrameState {
                time,
                frame_rate: 30.0,
                gameplay: Some(gameplay),
                ..UiFrameState::default()
            };
            let mut frame = rgb(WIDTH, HEIGHT, [0, 0, 0]);
            render_gameplay_ui(&mut frame, WIDTH, HEIGHT, &config, &state).unwrap();
            frame
        };
        let bounds = |frame: &[u8]| {
            let mut min_x = WIDTH;
            let mut min_y = HEIGHT;
            let mut max_x = 0;
            let mut max_y = 0;
            let mut peak = 0_u8;
            let mut found = false;
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let offset = ((y * WIDTH + x) * 3) as usize;
                    let pixel_peak = frame[offset..offset + 3].iter().copied().max().unwrap();
                    peak = peak.max(pixel_peak);
                    if pixel_peak > 2 {
                        found = true;
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
            }
            found.then_some((min_x, min_y, max_x, max_y, peak))
        };

        let before = render_at_age(1.0);
        assert_eq!(
            bounds(&before),
            None,
            "reference alpha is zero before frame 2"
        );
        let at_frame_two = render_at_age(2.0);
        let (min_x, min_y, max_x, max_y, peak) = bounds(&at_frame_two).unwrap();
        assert!((50..=70).contains(&(max_x - min_x + 1)));
        assert!(
            (8..=12).contains(&(max_y - min_y + 1)),
            "frame 2 rendered bounds are x={min_x}..{max_x}, y={min_y}..{max_y}"
        );
        assert!(
            peak > 200,
            "visible judgment is opaque at frame 2, peak={peak}"
        );
        let at_frame_three = render_at_age(3.0);
        let (min_x3, min_y3, max_x3, max_y3, peak3) = bounds(&at_frame_three).unwrap();
        assert!(max_x3 - min_x3 > max_x - min_x);
        assert!(max_y3 - min_y3 > max_y - min_y);
        assert!(peak3 > 200);
        let at_frame_five = render_at_age(5.0);
        let (min_x5, min_y5, max_x5, max_y5, peak5) = bounds(&at_frame_five).unwrap();
        assert!((290..=310).contains(&(max_x5 - min_x5 + 1)));
        assert!(
            (50..=58).contains(&(max_y5 - min_y5 + 1)),
            "frame 5 rendered bounds are x={min_x5}..{max_x5}, y={min_y5}..{max_y5}"
        );
        assert!(peak5 > 200);
        assert_eq!(
            at_frame_five,
            render_at_age(5.0),
            "same seek must render identically"
        );
        assert_eq!(
            bounds(&render_at_age(20.0)),
            None,
            "judgment disappears at frame 20"
        );
    }

    #[test]
    fn reference_hud_pop_and_judgment_are_visible_and_repeatable_at_arbitrary_times() {
        let config = profile("default.json");
        let render_at = |time: f64| {
            let mut gameplay = UiGameplayState {
                score: 123_456.0,
                combo: 12,
                life: 1_000.0,
                judgment: Some("PERFECT".into()),
                latest_event_time: Some(2.0),
                judgment_time: Some(2.0),
                ..UiGameplayState::default()
            };
            gameplay.variables.insert("combo_event_time".into(), 2.0);
            gameplay
                .variables
                .insert("previous_combo_event_time".into(), 1.0);
            apply_note_hud_animations(&mut gameplay, time, 30.0);
            let state = UiFrameState {
                time,
                frame_rate: 30.0,
                gameplay: Some(gameplay),
                ..UiFrameState::default()
            };
            let mut frame = rgb(960, 540, [20, 25, 36]);
            render_gameplay_ui(&mut frame, 960, 540, &config, &state).unwrap();
            frame
        };

        let at_event = render_at(2.0);
        let during_pop = render_at(2.0 + 8.0 / 30.0);
        let repeated_seek = render_at(2.0 + 8.0 / 30.0);
        assert_ne!(
            at_event, during_pop,
            "combo/judgment animation must change pixels"
        );
        assert_eq!(
            during_pop, repeated_seek,
            "seeking to one timestamp must be deterministic"
        );

        let mut combo_region_difference = 0_usize;
        for y in 150..330 {
            for x in 700..920 {
                let offset = (y * 960 + x) * 3;
                combo_region_difference += at_event[offset..offset + 3]
                    .iter()
                    .zip(&during_pop[offset..offset + 3])
                    .map(|(left, right)| left.abs_diff(*right) as usize)
                    .sum::<usize>();
            }
        }
        assert!(
            combo_region_difference > 10_000,
            "combo region did not animate enough: {combo_region_difference}"
        );
    }

    #[test]
    fn enabled_uninitialized_empty_hud_recovers_at_render_but_intentional_empty_stays_empty() {
        let mut legacy = RendererUiConfig::default();
        legacy.enabled = true;
        legacy.profile_name = "Custom".into();
        legacy.elements.clear();
        legacy.design_size = [640, 360];

        let mut frame = rgb(640, 360, [0, 0, 0]);
        render_gameplay_ui(&mut frame, 640, 360, &legacy, &UiFrameState::default()).unwrap();
        assert!(frame.iter().any(|value| *value != 0));
        assert!(
            legacy.elements.is_empty(),
            "render fallback must not mutate profile state"
        );

        legacy.intentionally_empty = true;
        let mut empty = rgb(640, 360, [0, 0, 0]);
        render_gameplay_ui(&mut empty, 640, 360, &legacy, &UiFrameState::default()).unwrap();
        assert!(empty.iter().all(|value| *value == 0));
    }

    #[test]
    fn legacy_empty_default_migrates_but_intentional_and_custom_empty_profiles_survive() {
        let mut legacy = RendererUiConfig::default();
        legacy.profile_name = "Custom".into();
        legacy.elements.clear();
        legacy.asset_pack = None;
        legacy.asset_root = None;
        legacy.assets.clear();
        legacy.fonts.clear();
        assert!(legacy.migrate_legacy_empty_default());
        assert!(!legacy.elements.is_empty());
        assert_eq!(legacy.profile_name, "Asset-backed Gameplay HUD");
        assert!(legacy
            .elements
            .iter()
            .any(|element| element.id == "score-panel"));

        let mut intentional = RendererUiConfig::default();
        intentional.profile_name = "My empty overlay".into();
        intentional.elements.clear();
        intentional.intentionally_empty = true;
        assert!(!intentional.migrate_legacy_empty_default());
        assert!(intentional.elements.is_empty());

        let mut named_empty = RendererUiConfig::default();
        named_empty.profile_name = "My empty overlay".into();
        named_empty.elements.clear();
        assert!(!named_empty.migrate_legacy_empty_default());
        assert!(named_empty.elements.is_empty());
    }

    #[test]
    fn populating_an_empty_profile_uses_the_pack_that_matches_the_hud() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = RendererUiConfig::default();
        config.asset_pack = Some(directory.path().join("unrelated-pack.json"));
        config.asset_root = Some(directory.path().to_path_buf());
        config.assets.insert(
            "unrelated-image".into(),
            PathBuf::from("pack://unrelated-image"),
        );

        assert!(config.populate_with_default_hud());
        let reference = RendererUiConfig::reference_default();
        assert_eq!(config.asset_pack, reference.asset_pack);
        assert!(config.assets.contains_key("score-panel"));
        assert!(!config.assets.contains_key("unrelated-image"));
        assert!(config
            .elements
            .iter()
            .any(|element| element.id == "score-panel"));
        assert!(config.enabled);
    }

    #[test]
    fn reference_canvas_relative_image_geometry_scales_once_at_multiple_outputs() {
        let mut config = RendererUiConfig::default();
        config.enabled = true;
        config.design_size = [1920, 1080];
        config
            .assets
            .insert("solid".into(), PathBuf::from("solid.png"));
        config.loaded_assets.insert(
            "solid".into(),
            Arc::new(UiBitmap {
                width: 1,
                height: 1,
                rgba: Arc::new(vec![245, 25, 35, 255]),
            }),
        );
        config.elements.push(UiElement {
            id: "center-quarter-panel".into(),
            layout: UiLayout {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
                ..UiLayout::default()
            },
            kind: UiElementKind::Image {
                asset: "solid".into(),
                tint: None,
            },
            ..UiElement::default()
        });

        for (width, height, expected) in [
            (1280, 720, (320, 180, 960, 540)),
            (1920, 1080, (480, 270, 1440, 810)),
            // Contain preserves the 16:9 canvas inside 4:3 and centers its
            // unused vertical space; it does not apply another scale later.
            (1024, 768, (256, 240, 768, 528)),
        ] {
            let mut frame = rgb(width, height, [0, 0, 0]);
            render_gameplay_ui(&mut frame, width, height, &config, &UiFrameState::default())
                .unwrap();
            let pixel = |x: u32, y: u32| {
                let offset = ((y * width + x) * 3) as usize;
                &frame[offset..offset + 3]
            };
            assert_eq!(pixel(expected.0, expected.1), &[245, 25, 35]);
            assert_eq!(pixel(expected.2 - 1, expected.3 - 1), &[245, 25, 35]);
            assert_eq!(pixel(expected.0 - 1, expected.1), &[0, 0, 0]);
            assert_eq!(pixel(expected.2, expected.3 - 1), &[0, 0, 0]);
        }
    }

    #[test]
    fn asset_gauge_uses_profile_source_region_mask_and_fraction() {
        let mut config = RendererUiConfig::default();
        config.enabled = true;
        config.design_size = [8, 2];
        config
            .assets
            .insert("fill".into(), PathBuf::from("fill.png"));
        config
            .assets
            .insert("mask".into(), PathBuf::from("mask.png"));
        config.loaded_assets.insert(
            "fill".into(),
            Arc::new(UiBitmap {
                width: 4,
                height: 1,
                rgba: Arc::new(vec![255, 0, 0, 255].repeat(4)),
            }),
        );
        config.loaded_assets.insert(
            "mask".into(),
            Arc::new(UiBitmap {
                width: 4,
                height: 1,
                rgba: Arc::new(vec![
                    255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255,
                ]),
            }),
        );
        config.elements.push(UiElement {
            id: "asset-backed-gauge".into(),
            layout: UiLayout {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                ..UiLayout::default()
            },
            kind: UiElementKind::Gauge {
                value: UiExpression::Constant { value: 0.75 },
                maximum: UiExpression::Constant { value: 1.0 },
                orientation: UiGaugeOrientation::LeftToRight,
                fill: [0, 0, 0, 0],
                track: [0, 0, 0, 0],
                fill_asset: Some("fill".into()),
                fill_source: Some([0.0, 0.0, 1.0, 1.0]),
                mask_asset: Some("mask".into()),
            },
            ..UiElement::default()
        });

        let mut frame = rgb(8, 2, [0, 0, 0]);
        render_gameplay_ui(&mut frame, 8, 2, &config, &UiFrameState::default()).unwrap();
        let pixel = |x: u32| {
            let offset = (x * 3) as usize;
            &frame[offset..offset + 3]
        };
        assert!(
            (180..=200).contains(&pixel(3)[0]),
            "linear mask filtering must soften the clipped alpha edge"
        );
        assert_eq!(
            pixel(5),
            &[0, 0, 0],
            "transparent mask pixel must clip fill art"
        );
        assert_eq!(
            pixel(6),
            &[0, 0, 0],
            "gauge fraction must clip later source pixels"
        );
    }

    #[test]
    fn ui_is_off_by_default_and_each_component_can_be_independent() {
        let mut frame = vec![0; 320 * 180 * 3];
        let config = RendererUiConfig::default();
        render_overlay(&mut frame, 320, 180, 0.5, 0.0, 1.0, &config, |_| Ok(7.0)).unwrap();
        assert!(frame.iter().all(|byte| *byte == 0));

        for component in 0..5 {
            let mut frame = vec![0; 320 * 180 * 3];
            let mut config = RendererUiConfig::default();
            config.enabled = true;
            config.primary_metric.enabled = component == 0;
            config.secondary_metric.enabled = component == 1;
            config.combo.enabled = component == 2;
            config.judgment.enabled = component == 3;
            config.progress = component == 4;
            render_overlay(&mut frame, 320, 180, 0.5, 0.0, 1.0, &config, |_| Ok(7.0)).unwrap();
            assert!(frame.iter().any(|byte| *byte != 0), "component {component}");
        }
    }

    #[test]
    fn memory_and_fixed_metric_providers_round_trip_as_configuration() {
        let json = serde_json::json!({"kind":"memory","block_id":2000,"index":7});
        let provider: UiValueProvider = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(provider).unwrap(), json);
        RendererUiConfig::default().validate().unwrap();
        assert!(RendererUiConfig {
            primary_metric: UiMetric {
                maximum: Some(0.0),
                ..UiMetric::default()
            },
            ..RendererUiConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn transformed_shapes_keep_local_offsets_for_text_glyphs_and_directional_gauges() {
        let mut frame = rgb(20, 20, [0, 0, 0]);
        let identity = UiMatrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
            opacity: 1.0,
            blend_mode: UiBlendMode::Normal,
        };
        draw_transformed_rect(
            &mut frame,
            20,
            20,
            identity,
            UiRect {
                x: 5.0,
                y: 7.0,
                width: 3.0,
                height: 4.0,
            },
            [255, 0, 0, 255],
            0.0,
            None,
        );
        let at = (7 * 20 + 5) * 3;
        assert_eq!(&frame[at..at + 3], &[255, 0, 0]);
        assert_eq!(&frame[0..3], &[0, 0, 0]);
        assert_eq!(
            transformed_bounds(
                identity,
                UiRect {
                    x: 5.0,
                    y: 7.0,
                    width: 3.0,
                    height: 4.0,
                },
                20,
                20
            ),
            Some((5, 7, 8, 11))
        );
    }

    #[test]
    fn animated_scale_transforms_bitmap_text_instead_of_only_its_alignment_box() {
        fn rendered_bounds(scale: f64) -> (usize, usize) {
            let mut config = RendererUiConfig::default();
            config.enabled = true;
            config.mode = GameplayUiMode::Custom;
            config.intentionally_empty = false;
            config.design_size = [100, 80];
            config.elements.push(UiElement {
                id: "animated-text".into(),
                layout: UiLayout {
                    x: 0.5,
                    y: 0.5,
                    width: 0.8,
                    height: 0.8,
                    anchor: UiAnchor::Center,
                    ..UiLayout::default()
                },
                animations: vec![
                    UiAnimation {
                        property: UiAnimationProperty::ScaleX,
                        from: scale,
                        to: scale,
                        ..UiAnimation::default()
                    },
                    UiAnimation {
                        property: UiAnimationProperty::ScaleY,
                        from: scale,
                        to: scale,
                        ..UiAnimation::default()
                    },
                ],
                kind: UiElementKind::Text {
                    text: "I".into(),
                    value: None,
                    color: [255, 255, 255, 255],
                    font_size: 42.0,
                    font: None,
                    align: UiTextAlign::Center,
                },
                ..UiElement::default()
            });
            let mut frame = rgb(100, 80, [0, 0, 0]);
            render_gameplay_ui(&mut frame, 100, 80, &config, &UiFrameState::default()).unwrap();
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (100, 80, 0, 0);
            for y in 0..80 {
                for x in 0..100 {
                    let offset = (y * 100 + x) * 3;
                    if frame[offset..offset + 3] != [0, 0, 0] {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                }
            }
            (max_x - min_x + 1, max_y - min_y + 1)
        }

        let normal = rendered_bounds(1.0);
        let popped = rendered_bounds(0.5);
        assert!(
            normal.0 as f64 > popped.0 as f64 * 1.7,
            "x size {normal:?} vs {popped:?}"
        );
        assert!(
            normal.1 as f64 > popped.1 as f64 * 1.7,
            "y size {normal:?} vs {popped:?}"
        );
    }

    #[test]
    fn builtin_text_scales_glyph_spacing_with_font_size() {
        let mut config = RendererUiConfig::default();
        config.design_size = [30, 20];
        config.enabled = true;
        config.elements.push(UiElement {
            id: "two-letters".into(),
            kind: UiElementKind::Text {
                text: "AA".into(),
                value: None,
                color: [255, 255, 255, 255],
                font_size: 14.0,
                font: None,
                align: UiTextAlign::Left,
            },
            ..UiElement::default()
        });
        let mut frame = rgb(30, 20, [0, 0, 0]);
        render_gameplay_ui(&mut frame, 30, 20, &config, &UiFrameState::default()).unwrap();
        let pixel = |x: usize, y: usize| &frame[(y * 30 + x) * 3..(y * 30 + x) * 3 + 3];
        assert_eq!(pixel(2, 3), &[255, 255, 255]);
        assert_eq!(pixel(14, 3), &[255, 255, 255]);
        assert_eq!(pixel(11, 3), &[0, 0, 0]);
    }

    #[test]
    fn included_profiles_parse_and_render_distinct_layouts_at_multiple_aspect_ratios() {
        let mut profiles = [
            profile("default.json"),
            profile("minimal.json"),
            profile("terminal.json"),
        ];
        assert!(profiles[0].loaded_assets.contains_key("score-panel"));
        assert!(profiles[0].loaded_assets.contains_key("score-bar"));
        assert!(profiles[0].loaded_assets.contains_key("life-panel"));
        assert!(profiles[2].loaded_assets.contains_key("spectrum"));
        let state = UiFrameState {
            time: 3.25,
            delta_time: 1.0 / 60.0,
            progress: 0.42,
            runtime_entities: 180,
            active_entities: 12,
            callbacks: 8,
            draws: 64,
            scheduled_events: 5,
            ..UiFrameState::default()
        };
        for (width, height) in [(320, 180), (256, 256), (1024, 768)] {
            let mut outputs = Vec::new();
            for config in &mut profiles {
                let mut frame = rgb(width, height, [14, 18, 25]);
                render_gameplay_ui(&mut frame, width, height, config, &state).unwrap();
                assert_ne!(frame, rgb(width, height, [14, 18, 25]));
                outputs.push(frame);
            }
            assert_ne!(
                outputs[0], outputs[1],
                "default and minimal at {width}x{height}"
            );
            assert_ne!(
                outputs[1], outputs[2],
                "minimal and terminal at {width}x{height}"
            );
            assert_ne!(
                outputs[0], outputs[2],
                "default and terminal at {width}x{height}"
            );
        }
    }

    #[test]
    fn ui_modes_preserve_legacy_fallback_and_off_is_noop() {
        let (width, height) = (320, 180);
        let state = UiFrameState {
            time: 0.5,
            progress: 0.5,
            ..UiFrameState::default()
        };
        let original = rgb(width, height, [0, 0, 0]);

        let mut custom = RendererUiConfig::default();
        custom.enabled = true;
        custom.mode = GameplayUiMode::Custom;
        custom.elements.push(UiElement {
            id: "custom-block".into(),
            kind: UiElementKind::Rectangle {
                color: [230, 30, 70, 255],
                border: None,
                radius: 0.0,
            },
            layout: UiLayout {
                x: 0.2,
                y: 0.2,
                width: 0.25,
                height: 0.2,
                ..UiLayout::default()
            },
            ..UiElement::default()
        });
        let mut custom_frame = original.clone();
        render_gameplay_ui(&mut custom_frame, width, height, &custom, &state).unwrap();

        let mut legacy = RendererUiConfig::default();
        legacy.enabled = true;
        legacy.mode = GameplayUiMode::Legacy;
        legacy.primary_metric.provider = UiValueProvider::Progress;
        let mut legacy_frame = original.clone();
        render_gameplay_ui(&mut legacy_frame, width, height, &legacy, &state).unwrap();

        let mut off = custom.clone();
        off.mode = GameplayUiMode::Off;
        let mut off_frame = original.clone();
        render_gameplay_ui(&mut off_frame, width, height, &off, &state).unwrap();

        assert_ne!(custom_frame, original);
        assert_ne!(legacy_frame, original);
        assert_ne!(custom_frame, legacy_frame);
        assert_eq!(off_frame, original);
    }

    #[test]
    fn configured_scoring_matches_event_and_note_type_and_clamps_life() {
        let scoring = UiScoringConfig {
            source: UiScoringSource::EventTrack,
            initial_score: 0.0,
            initial_combo: 0,
            initial_life: 90.0,
            minimum_life: 0.0,
            maximum_life: 100.0,
            rules: vec![
                UiScoreRule {
                    event: "hit".into(),
                    note_type: None,
                    score: 10.0,
                    combo: UiComboChange::Increment,
                    life: 1.0,
                    judgement: "GOOD".into(),
                    variables: BTreeMap::new(),
                },
                UiScoreRule {
                    event: "hit".into(),
                    note_type: Some("special".into()),
                    score: 50.0,
                    combo: UiComboChange::Increment,
                    life: 15.0,
                    judgement: "BURST".into(),
                    variables: BTreeMap::from([("special_hits".into(), 1.0)]),
                },
                UiScoreRule {
                    event: "miss".into(),
                    note_type: None,
                    score: 0.0,
                    combo: UiComboChange::Reset,
                    life: -20.0,
                    judgement: "MISS".into(),
                    variables: BTreeMap::new(),
                },
            ],
            forecast: Default::default(),
        };
        let mut state = UiGameplayState::from_config(&scoring);
        assert!(state
            .apply(
                &scoring,
                &UiGameplayEvent {
                    time: 1.0,
                    event: "hit".into(),
                    note_type: Some("normal".into()),
                    ..UiGameplayEvent::default()
                }
            )
            .unwrap());
        assert_eq!((state.score, state.combo, state.life), (10.0, 1, 91.0));
        assert_eq!(state.judgment.as_deref(), Some("GOOD"));
        assert_eq!(state.judgment_time, Some(1.0));

        state
            .apply(
                &scoring,
                &UiGameplayEvent {
                    time: 2.0,
                    event: "hit".into(),
                    note_type: Some("special".into()),
                    ..UiGameplayEvent::default()
                },
            )
            .unwrap();
        assert_eq!((state.score, state.combo, state.life), (60.0, 2, 100.0));
        assert_eq!(state.judgment.as_deref(), Some("BURST"));
        assert_eq!(state.judgment_time, Some(2.0));
        assert_eq!(state.variables.get("special_hits"), Some(&1.0));

        state
            .apply(
                &scoring,
                &UiGameplayEvent {
                    time: 3.0,
                    event: "miss".into(),
                    note_type: Some("normal".into()),
                    ..UiGameplayEvent::default()
                },
            )
            .unwrap();
        assert_eq!((state.score, state.combo, state.life), (60.0, 0, 80.0));
        assert_eq!(state.event.as_deref(), Some("miss"));
        assert_eq!(state.judgment.as_deref(), Some("MISS"));
        assert_eq!(state.latest_event_time, Some(3.0));
        assert_eq!(state.judgment_time, Some(3.0));
    }

    #[test]
    fn event_log_is_an_explicit_adapter_and_custom_state_is_not_fabricated() {
        let mut config = profile("default.json");
        assert!(config.gameplay_state_at(10.0).unwrap().is_none());
        config.scoring = Some(UiScoringConfig {
            initial_life: 100.0,
            maximum_life: 100.0,
            rules: vec![
                UiScoreRule {
                    event: "hit".into(),
                    note_type: Some("normal".into()),
                    score: 100.0,
                    combo: UiComboChange::Increment,
                    judgement: "GOOD".into(),
                    ..UiScoreRule::default()
                },
                UiScoreRule {
                    event: "hit".into(),
                    note_type: Some("special".into()),
                    score: 250.0,
                    combo: UiComboChange::Increment,
                    judgement: "BURST".into(),
                    ..UiScoreRule::default()
                },
                UiScoreRule {
                    event: "miss".into(),
                    combo: UiComboChange::Reset,
                    judgement: "MISS".into(),
                    ..UiScoreRule::default()
                },
            ],
            ..UiScoringConfig::default()
        });
        config.events_file = Some(PathBuf::from("example-events.json"));
        config.load_external_data().unwrap();
        let after_normal = config.gameplay_state_at(1.25).unwrap().unwrap();
        assert_eq!((after_normal.score, after_normal.combo), (100.0, 1));
        let after_special = config.gameplay_state_at(1.75).unwrap().unwrap();
        assert_eq!((after_special.score, after_special.combo), (350.0, 2));
        assert_eq!(after_special.note_type.as_deref(), Some("special"));
        let after_miss = config.gameplay_state_at(2.5).unwrap().unwrap();
        assert_eq!((after_miss.score, after_miss.combo), (350.0, 0));
        assert_eq!(after_miss.judgment.as_deref(), Some("MISS"));
    }

    #[test]
    fn unsupported_chart_forecast_reports_status_without_blocking_render_setup() {
        let mut config = RendererUiConfig::default();
        config.scoring = Some(UiScoringConfig {
            source: UiScoringSource::SonoOverlay,
            ..UiScoringConfig::default()
        });
        let level = crate::formats::LevelData {
            bgm_offset: Some(0.25),
            entities: Vec::new(),
            metadata: None,
            extra: serde_json::Map::new(),
        };
        config.prepare_scoring_for_level(&level);
        assert!(config.gameplay_state_at(1.0).unwrap().is_none());
        assert!(config
            .scoring_status()
            .unwrap()
            .contains("Rendering will continue without simulated score state"));
    }

    #[test]
    fn missing_scoring_configuration_uses_auto_ap_and_manual_track_takes_precedence() {
        let level = crate::formats::LevelData {
            bgm_offset: Some(0.0),
            entities: vec![
                crate::formats::LevelEntity {
                    archetype: serde_json::Value::String("#BPM_CHANGE".into()),
                    data: serde_json::json!([
                        { "name": "#BEAT", "value": 0.0 },
                        { "name": "#BPM", "value": 120.0 }
                    ]),
                    extra: Default::default(),
                },
                crate::formats::LevelEntity {
                    archetype: serde_json::Value::String("NormalTapNote".into()),
                    data: serde_json::json!([{ "name": "#BEAT", "value": 1.0 }]),
                    extra: Default::default(),
                },
            ],
            metadata: None,
            extra: serde_json::Map::new(),
        };

        let mut automatic = RendererUiConfig::default();
        assert_eq!(
            automatic.resolved_scoring_source(),
            UiScoringSource::SonoOverlay
        );
        automatic.prepare_scoring_for_level(&level);
        assert!(automatic.loaded_forecast.is_some());
        assert!(automatic
            .scoring_status()
            .unwrap()
            .starts_with("Gameplay values: Auto-generated All Perfect"));
        assert_eq!(automatic.gameplay_state_at(0.0).unwrap().unwrap().combo, 0);
        let first_note = automatic.gameplay_state_at(0.51).unwrap().unwrap();
        assert_eq!(first_note.combo, 1);
        assert!(first_note.variables["score_bonus"] > 0.0);
        assert_eq!(
            first_note.variables["score_bonus"],
            first_note.variables["score_delta"]
        );

        let mut manual = RendererUiConfig::default();
        manual.scoring = Some(UiScoringConfig::default());
        manual.events_file = Some(PathBuf::from("manual-events.json"));
        manual.prepare_scoring_for_level(&level);
        assert_eq!(
            manual.resolved_scoring_source(),
            UiScoringSource::EventTrack
        );
        assert!(manual.loaded_forecast.is_none());
        assert!(manual
            .scoring_status()
            .unwrap()
            .starts_with("Gameplay values: Manually loaded event track"));

        let mut disabled = RendererUiConfig::default();
        disabled.scoring = Some(UiScoringConfig {
            source: UiScoringSource::Disabled,
            ..UiScoringConfig::default()
        });
        disabled.prepare_scoring_for_level(&level);
        assert!(disabled.loaded_forecast.is_none());
        assert!(disabled.gameplay_state_at(0.0).unwrap().is_none());
    }

    #[test]
    fn watch_state_variables_conditioned_styles_and_animations_change_pixels() {
        let (width, height) = (320, 180);
        let mut memory = crate::runtime::Memory::new();
        memory.set(2000, 3, 0.75);
        let mut config = RendererUiConfig::default();
        config.enabled = true;
        config.variables.insert(
            "charge".into(),
            UiExpression::Source {
                source: UiStateSource::Memory {
                    block_id: 2000,
                    index: 3,
                },
            },
        );
        config.elements.push(UiElement {
            id: "charge-gauge".into(),
            layout: UiLayout {
                x: 0.1,
                y: 0.4,
                width: 0.8,
                height: 0.15,
                ..UiLayout::default()
            },
            kind: UiElementKind::Gauge {
                value: UiExpression::Variable {
                    name: "charge".into(),
                },
                maximum: UiExpression::Constant { value: 1.0 },
                orientation: UiGaugeOrientation::LeftToRight,
                fill: [10, 180, 80, 255],
                track: [30, 30, 30, 255],
                fill_asset: None,
                fill_source: None,
                mask_asset: None,
            },
            style_rules: vec![UiStyleRule {
                when: UiCondition::StringEquals {
                    source: UiStringSource::NoteType,
                    value: "special".into(),
                },
                color: Some([250, 180, 10, 255]),
                border: None,
                tint: None,
                asset: None,
                opacity: Some(0.8),
            }],
            animations: vec![UiAnimation {
                property: UiAnimationProperty::OffsetY,
                from: 0.0,
                to: 0.1,
                start: 0.0,
                start_from: None,
                duration: 1.0,
                repeat: false,
                easing: UiEasing::Linear,
                when: None,
                value_from_state: None,
            }],
            ..UiElement::default()
        });
        let state_at_start = UiFrameState {
            memory: Some(memory),
            gameplay: Some(UiGameplayState {
                event: Some("hit".into()),
                note_type: Some("normal".into()),
                ..UiGameplayState::default()
            }),
            time: 0.0,
            ..UiFrameState::default()
        };
        let mut first = rgb(width, height, [0, 0, 0]);
        render_gameplay_ui(&mut first, width, height, &config, &state_at_start).unwrap();
        let mut other_style = state_at_start.clone();
        other_style.time = 1.0;
        other_style.gameplay.as_mut().unwrap().note_type = Some("special".into());
        let mut second = rgb(width, height, [0, 0, 0]);
        render_gameplay_ui(&mut second, width, height, &config, &other_style).unwrap();
        assert_ne!(first, second);
        assert!(first.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn event_timestamp_anchors_animation_and_supports_transient_visibility() {
        let config = RendererUiConfig::default();
        let element = UiElement {
            animations: vec![UiAnimation {
                property: UiAnimationProperty::Opacity,
                from: 0.0,
                to: 1.0,
                start: 0.0,
                start_from: Some(UiStateSource::JudgmentTime),
                duration: 1.0,
                repeat: false,
                easing: UiEasing::Linear,
                when: None,
                value_from_state: None,
            }],
            ..UiElement::default()
        };
        let mut state = UiFrameState {
            time: 10.5,
            gameplay: Some(UiGameplayState {
                judgment: Some("PERFECT".into()),
                latest_event_time: Some(10.0),
                judgment_time: Some(10.0),
                ..UiGameplayState::default()
            }),
            ..UiFrameState::default()
        };
        let (opacity, ..) = animation_values(&element, &config, &state);
        assert!((opacity - 0.5).abs() < 1e-9);

        let age = UiExpression::Subtract {
            left: Box::new(UiExpression::Source {
                source: UiStateSource::Time,
            }),
            right: Box::new(UiExpression::Source {
                source: UiStateSource::JudgmentTime,
            }),
        };
        let expires_after = UiCondition::Less {
            left: age,
            right: UiExpression::Constant { value: 0.667 },
        };
        assert!(evaluate_condition(&expires_after, &config, &state));
        state.time = 10.8;
        assert!(!evaluate_condition(&expires_after, &config, &state));
    }

    #[test]
    fn custom_bitmap_font_atlas_is_loaded_and_drawn() {
        let mut config = profile("terminal.json");
        config.assets.insert(
            "zero-glyph".into(),
            PathBuf::from("../assets/life/digit/0.png"),
        );
        config.load_external_data().unwrap();
        let atlas = &config.loaded_assets["zero-glyph"];
        config.fonts.insert(
            "zero-only".into(),
            UiBitmapFont {
                asset: "zero-glyph".into(),
                glyph_width: atlas.width,
                glyph_height: atlas.height,
                columns: 1,
                first_codepoint: '0' as u32,
                advance: 1.0,
                glyph_map: BTreeMap::new(),
                glyph_assets: BTreeMap::new(),
                glyph_scales: BTreeMap::new(),
                glyph_advances: BTreeMap::new(),
                center_spacing: None,
                glyph_max_dimension: None,
            },
        );
        config.elements = vec![UiElement {
            id: "font-sample".into(),
            layout: UiLayout {
                x: 0.1,
                y: 0.1,
                width: 0.8,
                height: 0.3,
                ..UiLayout::default()
            },
            kind: UiElementKind::Text {
                text: "0000".into(),
                value: None,
                color: [255, 255, 255, 255],
                font_size: 24.0,
                font: Some("zero-only".into()),
                align: UiTextAlign::Left,
            },
            ..UiElement::default()
        }];
        let mut frame = rgb(320, 180, [0, 0, 0]);
        render_gameplay_ui(&mut frame, 320, 180, &config, &UiFrameState::default()).unwrap();
        assert!(frame.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn custom_bitmap_font_renders_at_its_profile_coordinates() {
        let mut config = profile("minimal.json");
        config.assets.insert(
            "validation-zero".into(),
            PathBuf::from("../assets/life/digit/0.png"),
        );
        config.load_external_data().unwrap();
        let (glyph_width, glyph_height) =
            config.loaded_asset_dimensions("validation-zero").unwrap();
        config.fonts.insert(
            "validation-zero".into(),
            UiBitmapFont {
                asset: "validation-zero".into(),
                glyph_width,
                glyph_height,
                columns: 1,
                first_codepoint: '0' as u32,
                advance: 1.0,
                glyph_map: BTreeMap::new(),
                glyph_assets: BTreeMap::new(),
                glyph_scales: BTreeMap::new(),
                glyph_advances: BTreeMap::new(),
                center_spacing: None,
                glyph_max_dimension: None,
            },
        );
        config.load_external_data().unwrap();
        config.elements.push(UiElement {
            id: "bitmap-font-validation".into(),
            layout: UiLayout {
                x: 0.78,
                y: 0.07,
                width: 0.2,
                height: 0.13,
                ..UiLayout::default()
            },
            kind: UiElementKind::Text {
                text: "0000".into(),
                value: None,
                color: [255; 4],
                font_size: 52.0,
                font: Some("validation-zero".into()),
                align: UiTextAlign::Center,
            },
            ..UiElement::default()
        });
        let mut frame = rgb(1280, 720, [0, 0, 0]);
        render_gameplay_ui(&mut frame, 1280, 720, &config, &UiFrameState::default()).unwrap();
        let top_right_has_ink = (0..150).any(|y| {
            (950..1280).any(|x| {
                let offset = (y * 1280 + x) * 3;
                frame[offset..offset + 3] != [0, 0, 0]
            })
        });
        assert!(
            top_right_has_ink,
            "bitmap glyphs are missing at the element coordinates"
        );
    }

    #[test]
    fn asset_pack_import_registers_explicit_glyphs_and_reports_unresolved_assets() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("glyphs")).unwrap();
        image::RgbaImage::from_pixel(8, 12, image::Rgba([240, 30, 50, 255]))
            .save(directory.path().join("glyphs/zero.png"))
            .unwrap();
        image::RgbaImage::from_pixel(16, 8, image::Rgba([20, 220, 80, 255]))
            .save(directory.path().join("perfect.png"))
            .unwrap();
        let manifest = directory.path().join("presentation-pack.json");
        let pack = crate::presentation_assets::PresentationAssetPack {
            id: "fixture".into(),
            name: "Fixture pack".into(),
            assets: vec![
                crate::presentation_assets::PresentationAssetEntry {
                    id: "digit-zero".into(),
                    source: "glyphs/zero.png".into(),
                    kind: crate::presentation_assets::PresentationAssetKind::Glyph,
                    family: Some("score-digits".into()),
                    character: Some("0".into()),
                    ..Default::default()
                },
                crate::presentation_assets::PresentationAssetEntry {
                    id: "judgment-perfect".into(),
                    source: "perfect.png".into(),
                    kind: crate::presentation_assets::PresentationAssetKind::Sprite,
                    family: Some("judgement-art".into()),
                    tags: vec!["visible-label:PERFECT".into()],
                    ..Default::default()
                },
                crate::presentation_assets::PresentationAssetEntry {
                    id: "missing-digit".into(),
                    source: "glyphs/missing.png".into(),
                    kind: crate::presentation_assets::PresentationAssetKind::Glyph,
                    family: Some("combo-style-unknown".into()),
                    ..Default::default()
                },
                crate::presentation_assets::PresentationAssetEntry {
                    id: "unsupported-video".into(),
                    source: "unused.mp4".into(),
                    kind: crate::presentation_assets::PresentationAssetKind::Video,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        std::fs::write(&manifest, serde_json::to_vec_pretty(&pack).unwrap()).unwrap();

        let (config, report) = RendererUiConfig::import_asset_pack(&manifest, None).unwrap();
        assert_eq!(report.total_assets, 4);
        assert_eq!(report.loaded_assets.len(), 2);
        assert_eq!(report.missing_assets.len(), 1);
        assert_eq!(report.unsupported_assets.len(), 1);
        assert_eq!(config.fonts["score-digits"].glyph_assets["0"], "digit-zero");
        assert!(config
            .elements
            .iter()
            .any(|element| element.id == "judgment-perfect"));
        assert!(!config.fonts.contains_key("combo-style-unknown"));
        assert!(report
            .unconfigured_element_families
            .contains(&"combo-style-unknown".into()));
        assert_eq!(
            config.loaded_asset_path("digit-zero"),
            Some(
                directory
                    .path()
                    .join("glyphs/zero.png")
                    .canonicalize()
                    .unwrap()
                    .as_path()
            )
        );

        let unrelated = directory.path().join("unrelated.json");
        std::fs::write(&unrelated, br#"{"schema_version":1,"elements":[]}"#).unwrap();
        assert!(RendererUiConfig::import_asset_pack(&unrelated, None).is_err());
    }

    #[test]
    fn missing_pack_assets_keep_profile_editable_and_can_be_relinked_to_a_new_root() {
        let directory = tempfile::tempdir().unwrap();
        let original_root = directory.path().join("original assets");
        let relocated_root = directory.path().join("relocated assets");
        std::fs::create_dir_all(original_root.join("nested images")).unwrap();
        std::fs::create_dir_all(relocated_root.join("nested images")).unwrap();
        let original_image = original_root.join("nested images/frame one.png");
        image::RgbaImage::from_pixel(12, 8, image::Rgba([40, 80, 220, 255]))
            .save(&original_image)
            .unwrap();
        let manifest = original_root.join("presentation-pack.json");
        let pack = crate::presentation_assets::PresentationAssetPack {
            id: "repair-fixture".into(),
            name: "Repair fixture".into(),
            assets: vec![crate::presentation_assets::PresentationAssetEntry {
                id: "frame-one".into(),
                source: "nested images/frame one.png".into(),
                kind: crate::presentation_assets::PresentationAssetKind::Sprite,
                ..Default::default()
            }],
            ..Default::default()
        };
        std::fs::write(&manifest, serde_json::to_vec_pretty(&pack).unwrap()).unwrap();

        let (mut config, _) = RendererUiConfig::import_asset_pack(&manifest, None).unwrap();
        config.elements.push(UiElement {
            id: "saved-frame".into(),
            kind: UiElementKind::Image {
                asset: "frame-one".into(),
                tint: None,
            },
            ..Default::default()
        });
        let profile = original_root.join("hud-profile.json");
        config.save_profile(&profile).unwrap();

        let relocated_image = relocated_root.join("nested images/frame one.png");
        image::RgbaImage::from_pixel(12, 8, image::Rgba([220, 80, 40, 255]))
            .save(&relocated_image)
            .unwrap();
        std::fs::remove_file(original_image).unwrap();

        assert!(RendererUiConfig::load_profile(&profile).is_err());
        let mut repairable = RendererUiConfig::load_profile_for_repair(&profile).unwrap();
        assert_eq!(
            repairable
                .elements
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>(),
            config
                .elements
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>()
        );
        assert!(repairable.elements.iter().any(|element| {
            matches!(&element.kind, UiElementKind::Image { asset, .. } if asset == "frame-one")
        }));
        repairable.asset_root = Some(relocated_root.clone());
        repairable.load_external_data().unwrap();
        assert_eq!(
            repairable.loaded_asset_path("frame-one"),
            Some(relocated_image.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            repairable
                .elements
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>(),
            config
                .elements
                .iter()
                .map(|element| element.id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn supplied_pack_import_renders_its_png_and_survives_profile_and_project_reopen() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest = repository.join("assets/presentation-pack.json");
        let (mut imported, report) = RendererUiConfig::import_asset_pack(&manifest, None).unwrap();
        assert_eq!(report.pack_id, "sono-reference");
        assert!(report.loaded_assets.iter().any(|id| id == "judge.1"));
        assert_eq!(imported.fonts["score"].glyph_assets["0"], "score-digit-0");
        assert_eq!(imported.fonts["life"].glyph_assets["0"], "life-digit-0");
        let mut builtin = RendererUiConfig::reference_default();
        builtin.load_external_data().unwrap();
        assert_eq!(
            imported.elements, builtin.elements,
            "importing the supplied Sono-Overlay pack and using the built-in default must resolve the same HUD geometry, layers, visibility rules, and animation definitions"
        );
        assert_eq!(imported.fonts["combo"].center_spacing, Some(108.0));
        assert_eq!(builtin.fonts["combo"].center_spacing, Some(108.0));
        assert_eq!(imported.fonts["combo-burst"].center_spacing, Some(108.0));
        assert_eq!(builtin.fonts["combo-burst"].center_spacing, Some(108.0));
        assert_eq!(imported.scoring, builtin.scoring);
        for (font_name, pack_family, asset_prefix) in [
            ("combo", "combo.glyph-family.p", "combo.p"),
            ("combo-burst", "combo.glyph-family.b", "combo.b"),
        ] {
            for digit in '0'..='9' {
                let character = digit.to_string();
                let alias = format!(
                    "{}-{digit}",
                    if font_name == "combo" {
                        "combo-digit"
                    } else {
                        "combo-burst"
                    }
                );
                let pack_id = format!("{asset_prefix}{digit}");
                assert_eq!(imported.fonts[font_name].glyph_assets[&character], alias);
                assert_eq!(builtin.fonts[font_name].glyph_assets[&character], alias);
                assert!(report.glyph_families[pack_family]
                    .mapped_characters
                    .contains(&character));
                assert_eq!(
                    imported.assets[&alias],
                    PathBuf::from(format!("pack://{pack_id}")),
                    "imported {font_name} digit {digit} must resolve to its source family"
                );
                assert_eq!(
                    imported.loaded_assets[&alias].rgba,
                    builtin.loaded_assets[&alias].rgba,
                    "the imported and built-in profiles must rasterize the same {font_name} digit {digit}"
                );
            }
        }
        let normal_combo = &imported.fonts["combo.glyph-family.n"];
        assert_eq!(normal_combo.center_spacing, Some(108.0));
        for digit in '0'..='9' {
            let character = digit.to_string();
            let id = format!("combo.n{digit}");
            assert_eq!(normal_combo.glyph_assets[&character], id);
            assert_eq!(imported.assets[&id], PathBuf::from(format!("pack://{id}")));
        }
        assert_eq!(
            report.glyph_families["combo.glyph-family.n"]
                .mapped_characters
                .len(),
            10
        );
        assert!(!report
            .unconfigured_element_families
            .contains(&"combo.glyph-family.n".into()));
        assert_eq!(report.unresolved_glyph_assets.len(), 56);
        assert!(report
            .unresolved_glyph_assets
            .iter()
            .any(|asset| asset == "combo.n_%"));
        assert!(imported.elements.iter().any(|element| {
            element.id == "combo-value"
                && matches!(
                    &element.kind,
                    UiElementKind::Counter {
                        font: Some(font),
                        ..
                    } if font == "combo"
                )
        }));
        assert!(!imported.elements.iter().any(|element| {
            matches!(
                &element.kind,
                UiElementKind::Counter {
                    font: Some(font),
                    ..
                } if font == "combo.glyph-family.n"
            )
        }));
        for (family, font_name) in [
            ("score.glyphs", "score"),
            ("life.glyphs", "life"),
            ("combo.glyph-family.p", "combo"),
            ("combo.glyph-family.n", "combo.glyph-family.n"),
        ] {
            let mapped = &report.glyph_families[family].mapped_characters;
            for digit in '0'..='9' {
                assert!(
                    mapped.contains(&digit.to_string()),
                    "{family} must map reference digit {digit}"
                );
                assert!(imported.fonts[font_name]
                    .glyph_assets
                    .contains_key(&digit.to_string()));
            }
        }
        assert!(!report
            .unconfigured_element_families
            .contains(&"combo.glyph-family.p".into()));
        // The reference-based profile now includes the static combo shadow
        // plus both persistent and event-pulse combo glyph layers.
        assert_eq!(report.configured_elements, 28);
        let combo_shadow = &imported.fonts["combo-burst"];
        assert_eq!(combo_shadow.center_spacing, Some(108.0));
        assert!(combo_shadow.glyph_advances.is_empty());
        assert!(imported
            .elements
            .iter()
            .any(|element| element.id == "score-panel"));
        assert!(imported
            .elements
            .iter()
            .any(|element| element.id == "score-value"));
        assert!(imported
            .elements
            .iter()
            .any(|element| element.id == "life-panel"));

        let expected_png = repository
            .join("assets/judge/1.png")
            .canonicalize()
            .unwrap();
        assert_eq!(
            imported.loaded_asset_path("judge.1"),
            Some(expected_png.as_path())
        );
        imported.enabled = true;
        let width = 640u32;
        let height = 360u32;
        let background = [20, 25, 36];
        let mut frame = rgb(width, height, background);
        render_gameplay_ui(
            &mut frame,
            width,
            height,
            &imported,
            &UiFrameState {
                gameplay: Some(UiGameplayState {
                    score: 123456.0,
                    combo: 789,
                    life: 880.0,
                    judgment: Some("PERFECT".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let visible_pixels = frame
            .chunks_exact(3)
            .filter(|pixel| *pixel != background)
            .count();
        assert!(
            visible_pixels > 50,
            "imported judgement and glyph PNGs should visibly render, got {visible_pixels} changed pixels"
        );
        assert_eq!(
            imported.loaded_asset_path("score.digit.0"),
            Some(
                repository
                    .join("assets/score/digit/0.png")
                    .canonicalize()
                    .unwrap()
                    .as_path()
            )
        );
        assert_eq!(
            imported.loaded_asset_path("life.digit.8"),
            Some(
                repository
                    .join("assets/life/digit/8.png")
                    .canonicalize()
                    .unwrap()
                    .as_path()
            )
        );

        let scoring = imported.scoring.as_mut().unwrap();
        scoring.source = UiScoringSource::SonoOverlay;
        scoring.forecast.team_power = 321_000.0;
        scoring.forecast.level_rating = Some(14.0);
        scoring.forecast.score_mode = crate::sono_overlay_scoring::ScoreMode::Tournament;
        scoring.forecast.all_flick = true;
        scoring.forecast.chart_skills_enabled = false;

        let directory = tempfile::tempdir().unwrap();
        let profile_path = directory.path().join("imported-ui.json");
        imported.save_profile(&profile_path).unwrap();
        let reloaded = RendererUiConfig::load_profile(&profile_path).unwrap();
        assert_eq!(reloaded.scoring, imported.scoring);
        assert_eq!(reloaded.fonts["score"].glyph_assets["0"], "score-digit-0");
        assert_eq!(
            reloaded.fonts["combo.glyph-family.n"].glyph_assets["9"],
            "combo.n9"
        );
        assert_eq!(reloaded.fonts["combo"].glyph_assets["8"], "combo-digit-8");
        assert_eq!(reloaded.elements, imported.elements);
        assert_eq!(reloaded.asset_root, imported.asset_root);

        let mut render_config = crate::render::RenderConfig::default();
        render_config.ui = reloaded;
        render_config.ui.enabled = true;
        let profiles_path = directory.path().join("profiles.json");
        let profiles =
            crate::config_profiles::ConfigProfileManager::open_at(&profiles_path).unwrap();
        let saved_profile = profiles.create("Imported HUD", render_config).unwrap();
        let mut project = crate::project::RenderProject::from_config(
            &saved_profile.config,
            Some(crate::project::ProjectProfileRef {
                id: Some(saved_profile.id.clone()),
                name: Some(saved_profile.name.clone()),
                ..Default::default()
            }),
        );
        let project_path = directory.path().join("gameplay.sonoproject");
        project.save(&project_path).unwrap();
        let reopened = crate::project::RenderProject::load(&project_path).unwrap();
        let (mut resolved, profile) = reopened
            .resolve_config(
                &project_path,
                &crate::render::RenderConfig::default(),
                Some(&profiles),
                None,
            )
            .unwrap();
        assert_eq!(profile.unwrap().id, saved_profile.id);
        assert!(resolved.ui.enabled);
        assert_eq!(resolved.ui.asset_root, imported.asset_root);
        assert_eq!(
            resolved.ui.fonts["score"].glyph_assets["0"],
            "score-digit-0"
        );
        assert_eq!(resolved.ui.scoring, imported.scoring);
        assert_eq!(resolved.ui.elements, imported.elements);
        resolved.ui.load_external_data().unwrap();
        assert_eq!(
            resolved.ui.loaded_asset_path("judge.1"),
            Some(expected_png.as_path())
        );
    }
}
