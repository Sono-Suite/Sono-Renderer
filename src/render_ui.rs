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
    pub layout: UiLayout,
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
            layout: UiLayout::default(),
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
            enabled: false,
            profile_name: seed.profile_name.clone(),
            intentionally_empty: false,
            design_size: seed.design_size,
            fit: seed.fit,
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
        if config.has_complete_reference_asset_set() {
            config.use_reference_hud(true);
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
        let glyph_asset_ids: BTreeSet<_> = self
            .fonts
            .values()
            .flat_map(|font| font.glyph_assets.values().cloned())
            .collect();
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
            let max_dimension = if glyph_asset_ids.contains(id) {
                128
            } else {
                1024
            };
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
            .unwrap_or(UiScoringSource::Disabled)
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
        let Some(scoring) = self.scoring.as_ref() else {
            if self.events_file.is_some() {
                self.scoring_status =
                    Some("Event track selected, but no event-to-HUD rules are configured.".into());
            }
            return;
        };
        match self.resolved_scoring_source() {
            UiScoringSource::Disabled => {
                self.scoring_status = Some("Gameplay scoring is disabled.".into());
            }
            UiScoringSource::EventTrack => {
                self.scoring_status = Some(if self.events_file.is_some() {
                    format!(
                        "Event track selected ({} events loaded).",
                        self.loaded_events.len()
                    )
                } else {
                    "Event track selected, but no event file is configured.".into()
                });
            }
            UiScoringSource::SonoOverlay => {
                match crate::sono_overlay_scoring::simulate(level, &scoring.forecast) {
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
                                "Scoring: best effort — {incomplete_count} unrecognized archetype{} skipped. Effective level rating {:.1} ({rating_source}); recognized notes remain active, but the forecast may be incomplete.",
                                if incomplete_count == 1 { "" } else { "s" },
                                forecast.level_rating,
                            )
                        } else {
                            format!(
                                "Sono-Overlay all-perfect forecast: {} recognized notes; {:.3} total weight. Effective level rating {:.1} ({rating_source}). This is not recorded player performance.",
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
    let (scale_x, scale_y) = match config.fit {
        UiFitMode::Stretch => (output_w / design_w, output_h / design_h),
        UiFitMode::Contain | UiFitMode::Cover => {
            let scale = if config.fit == UiFitMode::Contain {
                (output_w / design_w).min(output_h / design_h)
            } else {
                (output_w / design_w).max(output_h / design_h)
            };
            (scale, scale)
        }
    };
    let root = UiMatrix {
        a: scale_x,
        b: 0.0,
        c: 0.0,
        d: scale_y,
        tx: (output_w - design_w * scale_x) * 0.5,
        ty: (output_h - design_h * scale_y) * 0.5,
        opacity: 1.0,
    };
    let screen = UiRect {
        x: 0.0,
        y: 0.0,
        width: design_w,
        height: design_h,
    };
    let mut ordered: Vec<_> = config.elements.iter().enumerate().collect();
    ordered.sort_by_key(|(_, element)| element.z_index);
    for (_, element) in ordered {
        render_element(rgb, width, height, config, state, element, screen, root, 0)?;
    }
    Ok(())
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

fn render_element(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &RendererUiConfig,
    state: &UiFrameState,
    element: &UiElement,
    parent: UiRect,
    parent_matrix: UiMatrix,
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
    let mut bounds = layout_rect(&element.layout, parent);
    let (opacity, offset_x, offset_y, scale_x, scale_y, rotation) =
        animation_values(element, config, state);
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
    bounds.x += offset_x;
    bounds.y += offset_y;
    bounds.width *= scale_x;
    bounds.height *= scale_y;
    let anchor = anchor_fraction(element.layout.anchor);
    let origin_x = bounds.x - bounds.width * anchor[0];
    let origin_y = bounds.y - bounds.height * anchor[1];
    let center_x = bounds.width * 0.5;
    let center_y = bounds.height * 0.5;
    let angle = (element.layout.rotation + rotation).to_radians();
    let cos = angle.cos();
    let sin = angle.sin();
    let local = UiMatrix {
        a: cos * element.layout.scale[0],
        b: sin * element.layout.scale[0],
        c: -sin * element.layout.scale[1],
        d: cos * element.layout.scale[1],
        tx: origin_x + center_x - cos * element.layout.scale[0] * center_x
            + sin * element.layout.scale[1] * center_y,
        ty: origin_y + center_y
            - sin * element.layout.scale[0] * center_x
            - cos * element.layout.scale[1] * center_y,
        opacity: (element.opacity * opacity * style_opacity).clamp(0.0, 1.0),
    };
    let world = parent_matrix.compose(local);
    let rect = UiRect {
        x: 0.0,
        y: 0.0,
        width: bounds.width,
        height: bounds.height,
    };
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
            );
        }
        UiElementKind::Counter {
            value,
            prefix,
            suffix,
            digits,
            color,
            font_size,
            font,
            align,
        } => {
            let text = evaluate_expression(value, config, state)
                .map(|value| format!("{prefix}{}{suffix}", format_counter(value, *digits)))
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
    let judgment_progress = state.judgment_time.map(frame_delta);
    let judgment_opacity = judgment_progress.map_or(0.0, |progress| {
        if progress < 2.0 {
            0.0
        } else if progress < 5.0 {
            (2.0 / 3.0 - (-1.45 + progress / 4.0).powi(4) * (2.0 / 3.0)).clamp(0.0, 1.0)
        } else if progress < 20.0 {
            2.0 / 3.0
        } else {
            0.0
        }
    });
    state
        .variables
        .insert("judgment_opacity".into(), judgment_opacity);
    state.variables.insert(
        "judgment_visible".into(),
        if judgment_progress.is_some_and(|progress| progress < 20.0) {
            1.0
        } else {
            0.0
        },
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

fn format_counter(value: f64, digits: usize) -> String {
    let value = format_value(value);
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
) {
    draw_transformed_rect(rgb, width, height, matrix, rect, track, 0.0);
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
            rgb, width, height, matrix, filled, bitmap, source, None, mask,
        );
    } else {
        draw_transformed_rect(rgb, width, height, matrix, filled, fill, 0.0);
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
) {
    let custom_font = font_name.and_then(|name| fonts.get(name));
    let scale = font_size / 7.0;
    let glyph_height = font_size;
    let mut chars_drawn = 0;
    let lines: Vec<_> = text.split('\n').take(16).collect();
    let block_height = glyph_height * lines.len() as f64;
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
        let text_width = metrics.iter().map(|(_, advance)| advance).sum::<f64>();
        let mut x = match align {
            UiTextAlign::Left => 0.0,
            UiTextAlign::Center => (rect.width - text_width) * 0.5,
            UiTextAlign::Right => rect.width - text_width,
        };
        let y = (rect.height - block_height) * 0.5 + line_index as f64 * glyph_height;
        for (ch, (glyph_width, advance)) in chars.into_iter().zip(metrics) {
            if let Some(font) = custom_font {
                if let Some((bitmap, source)) = bitmap_glyph(font, ch, loaded_assets) {
                    draw_transformed_image(
                        rgb,
                        width,
                        height,
                        matrix,
                        UiRect {
                            x,
                            y,
                            width: glyph_width,
                            height: glyph_height,
                        },
                        bitmap,
                        source,
                        Some(color),
                    );
                } else {
                    draw_builtin_glyph(
                        rgb,
                        width,
                        height,
                        matrix,
                        x,
                        y,
                        scale,
                        builtin_case(ch, preserve_builtin_case),
                        color,
                    );
                }
            } else {
                draw_builtin_glyph(
                    rgb,
                    width,
                    height,
                    matrix,
                    x,
                    y,
                    scale,
                    builtin_case(ch, preserve_builtin_case),
                    color,
                );
            }
            x += advance;
        }
    }
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
) -> (f64, f64) {
    if let Some(font) = font {
        if let Some((_, source)) = bitmap_glyph(font, ch, assets) {
            let glyph_width = font_size * f64::from(source[2]) / f64::from(source[3].max(1));
            return (glyph_width, glyph_width * font.advance);
        }
    }
    let glyph_width = font_size * 5.0 / 7.0;
    (glyph_width, font_size * 6.0 / 7.0)
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
                .map(|ch| text_glyph_metrics(ch, font_size, font, assets).1)
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

fn draw_transformed_rect(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    color: [u8; 4],
    radius: f64,
) {
    draw_transformed_shape(rgb, width, height, matrix, rect, radius, |_, _| true, color);
}

fn draw_transformed_border(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    color: [u8; 4],
    radius: f64,
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
    );
}

fn draw_transformed_shape(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    radius: f64,
    predicate: impl Fn(f64, f64) -> bool,
    color: [u8; 4],
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
            blend_pixel(rgb, width, height, x, y, color, matrix.opacity);
        }
    }
}

fn draw_transformed_image(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    matrix: UiMatrix,
    rect: UiRect,
    bitmap: &UiBitmap,
    source: [u32; 4],
    tint: Option<[u8; 4]>,
) {
    draw_transformed_image_masked(rgb, width, height, matrix, rect, bitmap, source, tint, None);
}

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
            blend_pixel(rgb, width, height, x, y, color, matrix.opacity);
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

fn blend_pixel(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    color: [u8; 4],
    opacity: f64,
) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let alpha = (f64::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u16;
    if alpha == 0 {
        return;
    }
    let offset = (y as usize * width as usize + x as usize) * 3;
    for channel in 0..3 {
        rgb[offset + channel] = ((u16::from(rgb[offset + channel]) * (255 - alpha)
            + u16::from(color[channel]) * alpha)
            / 255) as u8;
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
    fn bundled_sono_overlay_hud_renders_at_landscape_and_portrait_sizes() {
        let mut config = RendererUiConfig::reference_default();
        assert_eq!(config.profile_name, "Asset-backed Gameplay HUD");
        assert!(!config.enabled);
        assert_eq!(config.elements.len(), 26);
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
        assert_eq!(combo_value.layout.x, 0.85);
        assert_eq!(combo_value.layout.y, 0.445);
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
        assert!(matches!(
            &combo_pulse.kind,
            UiElementKind::Counter { font: Some(font), .. } if font == "combo-burst"
        ));
        let judgment = config
            .elements
            .iter()
            .find(|element| element.id == "judgement-perfect")
            .expect("reference judgment artwork");
        assert_eq!(judgment.layout.x, 0.5);
        assert_eq!(judgment.layout.y, 0.62);
        assert!(judgment.visible_when.is_some());
        assert!(judgment.animations.iter().any(|animation| matches!(
            animation.value_from_state.as_ref(),
            Some(UiExpression::Source {
                source: UiStateSource::GameplayVariable { name }
            }) if name == "judgment_opacity"
        )));
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

        for (width, height) in [(1920, 1080), (1280, 720), (640, 360), (1080, 1920)] {
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
    fn overlay_combo_and_judgment_curves_are_timestamp_based_and_seekable() {
        let mut before = UiGameplayState::default();
        apply_note_hud_animations(&mut before, 2.0, 30.0);
        assert_eq!(before.variables["combo_pop_scale"], 1.0);
        assert_eq!(before.variables["combo_pulse_opacity"], 0.0);
        assert_eq!(before.variables["judgment_opacity"], 0.0);

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

        let mut during = at_event.clone();
        apply_note_hud_animations(&mut during, 2.0 + 8.0 / 30.0, 30.0);
        assert!((during.variables["combo_pop_scale"] - 1.0).abs() < 1e-12);
        assert!((during.variables["combo_pulse_scale"] - 1.0).abs() < 1e-12);
        assert!((during.variables["combo_pulse_opacity"] - 6.0 / 14.0).abs() < 1e-12);
        assert!((during.variables["judgment_opacity"] - 2.0 / 3.0).abs() < 1e-12);

        let mut judgment_in = at_event.clone();
        apply_note_hud_animations(&mut judgment_in, 2.0 + 2.0 / 30.0, 30.0);
        let lua_curve_at_two = 2.0 / 3.0 - (-1.45 + 2.0 / 4.0_f64).powi(4) * (2.0 / 3.0);
        assert!((judgment_in.variables["judgment_opacity"] - lua_curve_at_two).abs() < 1e-12);

        let mut after = at_event.clone();
        apply_note_hud_animations(&mut after, 2.0 + 20.0 / 30.0, 30.0);
        assert_eq!(after.variables["judgment_opacity"], 0.0);
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
        for (family, font_name) in [
            ("score.glyphs", "score"),
            ("life.glyphs", "life"),
            ("combo.glyph-family.p", "combo"),
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
        assert_eq!(report.configured_elements, 26);
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
